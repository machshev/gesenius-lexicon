//! Minimal ALTO 4 reader/writer and layout-aware structural parser.

use crate::language::{identify_languages, profile_for_label};
use crate::metrics::{normalized_disagreement, polygon_iou};
use crate::model::{
    stable_entry_id, BlockKind, CorpusEntry, Direction, EntryBlock, EntryProvenance, OcrHypothesis,
    Point, ReviewState, SourceCoordinate, TextSpan,
};
use crate::unicode::{
    aggregate_confidence, classify_script, infer_direction, normalize_nfc, unicode_warnings,
    without_bidi_controls,
};
use anyhow::{bail, Context, Result};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use unicode_script::UnicodeScript;

/// Engine metadata attached to an imported ALTO hypothesis.
#[derive(Debug, Clone)]
pub struct EngineIdentity {
    /// Executable name.
    pub engine: String,
    /// Reported executable version.
    pub version: String,
    /// Model identifier.
    pub model: String,
    /// Model content digest or immutable package identity.
    pub model_hash: String,
}

/// In-memory subset of ALTO needed by the corpus pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltoPage {
    /// Page width in pixels.
    pub width: u32,
    /// Page height in pixels.
    pub height: u32,
    /// Regions in explicit reading order.
    pub regions: Vec<AltoRegion>,
}

/// One ALTO layout region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltoRegion {
    /// Region identifier.
    pub id: String,
    /// Region polygon in image coordinates.
    pub polygon: Vec<Point>,
    /// Lines in reading order.
    pub lines: Vec<AltoLine>,
}

/// Recognized line and its word-level confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltoLine {
    /// Line identifier.
    pub id: String,
    /// Line polygon.
    pub polygon: Vec<Point>,
    /// Recognized words in the engine's logical order.
    #[serde(default)]
    pub words: Vec<AltoWord>,
    /// Logical-order line content.
    pub text: String,
    /// Character-weighted word confidence.
    pub confidence: f32,
}

/// Recognized ALTO word with its original geometry and confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltoWord {
    /// Word identifier when supplied by the OCR engine.
    pub id: String,
    /// Word polygon in image coordinates.
    pub polygon: Vec<Point>,
    /// Unmodified recognized content.
    pub text: String,
    /// OCR engine confidence in the inclusive range 0 through 1.
    pub confidence: f32,
    /// Tesseract language selected for isolated word recognition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Whether that language comes from the word's structural role rather than
    /// from what was recognized. A lexicon headword is square Hebrew whatever
    /// the multilingual pass made of it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub structural_language: bool,
}

/// Assignment proving that every recognized line was accounted for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "entry_id", rename_all = "snake_case")]
pub enum LineAssignment {
    /// Assigned to a dictionary entry.
    Entry(String),
    /// Explicitly classified as front matter.
    FrontMatter,
    /// Retained but not parsed.
    Unparsed,
}

/// Parsing result for one page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedPage {
    /// One-based source PDF page represented by the assignments.
    #[serde(default)]
    pub source_page: u32,
    /// Parsed entries.
    pub entries: Vec<CorpusEntry>,
    /// Region/line to entry or fallback assignment.
    pub assignments: Vec<(String, String, LineAssignment)>,
}

/// Context required to create source-derived entries.
pub struct ParseContext<'a> {
    /// Edition identifier.
    pub edition: &'a str,
    /// Printed page label.
    pub printed_page: &'a str,
    /// One-based PDF page number.
    pub source_page: u32,
    /// Source PDF SHA-256.
    pub source_sha256: &'a str,
    /// Scan identifier.
    pub scan_id: &'a str,
    /// Content address of the pipeline run.
    pub pipeline_run: &'a str,
    /// Relative path to the preserved raster.
    pub page_image: &'a str,
    /// Applied transform identifier.
    pub transform_id: &'a str,
    /// Whether this page is known front matter.
    pub front_matter: bool,
}

/// Parses an ALTO page without discarding region or reading-order information.
pub fn parse_alto(xml: &str) -> Result<AltoPage> {
    let document = Document::parse(xml).context("invalid ALTO XML")?;
    let page_node = document
        .descendants()
        .find(|node| node.has_tag_name("Page"))
        .context("ALTO document has no Page element")?;
    let width = integer_attribute(page_node, "WIDTH")?;
    let height = integer_attribute(page_node, "HEIGHT")?;
    let mut regions = Vec::new();

    for (region_index, region_node) in page_node
        .descendants()
        .filter(|node| node.has_tag_name("TextBlock"))
        .enumerate()
    {
        let region_id = region_node
            .attribute("ID")
            .map_or_else(|| format!("region-{region_index:04}"), ToOwned::to_owned);
        let region_polygon = node_polygon(region_node)?;
        let mut lines = Vec::new();
        for (line_index, line_node) in region_node
            .descendants()
            .filter(|node| node.has_tag_name("TextLine"))
            .enumerate()
        {
            let id = line_node.attribute("ID").map_or_else(
                || format!("{region_id}-line-{line_index:04}"),
                ToOwned::to_owned,
            );
            let polygon = node_polygon(line_node)?;
            let mut text = String::new();
            let mut words = Vec::new();
            let mut weighted_confidence = 0.0_f32;
            let mut confidence_characters = 0_usize;
            for child in line_node.descendants().filter(|node| node.is_element()) {
                if child.has_tag_name("String") {
                    let content = child.attribute("CONTENT").unwrap_or_default();
                    if !text.is_empty() && !text.ends_with(char::is_whitespace) {
                        text.push(' ');
                    }
                    text.push_str(content);
                    let length = content.chars().count().max(1);
                    let confidence = child
                        .attribute("WC")
                        .and_then(|value| value.parse::<f32>().ok())
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0);
                    weighted_confidence += confidence * length as f32;
                    confidence_characters += length;
                    words.push(AltoWord {
                        id: child.attribute("ID").map_or_else(
                            || format!("{id}-word-{:04}", words.len() + 1),
                            ToOwned::to_owned,
                        ),
                        polygon: node_polygon(child)?,
                        text: content.to_owned(),
                        confidence,
                        language: None,
                        structural_language: false,
                    });
                } else if child.has_tag_name("SP") && !text.ends_with(' ') {
                    text.push(' ');
                } else if child.has_tag_name("HYP") {
                    text.push_str(child.attribute("CONTENT").unwrap_or("-"));
                }
            }
            let confidence = if confidence_characters == 0 {
                line_node
                    .attribute("WC")
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(0.0)
            } else {
                weighted_confidence / confidence_characters as f32
            };
            lines.push(AltoLine {
                id,
                polygon,
                words,
                text: text.trim().to_owned(),
                confidence,
            });
        }
        regions.push(AltoRegion {
            id: region_id,
            polygon: region_polygon,
            lines,
        });
    }
    if regions.is_empty() {
        bail!("ALTO page contains no TextBlock regions");
    }
    Ok(AltoPage {
        width,
        height,
        regions,
    })
}

/// Serializes the internal ALTO subset using ALTO 4.4 names.
#[must_use]
pub fn write_alto(page: &AltoPage, source_image: &str) -> String {
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <alto xmlns=\"http://www.loc.gov/standards/alto/ns-v4#\">\n\
         <Description><sourceImageInformation><fileName>",
    );
    escape_xml_into(source_image, &mut output);
    output.push_str("</fileName></sourceImageInformation></Description>\n<Layout>");
    let _ = write!(
        output,
        "<Page WIDTH=\"{}\" HEIGHT=\"{}\" PHYSICAL_IMG_NR=\"1\"><PrintSpace>",
        page.width, page.height
    );
    for region in &page.regions {
        let (x, y, width, height) = polygon_bounds(&region.polygon);
        let _ = write!(
            output,
            "<TextBlock ID=\"{}\" HPOS=\"{x}\" VPOS=\"{y}\" WIDTH=\"{width}\" HEIGHT=\"{height}\">",
            xml_escape(&region.id)
        );
        for line in &region.lines {
            let (x, y, width, height) = polygon_bounds(&line.polygon);
            let _ = write!(
                output,
                "<TextLine ID=\"{}\" HPOS=\"{x}\" VPOS=\"{y}\" WIDTH=\"{width}\" HEIGHT=\"{height}\">",
                xml_escape(&line.id)
            );
            if line.words.is_empty() {
                let _ = write!(
                    output,
                    "<String CONTENT=\"{}\" WC=\"{:.8}\" HPOS=\"{x}\" VPOS=\"{y}\" WIDTH=\"{width}\" HEIGHT=\"{height}\"/>",
                    xml_escape(&line.text),
                    line.confidence
                );
            } else {
                for word in &line.words {
                    let (word_x, word_y, word_width, word_height) = polygon_bounds(&word.polygon);
                    let _ = write!(
                        output,
                        "<String ID=\"{}\" CONTENT=\"{}\" WC=\"{:.8}\" HPOS=\"{word_x}\" VPOS=\"{word_y}\" WIDTH=\"{word_width}\" HEIGHT=\"{word_height}\"/>",
                        xml_escape(&word.id),
                        xml_escape(&word.text),
                        word.confidence
                    );
                }
            }
            output.push_str("</TextLine>");
        }
        output.push_str("</TextBlock>");
    }
    output.push_str("</PrintSpace></Page></Layout></alto>\n");
    output
}

/// Parses conservative entry boundaries while retaining both engine hypotheses.
///
/// An indented line beginning with a Hebrew-script word starts an entry. The
/// indentation check prevents an embedded Hebrew example at the start of a
/// continuation line from cutting the surrounding paragraph in two. Leading
/// non-margin content opens a headless fallback entry so section introductions
/// and page continuations are not discarded when no preceding page was
/// supplied. Known front matter is never coerced into lexical entries.
pub fn parse_entries(
    primary: (&AltoPage, &EngineIdentity),
    secondary: Option<(&AltoPage, &EngineIdentity)>,
    context: &ParseContext<'_>,
) -> ParsedPage {
    parse_entries_continuing(primary, secondary, context, None)
}

/// Parses entries with an optional entry continued from the preceding page.
///
/// Continuation is only supplied by the orchestrator for consecutive pages.
/// Margin lines remain explicitly unparsed rather than entering the continued
/// entry.
pub fn parse_entries_continuing(
    primary: (&AltoPage, &EngineIdentity),
    secondary: Option<(&AltoPage, &EngineIdentity)>,
    context: &ParseContext<'_>,
    continuation: Option<CorpusEntry>,
) -> ParsedPage {
    let mut hypotheses = vec![primary];
    hypotheses.extend(secondary);
    parse_entries_with_hypotheses_continuing(primary.0, &hypotheses, context, continuation)
}

/// Parses entries from derived canonical text while attaching every engine's
/// untouched, spatially aligned line hypothesis.
pub fn parse_entries_with_hypotheses(
    canonical: &AltoPage,
    hypotheses: &[(&AltoPage, &EngineIdentity)],
    context: &ParseContext<'_>,
) -> ParsedPage {
    parse_entries_with_hypotheses_continuing(canonical, hypotheses, context, None)
}

/// Parses entries with optional continuation and geometry-aligned hypotheses.
pub fn parse_entries_with_hypotheses_continuing(
    canonical: &AltoPage,
    hypotheses: &[(&AltoPage, &EngineIdentity)],
    context: &ParseContext<'_>,
    continuation: Option<CorpusEntry>,
) -> ParsedPage {
    let layout = PageLayout::from_page(canonical);
    let aligned_hypotheses: Vec<_> = hypotheses
        .iter()
        .map(|(page, _)| align_lines(canonical, page))
        .collect();
    let has_continuation = continuation.is_some();
    let mut entries: Vec<CorpusEntry> = continuation.into_iter().collect();
    let mut assignments = Vec::new();
    let mut page_entry_count = 0_usize;

    let mut previous_line: Option<(&str, &AltoLine)> = None;
    for (line_index, (region, line)) in flatten_lines(canonical).enumerate() {
        if context.front_matter {
            assignments.push((
                region.id.clone(),
                line.id.clone(),
                LineAssignment::FrontMatter,
            ));
            continue;
        }
        if is_margin_line(line, canonical.height)
            || is_isolated_page_artifact(line, region, canonical)
        {
            assignments.push((region.id.clone(), line.id.clone(), LineAssignment::Unparsed));
            if is_margin_line(line, canonical.height) {
                previous_line = None;
            }
            continue;
        }

        let first_line_in_region = region
            .lines
            .first()
            .is_some_and(|first_line| first_line.id == line.id);
        let canonical_grammar_candidate = grammar_labeled_headword_candidate(line);
        let hypothesis_grammar_candidate = aligned_hypotheses
            .iter()
            .filter_map(|aligned| aligned.get(line_index).and_then(|line| *line))
            .find_map(|hypothesis| {
                grammar_labeled_headword_candidate(hypothesis)
                    .map(|candidate_index| (hypothesis, candidate_index))
            });
        let grammar_labeled_headword =
            canonical_grammar_candidate.is_some() || hypothesis_grammar_candidate.is_some();
        let stem_heading = line_has_stem_heading(line, line_index, &aligned_hypotheses);
        let indented = layout.is_indented(line);
        let entry_indented = indented || is_indented_within_region(line, region);
        let grammar_indented = entry_indented
            || layout.is_indented_by(line, 0.6)
            || is_indented_within_region_by(line, region, 0.6);
        let grammar_boundary = grammar_labeled_headword && grammar_indented;
        let explicit_root_cue = line_has_explicit_root_cue(line, line_index, &aligned_hypotheses);
        let star_cue = line_has_star_cue(line, line_index, &aligned_hypotheses);
        let proper_name_cue = line_has_proper_name_cue(line, line_index, &aligned_hypotheses);
        let cross_reference_cue =
            line_has_cross_reference_cue(line, line_index, &aligned_hypotheses);
        let strong_region_cue = region_has_proper_name_cue(region);
        let allow_short_candidate = explicit_root_cue || star_cue || proper_name_cue;
        let canonical_structural_candidate =
            structural_headword_candidate(line, allow_short_candidate);
        let hypothesis_structural_candidate = aligned_hypotheses
            .iter()
            .filter_map(|aligned| aligned.get(line_index).and_then(|line| *line))
            .find_map(|hypothesis| {
                structural_headword_candidate(hypothesis, allow_short_candidate)
                    .map(|candidate_index| (hypothesis, candidate_index))
            });
        let structural_candidate =
            canonical_structural_candidate.is_some() || hypothesis_structural_candidate.is_some();
        let canonical_ocr_shape = canonical_structural_candidate
            .is_some_and(|index| candidate_has_ocr_shape(&line.words[index]));
        let (_, _, line_width, _) = polygon_bounds(&line.polygon);
        let short_line = line_width as f32 <= canonical.width as f32 * 0.2;
        let structural_boundary = structural_candidate
            && (explicit_root_cue
                || (star_cue && (first_line_in_region || entry_indented))
                || (first_line_in_region
                    && (strong_region_cue
                        || (entry_indented && canonical_ocr_shape && !short_line)))
                || (entry_indented && (proper_name_cue || cross_reference_cue)));
        let hebrew_boundary =
            begins_with_hebrew_headword(&line.text) && (first_line_in_region || entry_indented);
        let starts_entry = !stem_heading
            && (entries.is_empty() || grammar_boundary || structural_boundary || hebrew_boundary);
        let block_kind = if is_heading_line(line, region, canonical) {
            BlockKind::Heading
        } else {
            BlockKind::Paragraph
        };
        if starts_entry || entries.is_empty() {
            page_entry_count += 1;
            let ordinal = u32::try_from(page_entry_count).unwrap_or(u32::MAX);
            let entry_id = stable_entry_id(context.edition, context.printed_page, ordinal);
            entries.push(new_entry(&entry_id, ordinal, context));
        }
        let entry = entries
            .last_mut()
            .expect("a non-margin lexical line always has an entry");

        let line_hypotheses = hypotheses
            .iter()
            .zip(&aligned_hypotheses)
            .filter_map(|((_, identity), aligned)| {
                aligned
                    .get(line_index)
                    .and_then(Option::as_ref)
                    .map(|line| hypothesis(line, identity))
            })
            .collect();
        let span_index = entry.blocks.iter().map(|block| block.spans.len()).sum();
        let span = make_span(entry, line, region, line_hypotheses, context, span_index);
        if starts_entry && entry.headword.is_none() && !is_nonlexical_title(line, block_kind) {
            entry.headword = canonical_grammar_candidate
                .and_then(|candidate_index| {
                    extract_candidate_headword_at(&span, line, candidate_index)
                })
                .or_else(|| {
                    hypothesis_grammar_candidate.and_then(|(hypothesis, candidate_index)| {
                        aligned_candidate_index(line, &hypothesis.words[candidate_index]).and_then(
                            |canonical_index| {
                                extract_candidate_headword_at(&span, line, canonical_index)
                            },
                        )
                    })
                })
                .or_else(|| extract_headword(&span))
                .or_else(|| {
                    (structural_boundary || grammar_boundary || hebrew_boundary)
                        .then_some(canonical_structural_candidate)
                        .flatten()
                        .and_then(|candidate_index| {
                            extract_candidate_headword_at(&span, line, candidate_index)
                        })
                })
                .or_else(|| {
                    (structural_boundary || grammar_boundary || hebrew_boundary)
                        .then_some(hypothesis_structural_candidate)
                        .flatten()
                        .and_then(|(hypothesis, candidate_index)| {
                            aligned_candidate_index(line, &hypothesis.words[candidate_index])
                                .and_then(|canonical_index| {
                                    extract_candidate_headword_at(&span, line, canonical_index)
                                })
                        })
                });
        }
        let continues_block = !starts_entry
            && entry
                .blocks
                .last()
                .is_some_and(|block| block.kind == block_kind)
            && (previous_line.is_some_and(|(previous_region, previous)| {
                same_structural_block(
                    previous_region,
                    previous,
                    region,
                    line,
                    block_kind,
                    indented,
                )
            }) || (has_continuation
                && previous_line.is_none()
                && block_kind == BlockKind::Paragraph
                && !indented));
        if continues_block {
            entry
                .blocks
                .last_mut()
                .expect("continuing block exists")
                .spans
                .push(span);
        } else {
            entry.blocks.push(EntryBlock {
                id: format!("{}:block:{:04}", entry.id, entry.blocks.len() + 1),
                kind: block_kind,
                spans: vec![span],
            });
        }
        entry.confidence =
            aggregate_confidence(entry.blocks.iter().flat_map(|block| block.spans.iter()));
        assignments.push((
            region.id.clone(),
            line.id.clone(),
            LineAssignment::Entry(entry.id.clone()),
        ));
        previous_line = Some((&region.id, line));
    }

    ParsedPage {
        source_page: context.source_page,
        entries,
        assignments,
    }
}

fn is_heading_line(line: &AltoLine, region: &AltoRegion, page: &AltoPage) -> bool {
    let (x, _, width, _) = polygon_bounds(&line.polygon);
    let page_width = page.width as f32;
    let center_margin = page_width * 0.05;
    let spans_page_center = x as f32 <= page_width / 2.0 - center_margin
        && x.saturating_add(width) as f32 >= page_width / 2.0 + center_margin;
    let compact = width as f32 <= page_width * 0.65
        && line.text.chars().count() <= 100
        && line.text.split_whitespace().count() <= 12;
    let letters: Vec<_> = line
        .text
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    let uppercase = !letters.is_empty()
        && letters
            .iter()
            .filter(|character| character.is_uppercase())
            .count()
            * 5
            >= letters.len() * 4;
    let displayed = region.lines.len() <= 3 && (spans_page_center || uppercase);
    compact && displayed
}

fn is_nonlexical_title(line: &AltoLine, kind: BlockKind) -> bool {
    if kind != BlockKind::Heading {
        return false;
    }
    let letters = line
        .text
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect::<Vec<_>>();
    letters.len() >= 4
        && letters
            .iter()
            .all(|character| character.is_ascii_uppercase())
}

fn same_structural_block(
    previous_region: &str,
    previous: &AltoLine,
    region: &AltoRegion,
    line: &AltoLine,
    kind: BlockKind,
    indented: bool,
) -> bool {
    let (_, previous_y, _, previous_height) = polygon_bounds(&previous.polygon);
    let (_, y, _, height) = polygon_bounds(&line.polygon);
    let gap = y.saturating_sub(previous_y.saturating_add(previous_height)) as f32;
    let typical_height = previous_height.max(height).max(1) as f32;
    match kind {
        BlockKind::Heading => previous_region == region.id && gap <= typical_height,
        BlockKind::Paragraph => !indented,
        _ => false,
    }
}

struct PageLayout {
    page_width: u32,
    column_starts: [Option<u32>; 2],
}

impl PageLayout {
    fn from_page(page: &AltoPage) -> Self {
        let mut line_starts = [Vec::new(), Vec::new()];
        for (region, line) in flatten_lines(page) {
            let (_, _, width, _) = polygon_bounds(&line.polygon);
            if is_margin_line(line, page.height)
                || is_isolated_page_artifact(line, region, page)
                || is_heading_line(line, region, page)
                || width as f32 <= page.width as f32 * 0.2
            {
                continue;
            }
            let (x, _, _, _) = polygon_bounds(&line.polygon);
            line_starts[line_column(line, page.width)].push(x);
        }
        let column_starts = line_starts.map(|mut starts| {
            starts.sort_unstable();
            starts.get(starts.len() / 2).copied()
        });
        Self {
            page_width: page.width,
            column_starts,
        }
    }

    fn is_indented(&self, line: &AltoLine) -> bool {
        self.is_indented_by(line, 0.7)
    }

    fn is_indented_by(&self, line: &AltoLine, line_height_fraction: f32) -> bool {
        let (line_x, _, _, line_height) = polygon_bounds(&line.polygon);
        self.column_starts[line_column(line, self.page_width)].is_some_and(|column_start| {
            line_x.saturating_sub(column_start) as f32
                >= line_height.max(1) as f32 * line_height_fraction
        })
    }
}

fn line_column(line: &AltoLine, page_width: u32) -> usize {
    let (x, _, width, _) = polygon_bounds(&line.polygon);
    usize::from(x.saturating_add(width / 2) >= page_width / 2)
}

fn is_indented_within_region(line: &AltoLine, region: &AltoRegion) -> bool {
    is_indented_within_region_by(line, region, 0.7)
}

fn is_indented_within_region_by(
    line: &AltoLine,
    region: &AltoRegion,
    line_height_fraction: f32,
) -> bool {
    let (region_x, _, _, _) = polygon_bounds(&region.polygon);
    is_indented_from_by(line, region_x, line_height_fraction)
}

fn is_indented_from(line: &AltoLine, baseline_x: u32) -> bool {
    is_indented_from_by(line, baseline_x, 0.7)
}

fn is_indented_from_by(line: &AltoLine, baseline_x: u32, line_height_fraction: f32) -> bool {
    let (line_x, _, _, line_height) = polygon_bounds(&line.polygon);
    line_x.saturating_sub(baseline_x) as f32 >= line_height.max(1) as f32 * line_height_fraction
}

fn line_has_stem_heading(
    canonical: &AltoLine,
    line_index: usize,
    aligned_hypotheses: &[Vec<Option<&AltoLine>>],
) -> bool {
    std::iter::once(canonical)
        .chain(
            aligned_hypotheses
                .iter()
                .filter_map(|aligned| aligned.get(line_index).and_then(|line| *line)),
        )
        .any(|line| {
            line.text
                .split_whitespace()
                .next()
                .and_then(normalized_word)
                .is_some_and(|word| {
                    matches!(
                        word.as_str(),
                        "qal"
                            | "niph"
                            | "niphal"
                            | "nirr"
                            | "wiph"
                            | "piel"
                            | "pil"
                            | "pual"
                            | "hiph"
                            | "hiphil"
                            | "hipu"
                            | "hophal"
                            | "hithp"
                            | "hitap"
                            | "hithpael"
                    )
                })
        })
}

fn aligned_line_variants<'a>(
    canonical: &'a AltoLine,
    line_index: usize,
    aligned_hypotheses: &'a [Vec<Option<&'a AltoLine>>],
) -> impl Iterator<Item = &'a AltoLine> {
    std::iter::once(canonical).chain(
        aligned_hypotheses
            .iter()
            .filter_map(move |aligned| aligned.get(line_index).and_then(|line| *line)),
    )
}

fn line_has_explicit_root_cue(
    canonical: &AltoLine,
    line_index: usize,
    aligned_hypotheses: &[Vec<Option<&AltoLine>>],
) -> bool {
    aligned_line_variants(canonical, line_index, aligned_hypotheses).any(has_explicit_root_cue)
}

fn has_explicit_root_cue(line: &AltoLine) -> bool {
    let words: Vec<_> = line
        .words
        .iter()
        .map(|word| compact_word(&word.text))
        .collect();
    words.iter().any(|word| word == "root")
        || words
            .windows(2)
            .any(|pair| matches!(pair, [first, second] if first == "not" && second == "used"))
        || (words.first().is_some_and(|word| word != "prob")
            && words
                .windows(2)
                .any(|pair| matches!(pair, [first, second] if first == "prob" && second == "to")))
}

fn line_has_star_cue(
    canonical: &AltoLine,
    line_index: usize,
    aligned_hypotheses: &[Vec<Option<&AltoLine>>],
) -> bool {
    aligned_line_variants(canonical, line_index, aligned_hypotheses)
        .any(|line| line.text.trim_start().starts_with('*'))
}

fn line_has_proper_name_cue(
    canonical: &AltoLine,
    line_index: usize,
    aligned_hypotheses: &[Vec<Option<&AltoLine>>],
) -> bool {
    aligned_line_variants(canonical, line_index, aligned_hypotheses).any(has_proper_name_cue)
}

fn has_proper_name_cue(line: &AltoLine) -> bool {
    let words: Vec<_> = line
        .words
        .iter()
        .map(|word| compact_word(&word.text))
        .collect();
    words
        .windows(2)
        .any(|pair| matches!(pair, [first, second] if first == "pr" && second == "n"))
        || words.iter().any(|word| word.starts_with("prn"))
        || words
            .windows(2)
            .any(|pair| matches!(pair, [first, second] if first == "gentile" && second == "n"))
}

fn region_has_proper_name_cue(region: &AltoRegion) -> bool {
    region.lines.iter().any(has_proper_name_cue)
}

fn line_has_cross_reference_cue(
    canonical: &AltoLine,
    line_index: usize,
    aligned_hypotheses: &[Vec<Option<&AltoLine>>],
) -> bool {
    aligned_line_variants(canonical, line_index, aligned_hypotheses).any(|line| {
        line.words
            .iter()
            .take(4)
            .any(|word| compact_word(&word.text) == "see")
    })
}

/// Combines an English-primary page with foreign-script word runs from a
/// multilingual pass. Both original pages must still be retained as hypotheses.
#[must_use]
pub fn fuse_multilingual_words(primary: &AltoPage, multilingual: &AltoPage) -> AltoPage {
    let aligned: Vec<Option<AltoLine>> = align_lines(primary, multilingual)
        .into_iter()
        .map(|line| line.cloned())
        .collect();
    let mut fused = primary.clone();
    let mut line_index = 0_usize;
    for region in &mut fused.regions {
        for line in &mut region.lines {
            if let Some(multilingual_line) = aligned.get(line_index).and_then(Option::as_ref) {
                *line = fuse_line_words(line, multilingual_line);
            }
            line_index += 1;
        }
    }
    fused
}

/// Replaces weak Roman OCR tokens with a high-confidence, geometrically
/// matching reading from an independent recognizer.
///
/// The comparison is deliberately limited to lines without a distinctive
/// non-Roman script. General multilingual models can read the surrounding
/// English in mixed Hebrew or Syriac lines well while silently dropping vowel
/// points from the embedded word. Keeping those lines out of this stage makes
/// script-specific word recognition solely responsible for their contents.
#[must_use]
pub fn fuse_high_confidence_roman_words(primary: &AltoPage, secondary: &AltoPage) -> AltoPage {
    let aligned: Vec<Option<AltoLine>> = align_lines(primary, secondary)
        .into_iter()
        .map(|line| line.cloned())
        .collect();
    let mut fused = primary.clone();
    let mut line_index = 0_usize;
    for region in &mut fused.regions {
        for line in &mut region.lines {
            let Some(secondary_line) = aligned.get(line_index).and_then(Option::as_ref) else {
                line_index += 1;
                continue;
            };
            line_index += 1;
            if contains_foreign_script(&line.text) || contains_foreign_script(&secondary_line.text)
            {
                continue;
            }
            let mut changed = false;
            for word in &mut line.words {
                if word.confidence > 0.75 || !is_roman_word(&word.text) {
                    continue;
                }
                let Some((candidate, _overlap)) = secondary_line
                    .words
                    .iter()
                    .filter(|candidate| is_roman_word(&candidate.text))
                    .filter_map(|candidate| {
                        let overlap = polygon_iou(&word.polygon, &candidate.polygon);
                        (overlap >= 0.30).then_some((candidate, overlap))
                    })
                    .max_by(|left, right| {
                        left.1
                            .partial_cmp(&right.1)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                else {
                    continue;
                };
                let Some(primary_bounds) = word_bounds(word) else {
                    continue;
                };
                let Some(candidate_bounds) = word_bounds(candidate) else {
                    continue;
                };
                let primary_width = primary_bounds.2 - primary_bounds.0;
                let candidate_width = candidate_bounds.2 - candidate_bounds.0;
                let width_ratio = candidate_width / primary_width.max(f32::EPSILON);
                let comparable = (0.80..=1.25).contains(&width_ratio)
                    && normalized_disagreement(&word.text, &candidate.text) <= 0.34;
                if candidate.confidence >= 0.95 && comparable {
                    word.text.clone_from(&candidate.text);
                    word.confidence = candidate.confidence;
                    changed = true;
                }
            }
            if changed {
                line.text = join_words(&line.words);
                line.confidence = word_confidence(&line.words);
            }
        }
    }
    fused
}

fn is_roman_word(text: &str) -> bool {
    let alphabetic = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect::<Vec<_>>();
    !alphabetic.is_empty()
        && alphabetic
            .iter()
            .all(|character| character.script() == unicode_script::Script::Latin)
}

/// Tesseract model for the script the edition prints its lemmas in. Hebrew is
/// the dominant foreign script of the lexicon, so a word that is foreign but
/// otherwise unexplained is read as Hebrew before any other script.
pub const DOMINANT_WORD_LANGUAGE: &str = "heb";

/// Minimum score for a script the page explicitly announced, by printed label
/// or by recognized code points.
const MIN_ATTESTED_TRIAL_SCORE: f32 = 0.30;

/// Minimum score for a script that is only plausible from elsewhere on the
/// same line.
const MIN_CONTEXTUAL_TRIAL_SCORE: f32 = 0.45;

/// How far a non-dominant script must beat Hebrew before it is preferred.
const NON_DOMINANT_TRIAL_MARGIN: f32 = 0.10;

/// The same margin for a script that is only announced elsewhere on the line,
/// which says nothing about this particular word.
const ANNOUNCED_TRIAL_MARGIN: f32 = 0.25;

/// Smallest share of a reading that must be letters or their marks. Readings
/// below it are mostly stray punctuation from a noisy crop.
const MIN_TRIAL_CLEANLINESS: f32 = 0.60;

/// Confidence at which the multilingual pass's own script counts as evidence
/// strong enough to keep, rather than a guess to be re-arbitrated.
const CONFIDENT_DETECTION: f32 = 0.60;

/// One single-script recognition of an isolated word crop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptTrial {
    /// Tesseract model that produced the reading.
    pub language: String,
    /// Recognized text.
    pub text: String,
    /// Character-weighted confidence in the inclusive range 0 through 1.
    pub confidence: f32,
}

/// Classifies each foreign-script word into a single Tesseract language.
///
/// Words the multilingual pass already recognized in a distinctive script keep
/// that script as their route. Words it read as implausible Latin are routed to
/// the edition's dominant script instead of being abandoned as English; the
/// isolated recognition stage then re-reads them in every announced script and
/// arbitrates. Printed language labels can recover otherwise unreadable words,
/// but explicit Unicode script evidence is never replaced by neighbouring words
/// from another script.
#[must_use]
pub fn classify_word_languages(page: &AltoPage, supported_languages: &[String]) -> AltoPage {
    let layout = PageLayout::from_page(page);
    let mut classified = page.clone();
    for region in &mut classified.regions {
        let (region_x, _, _, _) = polygon_bounds(&region.polygon);
        for line in &mut region.lines {
            let is_indented = layout.is_indented(line) || is_indented_from(line, region_x);
            for word in &mut line.words {
                word.language = detected_word_language(&word.text)
                    .filter(|language| {
                        supported_languages
                            .iter()
                            .any(|supported| supported == language)
                    })
                    .map(ToOwned::to_owned);
            }
            apply_language_context_hints(&mut line.words, supported_languages);
            apply_headword_grammar_hint(&mut line.words, supported_languages, is_indented);
            apply_printed_label_route(&mut line.words, supported_languages);
            apply_dominant_script_fallback(&mut line.words, supported_languages);
        }
    }
    apply_starred_headword_hints(&mut classified, supported_languages);
    classified
}

/// Routes words a printed label governs to that label's model.
///
/// The English pass reads some Hebrew as digits and symbols rather than as
/// letters, so `Heb. 75%.` leaves nothing for script detection to work with. A
/// printed label is explicit enough to license re-reading such a word.
fn apply_printed_label_route(words: &mut [AltoWord], supported_languages: &[String]) {
    let labels = printed_label_languages(words);
    for (index, word) in words.iter_mut().enumerate() {
        if word.language.is_some() || !word.text.chars().any(char::is_alphanumeric) {
            continue;
        }
        if let Some(language) = labels[index].filter(|language| {
            supported_languages
                .iter()
                .any(|supported| supported == language)
        }) {
            word.language = Some(language.to_owned());
        }
    }
}

/// Routes words that were read as implausible Latin to the dominant script.
fn apply_dominant_script_fallback(words: &mut [AltoWord], supported_languages: &[String]) {
    if !supported_languages
        .iter()
        .any(|language| language == DOMINANT_WORD_LANGUAGE)
    {
        return;
    }
    for word in words {
        if word.language.is_none() && is_foreign_script_candidate(&word.text, word.confidence) {
            word.language = Some(DOMINANT_WORD_LANGUAGE.to_owned());
        }
    }
}

/// Whether a Latin-script reading is more plausibly a misrecognized
/// foreign-script word than real English.
///
/// The English pass has no Hebrew letters available, so pointed square Hebrew
/// comes back as Latin rubbish such as `R738` or `b%3`. Such readings mix
/// letters with digits, capitalize inside the word, reach for accented letters,
/// or contain no vowel at all, and the engine's own confidence in them is low.
/// Confident readings are left to the English pass, which keeps abbreviations
/// such as `Chr.` and roman numerals out of the foreign-script route.
#[must_use]
pub fn is_foreign_script_candidate(text: &str, confidence: f32) -> bool {
    if confidence >= 0.75 || contains_foreign_script(text) {
        return false;
    }
    let core = text.trim_matches(|character: char| !character.is_alphanumeric());
    let letters: Vec<char> = core
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    if letters.is_empty() {
        return false;
    }
    let interior_capital = core
        .chars()
        .skip(1)
        .any(|character| character.is_uppercase());
    let mixes_digits = core.chars().any(|character| character.is_ascii_digit());
    let accented = letters
        .iter()
        .any(|character| !character.is_ascii_alphabetic());
    let voiceless = letters.len() >= 3
        && !letters
            .iter()
            .any(|character| "aeiouyAEIOUY".contains(*character));
    interior_capital || mixes_digits || accented || voiceless
}

/// What the pipeline already knows about a word before its crop is arbitrated.
#[derive(Debug, Clone, Copy)]
pub struct WordScriptContext<'a> {
    /// Script the word was routed to before any isolated reading.
    pub routed: &'a str,
    /// Distinctive script the multilingual pass recognized, if any.
    pub detected: Option<&'a str>,
    /// That pass's confidence in its own reading.
    pub detected_confidence: f32,
    /// Script a printed label governs the word with, if any.
    pub label: Option<&'a str>,
    /// Scripts the line announced, by label or by recognized code points.
    pub announced: &'a [String],
}

/// Chooses the script that best explains an isolated word crop.
///
/// A reading is admissible only if it agrees with the model that produced it,
/// is not mostly punctuation from a noisy crop, and is in a script the page
/// announced — by printed label, by recognized code points on the line, or by
/// being the script the edition sets its lemmas in. Among admissible readings a
/// printed label decides outright, and so does a confident reading in a
/// distinctive script from the multilingual pass. Otherwise the best
/// agreement-weighted confidence wins, with the dominant script preferred
/// unless another beats it by a clear margin.
///
/// The score floors govern that competition between scripts. They are not a
/// quality bar on the text: the word has already been established as foreign,
/// so when no reading clears its floor the routed script's own reading is still
/// preferred to the Latin rubbish it would otherwise keep. Tesseract's
/// confidence in pointed display Hebrew is routinely near zero even when the
/// reading is exactly right, and the recorded confidence keeps such a word
/// below the review threshold either way.
#[must_use]
pub fn select_script_trial<'a>(
    trials: &'a [ScriptTrial],
    context: WordScriptContext<'_>,
) -> Option<&'a ScriptTrial> {
    let attested = |language: &str| {
        language == DOMINANT_WORD_LANGUAGE
            || context.detected == Some(language)
            || context.label == Some(language)
    };
    let admissible: Vec<(&ScriptTrial, f32)> = trials
        .iter()
        .filter(|trial| {
            word_matches_language(&trial.text, &trial.language)
                && trial_cleanliness(&trial.text) >= MIN_TRIAL_CLEANLINESS
                && (attested(&trial.language) || context.announced.contains(&trial.language))
        })
        .map(|trial| (trial, trial_score(trial)))
        .collect();
    if let Some(label) = context.label {
        if let Some((trial, _)) = admissible.iter().find(|(trial, _)| trial.language == label) {
            return Some(trial);
        }
    }
    if let Some(detected) = context
        .detected
        .filter(|_| context.detected_confidence >= CONFIDENT_DETECTION)
    {
        if let Some((trial, _)) = admissible
            .iter()
            .find(|(trial, _)| trial.language == detected)
        {
            return Some(trial);
        }
    }
    let competing: Vec<(&ScriptTrial, f32)> = admissible
        .iter()
        .copied()
        .filter(|(trial, score)| {
            let floor = if attested(&trial.language) {
                MIN_ATTESTED_TRIAL_SCORE
            } else {
                MIN_CONTEXTUAL_TRIAL_SCORE
            };
            *score >= floor
        })
        .collect();
    let dominant = admissible
        .iter()
        .find(|(trial, _)| trial.language == DOMINANT_WORD_LANGUAGE);
    if let Some((best, best_score)) = competing
        .iter()
        .copied()
        .max_by(|left, right| left.1.total_cmp(&right.1))
    {
        if best.language != DOMINANT_WORD_LANGUAGE {
            if let Some((dominant, dominant_score)) = dominant {
                // Being announced elsewhere on the line says nothing about
                // this word, so such a script needs a wider margin than one a
                // label or the word's own code points name.
                let margin = if attested(&best.language) {
                    NON_DOMINANT_TRIAL_MARGIN
                } else {
                    ANNOUNCED_TRIAL_MARGIN
                };
                if best_score < dominant_score + margin {
                    return Some(dominant);
                }
            }
        }
        return Some(best);
    }
    admissible
        .iter()
        .find(|(trial, _)| trial.language == context.routed)
        .map(|(trial, _)| *trial)
}

/// Confidence weighted by how much of the reading belongs to its own script.
fn trial_score(trial: &ScriptTrial) -> f32 {
    trial.confidence * script_agreement(&trial.text, &trial.language)
}

/// Share of a reading that is letters or the marks belonging to them.
fn trial_cleanliness(text: &str) -> f32 {
    let (linguistic, total) = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .fold((0_usize, 0_usize), |(linguistic, total), character| {
            let counted = character.is_alphabetic()
                || unicode_normalization::char::canonical_combining_class(character) != 0;
            (linguistic + usize::from(counted), total + 1)
        });
    if total == 0 {
        0.0
    } else {
        linguistic as f32 / total as f32
    }
}

fn script_agreement(text: &str, language: &str) -> f32 {
    use unicode_script::Script;
    let expected = match language {
        "heb" => Script::Hebrew,
        "ara" => Script::Arabic,
        "syr" => Script::Syriac,
        "grc" => Script::Greek,
        _ => return 0.0,
    };
    let (matching, alphabetic) = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .fold((0_usize, 0_usize), |(matching, alphabetic), character| {
            (
                matching + usize::from(character.script() == expected),
                alphabetic + 1,
            )
        });
    if alphabetic == 0 {
        0.0
    } else {
        matching as f32 / alphabetic as f32
    }
}

/// Scripts a line explicitly announces, by printed label or by code points the
/// multilingual pass already recognized.
#[must_use]
pub fn announced_line_languages(line: &AltoLine, supported_languages: &[String]) -> Vec<String> {
    let mut announced: Vec<String> = Vec::new();
    let labels = printed_label_languages(&line.words);
    for (index, word) in line.words.iter().enumerate() {
        for language in detected_word_language(&word.text)
            .into_iter()
            .chain(labels[index])
        {
            if supported_languages
                .iter()
                .any(|supported| supported == language)
                && !announced.iter().any(|existing| existing == language)
            {
                announced.push(language.to_owned());
            }
        }
    }
    announced
}

/// Tesseract model each word inherits from a printed language label.
///
/// A label governs the printed enumeration that follows it, so its scope
/// crosses list punctuation and the conjunctions the edition uses inside a
/// list, and ends at the first ordinary English word.
#[must_use]
pub fn printed_label_languages(words: &[AltoWord]) -> Vec<Option<&'static str>> {
    let mut governing: Vec<Option<&'static str>> = vec![None; words.len()];
    let mut active: Option<&'static str> = None;
    for (index, word) in words.iter().enumerate() {
        if let Some(language) =
            profile_for_label(&word.text).and_then(|profile| ocr_language_for_tag(profile.tag))
        {
            active = Some(language);
            continue;
        }
        if !word.text.chars().any(char::is_alphanumeric) {
            continue;
        }
        if contains_foreign_script(&word.text) || !is_plausible_english_word(&word.text) {
            governing[index] = active;
            continue;
        }
        if !is_list_conjunction(&word.text) {
            active = None;
        }
    }
    governing
}

fn is_list_conjunction(text: &str) -> bool {
    let word: String = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(word.as_str(), "and" | "or")
}

/// Maps a semantic language to the single-script Tesseract model that reads the
/// type the edition sets it in. Chaldee is printed in square Hebrew and Persian
/// in Arabic type, so both share a model with another language.
fn ocr_language_for_tag(tag: &str) -> Option<&'static str> {
    match tag {
        "he" | "arc" => Some("heb"),
        "ar" | "fa" => Some("ara"),
        "syr" => Some("syr"),
        "grc" => Some("grc"),
        _ => None,
    }
}

fn apply_headword_grammar_hint(
    words: &mut [AltoWord],
    supported_languages: &[String],
    is_indented: bool,
) {
    if !supported_languages.iter().any(|language| language == "heb") {
        return;
    }
    let candidate_index = words
        .first()
        .filter(|word| {
            word.text
                .chars()
                .all(|character| !character.is_alphanumeric())
        })
        .map_or(0, |_| 1);
    if !is_indented && candidate_index == 0 {
        return;
    }
    let Some(candidate) = words.get(candidate_index) else {
        return;
    };
    if !is_headword_candidate(candidate) {
        return;
    }
    if is_grammar_labeled_headword(words, candidate_index) {
        words[candidate_index].language = Some("heb".to_owned());
        words[candidate_index].structural_language = true;
    }
}

fn is_grammar_labeled_headword(words: &[AltoWord], candidate_index: usize) -> bool {
    let following: Vec<_> = words
        .iter()
        .skip(candidate_index + 1)
        .map(|word| compact_word(&word.text))
        .collect();
    let first = following.iter().find(|word| !word.is_empty());
    first.is_some_and(|word| is_direct_grammar_label(word))
        || words
            .get(candidate_index + 1)
            .is_some_and(|word| word.text.trim() == "£")
        || following
            .windows(2)
            .any(|pair| pair[0] == "pr" && pair[1] == "n")
        || following
            .windows(2)
            .any(|pair| pair[0] == "pers" && pair[1] == "pr")
        || following.iter().any(|word| word.starts_with("prn"))
        || following
            .windows(2)
            .any(|pair| pair[0] == "gentile" && pair[1] == "n")
        || ((begins_with_hebrew(&words[candidate_index].text)
            || !words[candidate_index]
                .text
                .chars()
                .any(|character| character.is_ascii_lowercase()))
            && matches!(
                following.as_slice(),
                [first, second, ..] if first == "in" && matches!(second.as_str(), "heb" | "hebr")
            ))
}

fn is_direct_grammar_label(word: &str) -> bool {
    matches!(
        word,
        "m" | "f"
            | "n"
            | "adj"
            | "adv"
            | "chald"
            | "constr"
            | "fut"
            | "plur"
            | "pil"
            | "piel"
            | "pual"
            | "hithp"
            | "hiph"
            | "hophal"
    )
}

fn normalized_word(word: &str) -> Option<String> {
    if word.chars().any(char::is_numeric) {
        return None;
    }
    let word = word.trim_matches(|character: char| !character.is_alphabetic());
    (!word.is_empty()).then(|| word.to_ascii_lowercase())
}

fn compact_word(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_candidate_stop_word(candidate: &str) -> bool {
    !candidate.chars().any(char::is_numeric)
        && matches!(
            compact_word(candidate).as_str(),
            "arab"
                | "arabic"
                | "chald"
                | "chaldee"
                | "syr"
                | "syriac"
                | "greek"
                | "gr"
                | "heb"
                | "hebr"
                | "pers"
                | "persian"
                | "talmud"
                | "note"
                | "deriv"
                | "hence"
                | "comp"
                | "pr"
                | "prn"
                | "m"
                | "f"
                | "n"
                | "adj"
                | "adv"
                | "plur"
                | "constr"
                | "qal"
                | "niph"
                | "niphal"
                | "nirr"
                | "wiph"
                | "piel"
                | "pil"
                | "pual"
                | "hiph"
                | "hiphil"
                | "hipu"
                | "hophal"
                | "hithp"
                | "hitap"
                | "hithpael"
        )
}

fn is_headword_candidate(candidate: &AltoWord) -> bool {
    let candidate_text = candidate
        .text
        .trim_matches(|character: char| !character.is_alphanumeric());
    let punctuated_number = candidate_text.chars().all(char::is_numeric)
        && candidate
            .text
            .chars()
            .any(|character| !character.is_alphanumeric());
    let lowercase_candidate = candidate_text
        .chars()
        .any(|character| character.is_alphabetic())
        && candidate_text
            .chars()
            .filter(|character| character.is_alphabetic())
            .all(|character| character.is_ascii_lowercase());
    !candidate_text.is_empty()
        && candidate_text.chars().count() <= 6
        && !lowercase_candidate
        && !is_candidate_stop_word(candidate_text)
        && !punctuated_number
}

fn is_structural_headword_candidate(candidate: &AltoWord, allow_short: bool) -> bool {
    use unicode_script::Script;

    let candidate_text = candidate
        .text
        .trim_matches(|character: char| !character.is_alphanumeric());
    let alphanumeric_count = candidate_text
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count();
    let alphabetic: Vec<_> = candidate_text
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    let numeric_count = candidate_text
        .chars()
        .filter(|character| character.is_numeric())
        .count();
    let foreign_script = alphabetic.iter().any(|character| {
        matches!(
            character.script(),
            Script::Hebrew | Script::Arabic | Script::Syriac | Script::Greek
        )
    });
    let ascii_titlecase = alphabetic.len() >= 2
        && alphabetic[0].is_ascii_uppercase()
        && alphabetic[1..]
            .iter()
            .all(|character| character.is_ascii_lowercase());
    let lowercase_ascii = !alphabetic.is_empty()
        && alphabetic
            .iter()
            .all(|character| character.is_ascii_lowercase());
    let starts_with_ascii_lowercase = alphabetic
        .first()
        .is_some_and(|character| character.is_ascii_lowercase());
    let mixed_single_letter_number = numeric_count > 0 && alphabetic.len() == 1 && foreign_script;

    !candidate_text.is_empty()
        && alphanumeric_count <= 16
        && (allow_short || alphanumeric_count >= 2)
        && !is_candidate_stop_word(candidate_text)
        && !ascii_titlecase
        && (!lowercase_ascii || foreign_script)
        && (!starts_with_ascii_lowercase || foreign_script)
        && (!mixed_single_letter_number || allow_short)
}

fn candidate_has_ocr_shape(candidate: &AltoWord) -> bool {
    candidate
        .text
        .chars()
        .any(|character| character.is_numeric() || !character.is_alphanumeric())
        || detected_word_language(&candidate.text).is_some()
}

fn structural_headword_candidate(line: &AltoLine, allow_short: bool) -> Option<usize> {
    let first_candidate = line.words.iter().enumerate().find(|(_, candidate)| {
        candidate
            .text
            .chars()
            .any(|character| character.is_alphanumeric())
    });
    if !allow_short {
        let candidate = first_candidate
            .filter(|(_, candidate)| is_structural_headword_candidate(candidate, false))
            .map(|(index, _)| index);
        if candidate.is_some() {
            return candidate;
        }
        let leading_single_digit = first_candidate.is_some_and(|(_, candidate)| {
            candidate.text.chars().count() == 1 && candidate.text.chars().all(char::is_numeric)
        });
        return leading_single_digit
            .then(|| {
                line.words
                    .iter()
                    .enumerate()
                    .skip(1)
                    .take(1)
                    .find(|(_, candidate)| {
                        is_structural_headword_candidate(candidate, false)
                            || (detected_word_language(&candidate.text).is_some()
                                && is_structural_headword_candidate(candidate, true))
                    })
                    .map(|(index, _)| index)
            })
            .flatten();
    }

    let candidates: Vec<_> = line
        .words
        .iter()
        .take(3)
        .enumerate()
        .filter(|(_, candidate)| is_structural_headword_candidate(candidate, true))
        .collect();
    candidates
        .iter()
        .find(|(_, candidate)| detected_word_language(&candidate.text).is_some())
        .or_else(|| candidates.first())
        .map(|(index, _)| *index)
}

fn grammar_labeled_headword_candidate(line: &AltoLine) -> Option<usize> {
    let candidate_index = line
        .words
        .first()
        .filter(|word| {
            word.text
                .chars()
                .all(|character| !character.is_alphanumeric())
        })
        .map_or(0, |_| 1);
    line.words
        .get(candidate_index)
        .is_some_and(|candidate| {
            is_headword_candidate(candidate)
                && is_grammar_labeled_headword(&line.words, candidate_index)
        })
        .then_some(candidate_index)
}

fn apply_starred_headword_hints(page: &mut AltoPage, supported_languages: &[String]) {
    if !supported_languages.iter().any(|language| language == "heb") {
        return;
    }
    let stars: Vec<_> = page
        .regions
        .iter()
        .flat_map(|region| &region.lines)
        .filter(|line| line.text.trim() == "*")
        .map(|line| polygon_bounds(&line.polygon))
        .collect();
    for line in page.regions.iter_mut().flat_map(|region| &mut region.lines) {
        let (line_x, line_y, _, line_height) = polygon_bounds(&line.polygon);
        let vertically_aligned = stars
            .iter()
            .any(|(star_x, star_y, star_width, star_height)| {
                let star_bottom = star_y.saturating_add(*star_height);
                let line_bottom = line_y.saturating_add(line_height);
                line_x >= star_x.saturating_add(*star_width)
                    && line_y < star_bottom
                    && *star_y < line_bottom
            });
        if vertically_aligned {
            if let Some(word) = line
                .words
                .first_mut()
                .filter(|word| word.language.is_some())
            {
                word.language = Some("heb".to_owned());
                word.structural_language = true;
            }
        }
    }
}

fn apply_language_context_hints(words: &mut [AltoWord], supported_languages: &[String]) {
    for index in 0..words.len() {
        let hinted = match words[index]
            .text
            .trim_matches(|character: char| !character.is_alphabetic())
            .to_ascii_lowercase()
            .as_str()
        {
            "heb" | "hebr" => Some("heb"),
            "arab" | "arabic" => Some("ara"),
            "syr" | "syriac" => Some("syr"),
            "gr" | "greek" => Some("grc"),
            _ => None,
        };
        let Some(language) = hinted.filter(|language| {
            supported_languages
                .iter()
                .any(|supported| supported == language)
        }) else {
            continue;
        };
        if let Some(candidate) = words.get_mut(index + 1).filter(|candidate| {
            candidate.language.is_none()
                && (candidate.confidence < 0.75 || suspicious_script_placeholder(&candidate.text))
        }) {
            candidate.language = Some(language.to_owned());
        }
    }

    for index in 0..words.len().saturating_sub(2) {
        let Some(language) = words[index].language.clone() else {
            continue;
        };
        if words[index + 1].text.eq_ignore_ascii_case("and")
            && words[index + 2].language.is_none()
            && (words[index + 2].confidence < 0.75
                || suspicious_script_placeholder(&words[index + 2].text))
        {
            words[index + 2].language = Some(language);
        }
    }
}

fn suspicious_script_placeholder(text: &str) -> bool {
    let alphabetic = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    let digits_or_symbols = text
        .chars()
        .filter(|character| character.is_ascii_digit() || character.is_ascii_punctuation())
        .count();
    alphabetic == 0 && digits_or_symbols > 0
}

/// Maps a recognized word's strong script to its single-language Tesseract
/// model. Latin remains with the English-primary pass because a single word is
/// insufficient to distinguish English from Latin reliably.
#[must_use]
pub fn detected_word_language(text: &str) -> Option<&'static str> {
    use unicode_script::Script;
    let mut counts = [0_usize; 4];
    for character in text.chars() {
        match character.script() {
            Script::Hebrew => counts[0] += 1,
            Script::Arabic => counts[1] += 1,
            Script::Syriac => counts[2] += 1,
            Script::Greek => counts[3] += 1,
            _ => {}
        }
    }
    counts
        .into_iter()
        .enumerate()
        .filter(|(_, count)| *count > 0)
        .max_by_key(|(_, count)| *count)
        .and_then(|(index, _)| match index {
            0 => Some("heb"),
            1 => Some("ara"),
            2 => Some("syr"),
            3 => Some("grc"),
            _ => None,
        })
}

/// Whether the alphabetic content agrees with the selected OCR language.
#[must_use]
pub fn word_matches_language(text: &str, language: &str) -> bool {
    use unicode_script::Script;
    let expected = match language {
        "heb" => Script::Hebrew,
        "ara" => Script::Arabic,
        "syr" => Script::Syriac,
        "grc" => Script::Greek,
        _ => return false,
    };
    let (matching, alphabetic) = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .fold((0_usize, 0_usize), |(matching, alphabetic), character| {
            (
                matching + usize::from(character.script() == expected),
                alphabetic + 1,
            )
        });
    alphabetic > 0 && matching * 3 >= alphabetic * 2
}

fn new_entry(id: &str, ordinal: u32, context: &ParseContext<'_>) -> CorpusEntry {
    CorpusEntry {
        id: id.to_owned(),
        aliases: Vec::new(),
        edition: context.edition.to_owned(),
        printed_page: context.printed_page.to_owned(),
        entry_ordinal: ordinal,
        headword: None,
        homograph: None,
        grammatical_labels: Vec::new(),
        blocks: Vec::new(),
        senses: Vec::new(),
        citations: Vec::new(),
        cross_references: Vec::new(),
        etymology: Vec::new(),
        provenance: EntryProvenance {
            edition: context.edition.to_owned(),
            source_sha256: context.source_sha256.to_owned(),
            scan_id: context.scan_id.to_owned(),
            pipeline_run: context.pipeline_run.to_owned(),
        },
        confidence: 0.0,
        review_state: ReviewState::Machine,
        revision: 0,
    }
}

fn make_span(
    entry: &CorpusEntry,
    line: &AltoLine,
    region: &AltoRegion,
    hypotheses: Vec<OcrHypothesis>,
    context: &ParseContext<'_>,
    index: usize,
) -> TextSpan {
    let diplomatic = without_bidi_controls(&line.text);
    let normalized = normalize_nfc(&diplomatic);
    let (language, language_runs) = identify_languages(&normalized, "en");
    let script = classify_script(&normalized);
    let direction = infer_direction(&normalized);
    TextSpan {
        id: format!("{}:span:{:04}", entry.id, index + 1),
        normalized,
        diplomatic: diplomatic.clone(),
        language,
        language_runs,
        script,
        direction,
        confidence: line.confidence,
        review_state: ReviewState::Machine,
        hypotheses,
        coordinates: vec![SourceCoordinate {
            source_page: context.source_page,
            printed_page: Some(context.printed_page.to_owned()),
            region_id: region.id.clone(),
            line_id: line.id.clone(),
            polygon: line.polygon.clone(),
            transform_id: context.transform_id.to_owned(),
            page_image: context.page_image.to_owned(),
        }],
        warnings: unicode_warnings(&diplomatic),
    }
}

fn extract_headword(line_span: &TextSpan) -> Option<TextSpan> {
    let mut characters = line_span
        .diplomatic
        .chars()
        .skip_while(|character| character.script() != unicode_script::Script::Hebrew);
    let headword: String = characters
        .by_ref()
        .take_while(|character| {
            let script = character.script();
            matches!(
                script,
                unicode_script::Script::Hebrew | unicode_script::Script::Inherited
            ) || *character == '-'
                || *character == '־'
        })
        .collect();
    if headword.is_empty() {
        return None;
    }
    let mut span = line_span.clone();
    span.id = format!("{}:headword", line_span.id.trim_end_matches(":span:0001"));
    span.diplomatic.clone_from(&headword);
    span.normalized = normalize_nfc(&headword);
    let default_language = headword_default_language(&headword, None);
    (span.language, span.language_runs) = identify_languages(&span.normalized, default_language);
    span.script = "Hebr".to_owned();
    span.direction = Direction::Rtl;
    span.warnings = unicode_warnings(&headword);
    Some(span)
}

fn extract_candidate_headword_at(
    line_span: &TextSpan,
    line: &AltoLine,
    candidate_index: usize,
) -> Option<TextSpan> {
    let candidate = line.words.get(candidate_index)?;
    let headword = candidate
        .text
        .trim_matches(|character: char| !character.is_alphanumeric());
    if headword.is_empty() {
        return None;
    }
    let mut span = line_span.clone();
    span.id = format!("{}:headword", line_span.id.trim_end_matches(":span:0001"));
    span.diplomatic = headword.to_owned();
    span.normalized = normalize_nfc(headword);
    let label = line
        .words
        .iter()
        .skip(candidate_index + 1)
        .take(3)
        .find_map(|word| {
            profile_for_label(&word.text)
                .filter(|profile| matches!(profile.tag, "he" | "arc"))
                .map(|profile| profile.tag)
        });
    let default_language = headword_default_language(headword, label);
    (span.language, span.language_runs) = identify_languages(&span.normalized, default_language);
    if label.is_some() || span.language.as_deref() != Some(default_language) {
        for run in &mut span.language_runs {
            run.language = default_language.to_owned();
            run.evidence = crate::model::LanguageEvidence::PrintedLabel;
        }
        if label.is_none() {
            for run in &mut span.language_runs {
                run.evidence = crate::model::LanguageEvidence::EditionDefault;
            }
        }
        span.language = (!span.language_runs.is_empty()).then(|| default_language.to_owned());
    }
    span.script = classify_script(headword);
    span.direction = infer_direction(headword);
    span.warnings = unicode_warnings(headword);
    Some(span)
}

fn headword_default_language<'a>(_headword: &str, printed_label: Option<&'a str>) -> &'a str {
    printed_label.unwrap_or("he")
}

fn aligned_candidate_index(canonical: &AltoLine, hypothesis: &AltoWord) -> Option<usize> {
    canonical
        .words
        .iter()
        .enumerate()
        .filter(|(_, word)| is_headword_candidate(word))
        .filter_map(|(index, word)| {
            let overlap = polygon_iou(&word.polygon, &hypothesis.polygon);
            (overlap >= 0.35).then_some((index, overlap))
        })
        .max_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
}

fn hypothesis(line: &AltoLine, identity: &EngineIdentity) -> OcrHypothesis {
    OcrHypothesis {
        engine: identity.engine.clone(),
        engine_version: identity.version.clone(),
        model: identity.model.clone(),
        model_hash: identity.model_hash.clone(),
        text: line.text.clone(),
        confidence: line.confidence,
    }
}

fn flatten_lines(page: &AltoPage) -> impl Iterator<Item = (&AltoRegion, &AltoLine)> {
    page.regions
        .iter()
        .flat_map(|region| region.lines.iter().map(move |line| (region, line)))
}

fn align_lines<'a>(canonical: &AltoPage, hypothesis: &'a AltoPage) -> Vec<Option<&'a AltoLine>> {
    let canonical_lines: Vec<_> = flatten_lines(canonical).map(|(_, line)| line).collect();
    let hypothesis_lines: Vec<_> = flatten_lines(hypothesis).map(|(_, line)| line).collect();
    let mut candidates = Vec::new();
    for (canonical_index, canonical_line) in canonical_lines.iter().enumerate() {
        for (hypothesis_index, hypothesis_line) in hypothesis_lines.iter().enumerate() {
            let overlap = polygon_iou(&canonical_line.polygon, &hypothesis_line.polygon);
            if overlap >= 0.15 {
                candidates.push((overlap, canonical_index, hypothesis_index));
            }
        }
    }
    candidates.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut result = vec![None; canonical_lines.len()];
    let mut claimed = vec![false; hypothesis_lines.len()];
    for (_, canonical_index, hypothesis_index) in candidates {
        if result[canonical_index].is_none() && !claimed[hypothesis_index] {
            result[canonical_index] = Some(hypothesis_lines[hypothesis_index]);
            claimed[hypothesis_index] = true;
        }
    }
    result
}

fn fuse_line_words(primary: &AltoLine, multilingual: &AltoLine) -> AltoLine {
    if primary.words.is_empty() || multilingual.words.is_empty() {
        return primary.clone();
    }
    let mut replacements = Vec::new();
    let mut run_start = None;
    let mut last_foreign = None;
    for (index, word) in multilingual.words.iter().enumerate() {
        if contains_foreign_script(&word.text) {
            run_start.get_or_insert(index);
            last_foreign = Some(index);
        } else if word.text.chars().any(char::is_alphabetic) {
            if let (Some(start), Some(end)) = (run_start.take(), last_foreign.take()) {
                add_replacement(primary, multilingual, start, end, &mut replacements);
            }
        }
    }
    if let (Some(start), Some(end)) = (run_start, last_foreign) {
        add_replacement(primary, multilingual, start, end, &mut replacements);
    }
    if replacements.is_empty() {
        return primary.clone();
    }

    replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.primary_start));
    let mut words = primary.words.clone();
    for replacement in replacements {
        words.splice(
            replacement.primary_start..=replacement.primary_end,
            multilingual.words[replacement.secondary_start..=replacement.secondary_end]
                .iter()
                .cloned(),
        );
    }
    for (index, word) in words.iter_mut().enumerate() {
        word.id = format!("{}-fused-word-{:04}", primary.id, index + 1);
    }
    let text = join_words(&words);
    AltoLine {
        id: primary.id.clone(),
        polygon: primary.polygon.clone(),
        confidence: word_confidence(&words),
        words,
        text,
    }
}

struct WordReplacement {
    primary_start: usize,
    primary_end: usize,
    secondary_start: usize,
    secondary_end: usize,
}

fn add_replacement(
    primary: &AltoLine,
    multilingual: &AltoLine,
    secondary_start: usize,
    secondary_end: usize,
    replacements: &mut Vec<WordReplacement>,
) {
    let secondary_bounds = combined_bounds(
        multilingual.words[secondary_start..=secondary_end]
            .iter()
            .flat_map(|word| word.polygon.iter()),
    );
    let Some(secondary_bounds) = secondary_bounds else {
        return;
    };
    let overlapping: Vec<_> = primary
        .words
        .iter()
        .enumerate()
        .filter(|(_, word)| bounds_overlap_fraction(word_bounds(word), secondary_bounds) >= 0.35)
        .map(|(index, _)| index)
        .collect();
    let (Some(primary_start), Some(primary_end)) =
        (overlapping.first().copied(), overlapping.last().copied())
    else {
        return;
    };
    if replacements.iter().any(|replacement| {
        primary_start <= replacement.primary_end && primary_end >= replacement.primary_start
    }) {
        return;
    }
    let primary_confidence = word_confidence(&primary.words[primary_start..=primary_end]);
    let secondary_confidence =
        word_confidence(&multilingual.words[secondary_start..=secondary_end]);
    let first_alphanumeric = multilingual.words.iter().position(|word| {
        word.text
            .chars()
            .any(|character| character.is_alphanumeric())
    });
    let is_headword = secondary_start == secondary_end
        && first_alphanumeric == Some(secondary_start)
        && is_grammar_labeled_headword(&multilingual.words, secondary_start);
    let labels = printed_label_languages(&multilingual.words);
    let explicitly_labelled = labels[secondary_start..=secondary_end]
        .iter()
        .any(Option::is_some);
    let isolated_plausible_latin = primary_start == primary_end
        && secondary_start == secondary_end
        && !explicitly_labelled
        && is_plausible_english_word(&primary.words[primary_start].text)
        && !is_foreign_script_candidate(
            &primary.words[primary_start].text,
            primary.words[primary_start].confidence,
        );
    // A multilingual page model can occasionally turn an ordinary low-
    // confidence Latin word into a confident word in another script. Keep the
    // layout pass's reading when the proposed replacement is an isolated,
    // unlabelled word and the Latin reading has none of the digit, symbol,
    // capitalization, or vowel defects that normally expose disguised Hebrew.
    // Multiword foreign runs and explicitly labelled citations remain eligible
    // for fusion.
    if !is_headword && isolated_plausible_latin {
        return;
    }
    if !is_headword
        && primary_confidence >= 0.70
        && primary.words[primary_start..=primary_end]
            .iter()
            .all(|word| is_plausible_english_word(&word.text))
    {
        return;
    }
    if !is_headword
        && primary_confidence >= 0.86
        && secondary_confidence < primary_confidence + 0.05
    {
        return;
    }
    replacements.push(WordReplacement {
        primary_start,
        primary_end,
        secondary_start,
        secondary_end,
    });
}

fn contains_foreign_script(text: &str) -> bool {
    use unicode_script::Script;
    text.chars().any(|character| {
        matches!(
            character.script(),
            Script::Hebrew | Script::Arabic | Script::Syriac | Script::Greek
        )
    })
}

fn is_plausible_english_word(text: &str) -> bool {
    let word = text
        .trim_matches(|character: char| !character.is_ascii_alphabetic())
        .to_ascii_lowercase();
    (word.len() >= 2
        && word
            .chars()
            .all(|character| character.is_ascii_alphabetic()))
        || matches!(
            word.as_str(),
            "a" | "an"
                | "and"
                | "as"
                | "at"
                | "be"
                | "by"
                | "for"
                | "from"
                | "has"
                | "he"
                | "in"
                | "into"
                | "is"
                | "it"
                | "its"
                | "no"
                | "not"
                | "of"
                | "on"
                | "or"
                | "that"
                | "the"
                | "their"
                | "there"
                | "these"
                | "this"
                | "to"
                | "was"
                | "were"
                | "which"
                | "with"
        )
}

pub(crate) fn word_confidence(words: &[AltoWord]) -> f32 {
    let (weighted, characters) = words.iter().fold((0.0_f32, 0_usize), |acc, word| {
        let length = word.text.chars().count().max(1);
        (acc.0 + word.confidence * length as f32, acc.1 + length)
    });
    if characters == 0 {
        0.0
    } else {
        weighted / characters as f32
    }
}

pub(crate) fn join_words(words: &[AltoWord]) -> String {
    let mut text = String::new();
    for word in words {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&word.text);
    }
    text
}

type Bounds = (f32, f32, f32, f32);

fn word_bounds(word: &AltoWord) -> Option<Bounds> {
    combined_bounds(word.polygon.iter())
}

fn combined_bounds<'a>(points: impl Iterator<Item = &'a Point>) -> Option<Bounds> {
    points.fold(None, |bounds, point| match bounds {
        None => Some((point.x, point.y, point.x, point.y)),
        Some((min_x, min_y, max_x, max_y)) => Some((
            min_x.min(point.x),
            min_y.min(point.y),
            max_x.max(point.x),
            max_y.max(point.y),
        )),
    })
}

fn bounds_overlap_fraction(left: Option<Bounds>, right: Bounds) -> f32 {
    let Some((left_x1, left_y1, left_x2, left_y2)) = left else {
        return 0.0;
    };
    let (right_x1, right_y1, right_x2, right_y2) = right;
    let intersection_width = (left_x2.min(right_x2) - left_x1.max(right_x1)).max(0.0);
    let intersection_height = (left_y2.min(right_y2) - left_y1.max(right_y1)).max(0.0);
    let intersection = intersection_width * intersection_height;
    let smallest_area = ((left_x2 - left_x1) * (left_y2 - left_y1))
        .min((right_x2 - right_x1) * (right_y2 - right_y1));
    if smallest_area <= f32::EPSILON {
        0.0
    } else {
        intersection / smallest_area
    }
}

fn begins_with_hebrew(text: &str) -> bool {
    use unicode_script::{Script, UnicodeScript};
    text.chars()
        .find(|character| character.is_alphanumeric())
        .is_some_and(|character| character.script() == Script::Hebrew)
}

fn begins_with_hebrew_headword(text: &str) -> bool {
    use unicode_script::Script;
    let mut characters = text
        .chars()
        .skip_while(|character| !character.is_alphanumeric());
    let Some(first) = characters.next() else {
        return false;
    };
    first.script() == Script::Hebrew
        && std::iter::once(first)
            .chain(characters)
            .take_while(|character| {
                matches!(character.script(), Script::Hebrew | Script::Inherited)
            })
            .filter(|character| character.script() == Script::Hebrew)
            .count()
            >= 2
}

fn is_margin_line(line: &AltoLine, page_height: u32) -> bool {
    if page_height == 0 || line.polygon.is_empty() {
        return false;
    }
    let minimum_y = line
        .polygon
        .iter()
        .map(|point| point.y)
        .fold(f32::MAX, f32::min);
    let maximum_y = line
        .polygon
        .iter()
        .map(|point| point.y)
        .fold(f32::MIN, f32::max);
    minimum_y < page_height as f32 * 0.05 || maximum_y > page_height as f32 * 0.975
}

fn is_isolated_page_artifact(line: &AltoLine, _region: &AltoRegion, page: &AltoPage) -> bool {
    let (_, _, width, height) = polygon_bounds(&line.polygon);
    line.text
        .chars()
        .all(|character| character.is_ascii_digit() || character.is_ascii_punctuation())
        && width as f32 <= page.width as f32 * 0.02
        && height as f32 <= page.height as f32 * 0.02
}

fn integer_attribute(node: Node<'_, '_>, attribute: &str) -> Result<u32> {
    node.attribute(attribute)
        .with_context(|| format!("{} has no {attribute}", node.tag_name().name()))?
        .parse()
        .with_context(|| format!("invalid {attribute} on {}", node.tag_name().name()))
}

fn node_polygon(node: Node<'_, '_>) -> Result<Vec<Point>> {
    if let Some(shape) = node
        .children()
        .find(|child| child.has_tag_name("Shape"))
        .and_then(|shape| shape.children().find(|child| child.has_tag_name("Polygon")))
        .and_then(|polygon| polygon.attribute("POINTS"))
    {
        let points = shape
            .split_whitespace()
            .map(|pair| {
                let (x, y) = pair
                    .split_once(',')
                    .with_context(|| format!("invalid ALTO polygon point `{pair}`"))?;
                Ok(Point {
                    x: x.parse()?,
                    y: y.parse()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if !points.is_empty() {
            return Ok(points);
        }
    }
    let x = integer_attribute(node, "HPOS")? as f32;
    let y = integer_attribute(node, "VPOS")? as f32;
    let width = integer_attribute(node, "WIDTH")? as f32;
    let height = integer_attribute(node, "HEIGHT")? as f32;
    Ok(vec![
        Point { x, y },
        Point { x: x + width, y },
        Point {
            x: x + width,
            y: y + height,
        },
        Point { x, y: y + height },
    ])
}

fn polygon_bounds(points: &[Point]) -> (u32, u32, u32, u32) {
    if points.is_empty() {
        return (0, 0, 0, 0);
    }
    let min_x = points.iter().map(|point| point.x).fold(f32::MAX, f32::min);
    let min_y = points.iter().map(|point| point.y).fold(f32::MAX, f32::min);
    let max_x = points.iter().map(|point| point.x).fold(f32::MIN, f32::max);
    let max_y = points.iter().map(|point| point.y).fold(f32::MIN, f32::max);
    (
        min_x.round().max(0.0) as u32,
        min_y.round().max(0.0) as u32,
        (max_x - min_x).round().max(0.0) as u32,
        (max_y - min_y).round().max(0.0) as u32,
    )
}

fn escape_xml_into(value: &str, output: &mut String) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
}

fn xml_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    escape_xml_into(value, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::{
        align_lines, classify_word_languages, fuse_high_confidence_roman_words,
        fuse_multilingual_words, is_foreign_script_candidate, join_words, parse_alto,
        select_script_trial, word_matches_language, write_alto, ScriptTrial, WordScriptContext,
    };

    const ALTO: &str = r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
<Layout><Page WIDTH="1200" HEIGHT="1800"><PrintSpace>
<TextBlock ID="b1" HPOS="10" VPOS="20" WIDTH="500" HEIGHT="100">
<TextLine ID="l1" HPOS="10" VPOS="20" WIDTH="500" HEIGHT="40">
<String CONTENT="אָב" WC="0.91" HPOS="10" VPOS="20" WIDTH="50" HEIGHT="40"/>
<SP WIDTH="5"/><String CONTENT="father" WC="0.99" HPOS="70" VPOS="20" WIDTH="100" HEIGHT="40"/>
</TextLine></TextBlock></PrintSpace></Page></Layout></alto>"#;

    const ENGLISH_PRIMARY: &str = r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
<Layout><Page WIDTH="500" HEIGHT="500"><PrintSpace>
<TextBlock ID="primary" HPOS="10" VPOS="100" WIDTH="400" HEIGHT="140">
<TextLine ID="primary-top" HPOS="10" VPOS="100" WIDTH="300" HEIGHT="40">
<String ID="primary-word-1" CONTENT="AB" WC="0.50" HPOS="10" VPOS="100" WIDTH="50" HEIGHT="40"/>
<String ID="primary-word-2" CONTENT="father" WC="0.98" HPOS="70" VPOS="100" WIDTH="100" HEIGHT="40"/>
</TextLine>
<TextLine ID="primary-bottom" HPOS="10" VPOS="200" WIDTH="300" HEIGHT="40">
<String ID="primary-word-3" CONTENT="with" WC="0.84" HPOS="10" VPOS="200" WIDTH="70" HEIGHT="40"/>
</TextLine>
</TextBlock></PrintSpace></Page></Layout></alto>"#;

    const MULTILINGUAL_REORDERED: &str = r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
<Layout><Page WIDTH="500" HEIGHT="500"><PrintSpace>
<TextBlock ID="multilingual" HPOS="10" VPOS="100" WIDTH="400" HEIGHT="140">
<TextLine ID="multilingual-bottom" HPOS="10" VPOS="200" WIDTH="300" HEIGHT="40">
<String ID="multilingual-word-3" CONTENT="מוושו" WC="0.80" HPOS="10" VPOS="200" WIDTH="70" HEIGHT="40"/>
</TextLine>
<TextLine ID="multilingual-top" HPOS="10" VPOS="100" WIDTH="300" HEIGHT="40">
<String ID="multilingual-word-1" CONTENT="אָב" WC="0.91" HPOS="10" VPOS="100" WIDTH="50" HEIGHT="40"/>
<String ID="multilingual-word-2" CONTENT="father" WC="0.97" HPOS="70" VPOS="100" WIDTH="100" HEIGHT="40"/>
</TextLine>
</TextBlock></PrintSpace></Page></Layout></alto>"#;

    #[test]
    fn reads_geometry_text_and_confidence() {
        let page = parse_alto(ALTO).unwrap();
        assert_eq!(page.width, 1200);
        assert_eq!(page.regions[0].lines[0].text, "אָב father");
        assert_eq!(page.regions[0].lines[0].words.len(), 2);
        assert_eq!(page.regions[0].lines[0].words[0].text, "אָב");
        assert_eq!(page.regions[0].lines[0].words[1].polygon[0].x, 70.0);
        assert!(page.regions[0].lines[0].confidence > 0.95);
    }

    #[test]
    fn generated_alto_round_trips() {
        let page = parse_alto(ALTO).unwrap();
        let generated = write_alto(&page, "page.png");
        let reparsed = parse_alto(&generated).unwrap();
        assert_eq!(reparsed.width, page.width);
        assert_eq!(reparsed.height, page.height);
        assert_eq!(reparsed.regions[0].id, page.regions[0].id);
        assert_eq!(
            reparsed.regions[0].lines[0].text,
            page.regions[0].lines[0].text
        );
        assert_eq!(
            reparsed.regions[0].lines[0].words,
            page.regions[0].lines[0].words
        );
        assert!(
            (reparsed.regions[0].lines[0].confidence - page.regions[0].lines[0].confidence).abs()
                < 0.000_001
        );
    }

    #[test]
    fn aligns_reordered_hypotheses_by_geometry() {
        let primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        let multilingual = parse_alto(MULTILINGUAL_REORDERED).unwrap();
        let aligned = align_lines(&primary, &multilingual);
        assert_eq!(aligned[0].unwrap().id, "multilingual-top");
        assert_eq!(aligned[1].unwrap().id, "multilingual-bottom");
    }

    #[test]
    fn fuses_low_confidence_foreign_words_but_keeps_confident_english() {
        let primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        let multilingual = parse_alto(MULTILINGUAL_REORDERED).unwrap();
        let fused = fuse_multilingual_words(&primary, &multilingual);
        assert_eq!(fused.regions[0].lines[0].text, "אָב father");
        assert_eq!(
            fused.regions[0].lines[0].words[0].id,
            "primary-top-fused-word-0001"
        );
        assert_eq!(
            fused.regions[0].lines[0].words[0].polygon,
            multilingual.regions[0].lines[1].words[0].polygon
        );
        assert_eq!(fused.regions[0].lines[1].text, "with");
        assert_eq!(fused.regions[0].lines[1].words[0].id, "primary-word-3");
    }

    #[test]
    fn fusion_keeps_an_isolated_unlabelled_latin_word() {
        let mut primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        let primary_line = &mut primary.regions[0].lines[0];
        primary_line.words.truncate(1);
        primary_line.words[0].text = "Quest.".to_owned();
        primary_line.words[0].confidence = 0.50;
        primary_line.text = "Quest.".to_owned();

        let mut multilingual = primary.clone();
        let multilingual_word = &mut multilingual.regions[0].lines[0].words[0];
        multilingual_word.text = "اوعدي".to_owned();
        multilingual_word.confidence = 0.75;
        multilingual.regions[0].lines[0].text = "اوعدي".to_owned();

        let fused = fuse_multilingual_words(&primary, &multilingual);

        assert_eq!(fused.regions[0].lines[0].text, "Quest.");
    }

    #[test]
    fn high_confidence_roman_fusion_corrects_only_comparable_weak_words() {
        let mut primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut primary.regions[0].lines[0];
        line.words[0].text = "Quest.".to_owned();
        line.words[0].confidence = 0.50;
        line.text = join_words(&line.words);

        let mut secondary = primary.clone();
        let line = &mut secondary.regions[0].lines[0];
        line.words[0].text = "Quæst.".to_owned();
        line.words[0].confidence = 0.99;
        line.text = join_words(&line.words);

        let fused = fuse_high_confidence_roman_words(&primary, &secondary);

        assert_eq!(fused.regions[0].lines[0].text, "Quæst. father");
        assert_eq!(fused.regions[0].lines[0].words[0].confidence, 0.99);
    }

    #[test]
    fn high_confidence_roman_fusion_keeps_mixed_script_lines() {
        let mut primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut primary.regions[0].lines[0];
        line.words[0].text = "Quest.".to_owned();
        line.words[0].confidence = 0.50;
        line.words[1].text = "אֶלֶף".to_owned();
        line.text = join_words(&line.words);

        let mut secondary = primary.clone();
        let line = &mut secondary.regions[0].lines[0];
        line.words[0].text = "Quæst.".to_owned();
        line.words[0].confidence = 0.99;
        line.words[1].text = "אלף".to_owned();
        line.text = join_words(&line.words);

        let fused = fuse_high_confidence_roman_words(&primary, &secondary);

        assert_eq!(fused.regions[0].lines[0].text, "Quest. אֶלֶף");
    }

    #[test]
    fn high_confidence_roman_fusion_rejects_partial_word_geometry() {
        let mut primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut primary.regions[0].lines[0];
        line.words[0].text = "Inrespect".to_owned();
        line.words[0].confidence = 0.50;
        line.text = join_words(&line.words);

        let mut secondary = primary.clone();
        let word = &mut secondary.regions[0].lines[0].words[0];
        word.text = "respect".to_owned();
        word.confidence = 0.99;
        word.polygon[1].x = word.polygon[0].x + 30.0;
        word.polygon[2].x = word.polygon[0].x + 30.0;
        secondary.regions[0].lines[0].text = join_words(&secondary.regions[0].lines[0].words);

        let fused = fuse_high_confidence_roman_words(&primary, &secondary);

        assert_eq!(fused.regions[0].lines[0].words[0].text, "Inrespect");
    }

    #[test]
    fn grammar_labels_do_not_turn_midline_english_into_foreign_headwords() {
        let mut primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        let primary_line = &mut primary.regions[0].lines[0];
        let template = primary_line.words[0].clone();
        primary_line.words = [
            ("Very", 10.0, 50.0, 0.91),
            ("frequent", 70.0, 100.0, 0.91),
            ("in", 180.0, 30.0, 0.91),
            ("Plur.", 220.0, 60.0, 0.76),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (text, x, width, confidence))| {
            let mut word = template.clone();
            word.id = format!("primary-{index}");
            word.text = text.to_owned();
            word.confidence = confidence;
            word.polygon = vec![
                crate::model::Point { x, y: 100.0 },
                crate::model::Point {
                    x: x + width,
                    y: 100.0,
                },
                crate::model::Point {
                    x: x + width,
                    y: 140.0,
                },
                crate::model::Point { x, y: 140.0 },
            ];
            word
        })
        .collect();

        let mut multilingual = primary.clone();
        multilingual.regions[0].lines[0].words[2].text = "מ1".to_owned();
        multilingual.regions[0].lines[0].words[2].confidence = 0.67;

        let fused = fuse_multilingual_words(&primary, &multilingual);
        assert_eq!(fused.regions[0].lines[0].words[2].text, "in");
    }

    #[test]
    fn grammar_labeled_headwords_override_plausible_latin_ocr_shapes() {
        let mut primary = parse_alto(ENGLISH_PRIMARY).unwrap();
        primary.regions[0].lines[0].words[0].confidence = 0.99;
        let mut multilingual = parse_alto(MULTILINGUAL_REORDERED).unwrap();
        let line = &mut multilingual.regions[0].lines[1];
        line.words[0].text = "אֶ".to_owned();
        line.words[0].confidence = 0.65;
        line.words[1].text = "m.".to_owned();

        let fused = fuse_multilingual_words(&primary, &multilingual);

        assert_eq!(fused.regions[0].lines[0].words[0].text, "אֶ");
    }

    #[test]
    fn latin_rubbish_is_routed_to_the_dominant_script_but_english_is_not() {
        for rubbish in ["R738", "b%3", "BXNE", "n\u{a5}13928,", "P82"] {
            assert!(
                is_foreign_script_candidate(rubbish, 0.30),
                "`{rubbish}` should be routed to a foreign script"
            );
        }
        for english in ["father", "the", "Comp.", "grape", "i.", "e."] {
            assert!(
                !is_foreign_script_candidate(english, 0.30),
                "`{english}` should stay with the English pass"
            );
        }
        // Confident readings are left alone, which keeps abbreviations and
        // roman numerals such as `Chr.`, `IX.` and `LEXICON.` out of the route.
        assert!(!is_foreign_script_candidate("IX.", 0.84));
        assert!(!is_foreign_script_candidate("LEXICON.", 0.85));
    }

    #[test]
    fn script_arbitration_prefers_the_dominant_script_without_ignoring_evidence() {
        let trial = |language: &str, text: &str, confidence: f32| ScriptTrial {
            language: language.to_owned(),
            text: text.to_owned(),
            confidence,
        };
        let announced = vec![
            "heb".to_owned(),
            "ara".to_owned(),
            "syr".to_owned(),
            "grc".to_owned(),
        ];
        let dominant_only = vec!["heb".to_owned()];
        fn context(announced: &[String]) -> WordScriptContext<'_> {
            WordScriptContext {
                routed: "heb",
                detected: None,
                detected_confidence: 0.0,
                label: None,
                announced,
            }
        }

        // Pointed Hebrew read as Latin rubbish by the English pass: Hebrew is
        // chosen even though another script scored slightly higher.
        let trials = vec![
            trial("heb", "אחד", 0.89),
            trial("ara", "و75", 0.55),
            trial("syr", "ܨܡ", 0.51),
        ];
        assert_eq!(
            select_script_trial(&trials, context(&announced)).map(|trial| &trial.language),
            Some(&"heb".to_owned())
        );

        // A clearly better non-dominant reading still wins.
        let trials = vec![trial("heb", "ווס", 0.59), trial("grc", "διὰ", 0.91)];
        assert_eq!(
            select_script_trial(&trials, context(&announced)).map(|trial| &trial.language),
            Some(&"grc".to_owned())
        );

        // A printed label decides outright.
        let trials = vec![trial("heb", "אחד", 0.89), trial("syr", "ܐܒܐ", 0.62)];
        assert_eq!(
            select_script_trial(
                &trials,
                WordScriptContext {
                    label: Some("syr"),
                    ..context(&announced)
                }
            )
            .map(|trial| &trial.language),
            Some(&"syr".to_owned())
        );

        // A script the line never announced is not introduced, and readings
        // that disagree with their own model are discarded, so nothing is left.
        let trials = vec![trial("syr", "ܫܗ", 0.82), trial("heb", "20", 0.53)];
        assert_eq!(select_script_trial(&trials, context(&dominant_only)), None);

        // A lexicon headword is square Hebrew whatever else the line announced,
        // so its structural route decides the script like a printed label.
        let trials = vec![trial("heb", "אָבֶר", 0.26), trial("syr", "ܡ", 0.62)];
        assert_eq!(
            select_script_trial(
                &trials,
                WordScriptContext {
                    label: Some("heb"),
                    ..context(&announced)
                }
            )
            .map(|trial| &trial.text),
            Some(&"אָבֶר".to_owned())
        );

        // Tesseract reports near-zero confidence for pointed display Hebrew
        // even when the reading is right. Nothing clears the competition floor,
        // so the routed script's reading is kept rather than Latin rubbish.
        let trials = vec![
            trial("heb", "אְבְיון", 0.0),
            trial("ara", "11", 0.61),
            trial("syr", "¡”5", 0.24),
        ];
        assert_eq!(
            select_script_trial(&trials, context(&dominant_only)).map(|trial| &trial.text),
            Some(&"אְבְיון".to_owned())
        );
    }

    #[test]
    fn classifies_each_foreign_word_without_overriding_explicit_script_evidence() {
        let mut page = parse_alto(MULTILINGUAL_REORDERED).unwrap();
        page.regions[0].lines[1].words = vec![
            page.regions[0].lines[1].words[0].clone(),
            page.regions[0].lines[1].words[0].clone(),
            page.regions[0].lines[0].words[0].clone(),
        ];
        page.regions[0].lines[1].words[0].text = "אָבִיךָ".to_owned();
        page.regions[0].lines[1].words[1].text = "הָרִאשׁוֹן".to_owned();
        page.regions[0].lines[1].words[2].text = "وو".to_owned();

        let classified = classify_word_languages(
            &page,
            &["eng", "heb", "ara", "syr", "grc", "lat"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
        );
        let languages = classified.regions[0].lines[1]
            .words
            .iter()
            .map(|word| word.language.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(languages, vec![Some("heb"), Some("heb"), Some("ara")]);
    }

    #[test]
    fn uses_printed_language_labels_and_enumeration_context() {
        let mut page = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut page.regions[0].lines[0];
        line.words[0].text = "Heb.".to_owned();
        line.words[1].text = "75%".to_owned();
        line.words[1].confidence = 0.0;
        let mut third = line.words[1].clone();
        third.text = "and".to_owned();
        third.confidence = 0.9;
        let mut fourth = line.words[1].clone();
        fourth.text = "2".to_owned();
        fourth.confidence = 0.4;
        line.words.extend([third, fourth]);

        let classified = classify_word_languages(&page, &["eng".to_owned(), "heb".to_owned()]);
        let words = &classified.regions[0].lines[0].words;
        assert_eq!(words[1].language.as_deref(), Some("heb"));
        assert_eq!(words[3].language.as_deref(), Some("heb"));
    }

    #[test]
    fn a_printed_label_governs_its_whole_list_and_ends_at_english_prose() {
        let mut page = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut page.regions[0].lines[0];
        let template = line.words[1].clone();
        line.words.clear();
        for (text, confidence) in [
            ("Comp.", 0.95),
            ("Syr.", 0.94),
            ("I.D'_*éo:", 0.49),
            (",", 0.90),
            ("321", 0.24),
            ("and", 0.93),
            ("2y", 0.30),
            ("flower", 0.96),
            ("75%.", 0.00),
        ] {
            let mut word = template.clone();
            word.text = text.to_owned();
            word.confidence = confidence;
            line.words.push(word);
        }
        let supported = ["eng".to_owned(), "heb".to_owned(), "syr".to_owned()];
        let classified = classify_word_languages(&page, &supported);
        let words = &classified.regions[0].lines[0].words;
        let routed: Vec<_> = words.iter().map(|word| word.language.as_deref()).collect();
        assert_eq!(
            routed,
            vec![
                None,
                None,
                // The label governs its citation, the digits the English pass
                // could not read as letters, and the rest of the list.
                Some("syr"),
                None,
                Some("syr"),
                None,
                Some("syr"),
                // English prose ends the citation. Without a label there is
                // nothing to distinguish a word the English pass read as digits
                // from a verse reference, so it keeps the English reading.
                None,
                None,
            ]
        );
    }

    #[test]
    fn treats_short_leading_ocr_shapes_before_grammar_labels_as_hebrew() {
        let mut page = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut page.regions[0].lines[0];
        line.words.truncate(2);
        line.words[0].text = "2".to_owned();
        line.words[1].text = "Chald.".to_owned();
        for point in &mut line.polygon {
            point.x += 50.0;
        }

        let classified = classify_word_languages(&page, &["eng".to_owned(), "heb".to_owned()]);

        assert_eq!(
            classified.regions[0].lines[0].words[0].language.as_deref(),
            Some("heb")
        );
    }

    #[test]
    fn treats_numeric_ocr_shape_before_in_hebrew_label_as_hebrew() {
        let mut page = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut page.regions[0].lines[0];
        line.words.truncate(2);
        line.words[0].text = "22".to_owned();
        line.words[1].text = "in".to_owned();
        let mut language = line.words[1].clone();
        language.text = "Heb.".to_owned();
        line.words.push(language);
        for point in &mut line.polygon {
            point.x += 50.0;
        }

        let classified = classify_word_languages(&page, &["eng".to_owned(), "heb".to_owned()]);

        assert_eq!(
            classified.regions[0].lines[0].words[0].language.as_deref(),
            Some("heb")
        );
    }

    #[test]
    fn treats_short_leading_ocr_shapes_before_proper_name_labels_as_hebrew() {
        let mut page = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut page.regions[0].lines[0];
        line.words[0].text = "RN".to_owned();
        line.words[1].text = "Abagtha,".to_owned();
        let mut pers = line.words[1].clone();
        pers.text = "Pers.".to_owned();
        let mut proper = line.words[1].clone();
        proper.text = "pr.".to_owned();
        line.words.extend([pers, proper]);
        for point in &mut line.polygon {
            point.x += 50.0;
        }

        let classified = classify_word_languages(&page, &["eng".to_owned(), "heb".to_owned()]);

        assert_eq!(
            classified.regions[0].lines[0].words[0].language.as_deref(),
            Some("heb")
        );
    }

    #[test]
    fn does_not_treat_numbered_senses_as_hebrew_headwords() {
        let mut page = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut page.regions[0].lines[0];
        line.words.truncate(2);
        line.words[0].text = "2.".to_owned();
        line.words[1].text = "m.".to_owned();
        for point in &mut line.polygon {
            point.x += 50.0;
        }

        let classified = classify_word_languages(&page, &["eng".to_owned(), "heb".to_owned()]);

        assert_eq!(classified.regions[0].lines[0].words[0].language, None);
    }

    #[test]
    fn does_not_treat_wrapped_lowercase_prose_as_a_headword() {
        let mut page = parse_alto(ENGLISH_PRIMARY).unwrap();
        let line = &mut page.regions[0].lines[0];
        line.words[0].text = "han,".to_owned();
        line.words[1].text = "1".to_owned();
        let mut person = line.words[1].clone();
        person.text = "pers.".to_owned();
        line.words.push(person);
        for point in &mut line.polygon {
            point.x += 50.0;
        }

        let classified = classify_word_languages(&page, &["eng".to_owned(), "heb".to_owned()]);

        assert_eq!(classified.regions[0].lines[0].words[0].language, None);
    }

    #[test]
    fn verifies_that_isolated_recognition_uses_the_selected_script() {
        assert!(word_matches_language("חָטָא", "heb"));
        assert!(word_matches_language("ܐܒܐ", "syr"));
        assert!(!word_matches_language("ܐܒܐ", "heb"));
        assert!(!word_matches_language("father", "heb"));
    }
}
