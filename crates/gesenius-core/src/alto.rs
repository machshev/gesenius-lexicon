//! Minimal ALTO 4 reader/writer and layout-aware structural parser.

use crate::metrics::polygon_iou;
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

        let grammar_labeled_headword = line_has_grammar_labeled_headword(line);
        let stem_heading = line_has_stem_heading(line, line_index, &aligned_hypotheses);
        let indented = layout.is_indented(line);
        let entry_indented = indented || is_indented_within_region(line, region);
        let starts_entry = (begins_with_hebrew(&line.text) || grammar_labeled_headword)
            && !stem_heading
            && (entries.is_empty() || entry_indented || grammar_labeled_headword);
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
        if starts_entry && entry.headword.is_none() {
            entry.headword = grammar_labeled_headword
                .then(|| extract_candidate_headword(&span, line))
                .flatten()
                .or_else(|| extract_headword(&span));
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
        let (line_x, _, _, line_height) = polygon_bounds(&line.polygon);
        self.column_starts[line_column(line, self.page_width)].is_some_and(|column_start| {
            line_x.saturating_sub(column_start) as f32 >= line_height.max(1) as f32 * 0.7
        })
    }
}

fn line_column(line: &AltoLine, page_width: u32) -> usize {
    let (x, _, width, _) = polygon_bounds(&line.polygon);
    usize::from(x.saturating_add(width / 2) >= page_width / 2)
}

fn is_indented_within_region(line: &AltoLine, region: &AltoRegion) -> bool {
    let (region_x, _, _, _) = polygon_bounds(&region.polygon);
    is_indented_from(line, region_x)
}

fn is_indented_from(line: &AltoLine, baseline_x: u32) -> bool {
    let (line_x, _, _, line_height) = polygon_bounds(&line.polygon);
    line_x.saturating_sub(baseline_x) as f32 >= line_height.max(1) as f32 * 0.7
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
                            | "piel"
                            | "pil"
                            | "pual"
                            | "hiph"
                            | "hiphil"
                            | "hipu"
                            | "hophal"
                            | "hithp"
                            | "hithpael"
                    )
                })
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

/// Classifies each explicitly foreign-script word into a single Tesseract
/// language. Adjacent foreign words are smoothed when one script is a lone
/// outlier in an otherwise consistent run.
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
            smooth_foreign_language_runs(&mut line.words);
            apply_headword_grammar_hint(&mut line.words, supported_languages, is_indented);
        }
    }
    apply_starred_headword_hints(&mut classified, supported_languages);
    classified
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
    }
}

fn is_grammar_labeled_headword(words: &[AltoWord], candidate_index: usize) -> bool {
    let following: Vec<_> = words
        .iter()
        .skip(candidate_index + 1)
        .take(3)
        .map(|word| normalized_word(&word.text).unwrap_or_default())
        .collect();
    following.iter().any(|word| {
        matches!(
            word.as_str(),
            "m" | "f"
                | "n"
                | "pr"
                | "proper"
                | "pers"
                | "chald"
                | "pil"
                | "piel"
                | "pual"
                | "hithp"
                | "hiph"
                | "hophal"
        )
    }) || ((begins_with_hebrew(&words[candidate_index].text)
        || !words[candidate_index]
            .text
            .chars()
            .any(|character| character.is_ascii_lowercase()))
        && matches!(
            following.as_slice(),
            [first, second, ..] if first == "in" && matches!(second.as_str(), "heb" | "hebr")
        ))
}

fn normalized_word(word: &str) -> Option<String> {
    if word.chars().any(char::is_numeric) {
        return None;
    }
    let word = word.trim_matches(|character: char| !character.is_alphabetic());
    (!word.is_empty()).then(|| word.to_ascii_lowercase())
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
    !candidate_text.is_empty()
        && candidate_text.chars().count() <= 6
        && !candidate_text
            .chars()
            .all(|character| character.is_ascii_lowercase())
        && !punctuated_number
}

fn line_has_grammar_labeled_headword(line: &AltoLine) -> bool {
    let candidate_index = line
        .words
        .first()
        .filter(|word| {
            word.text
                .chars()
                .all(|character| !character.is_alphanumeric())
        })
        .map_or(0, |_| 1);
    line.words.get(candidate_index).is_some_and(|candidate| {
        is_headword_candidate(candidate)
            && is_grammar_labeled_headword(&line.words, candidate_index)
    })
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

fn smooth_foreign_language_runs(words: &mut [AltoWord]) {
    let mut start = 0_usize;
    while start < words.len() {
        if words[start].language.is_none() {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < words.len() && words[end].language.is_some() {
            end += 1;
        }
        let run = &mut words[start..end];
        if run.len() >= 3
            && run.iter().all(|word| {
                word.text
                    .chars()
                    .filter(|character| character.is_alphabetic())
                    .count()
                    >= 2
            })
        {
            let mut counts = std::collections::BTreeMap::new();
            for language in run.iter().filter_map(|word| word.language.as_deref()) {
                *counts.entry(language.to_owned()).or_insert(0_usize) += 1;
            }
            if let Some((language, count)) = counts.into_iter().max_by_key(|(_, count)| *count) {
                if count * 3 >= run.len() * 2 {
                    for word in run {
                        word.language = Some(language.clone());
                    }
                }
            }
        }
        start = end;
    }
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
    TextSpan {
        id: format!("{}:span:{:04}", entry.id, index + 1),
        normalized: normalize_nfc(&diplomatic),
        diplomatic: diplomatic.clone(),
        language: None,
        script: classify_script(&diplomatic),
        direction: infer_direction(&diplomatic),
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
    span.script = "Hebr".to_owned();
    span.direction = Direction::Rtl;
    span.warnings = unicode_warnings(&headword);
    Some(span)
}

fn extract_candidate_headword(line_span: &TextSpan, line: &AltoLine) -> Option<TextSpan> {
    let candidate_index = line
        .words
        .first()
        .filter(|word| {
            word.text
                .chars()
                .all(|character| !character.is_alphanumeric())
        })
        .map_or(0, |_| 1);
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
    span.script = classify_script(headword);
    span.direction = infer_direction(headword);
    span.warnings = unicode_warnings(headword);
    Some(span)
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
    let is_headword = secondary_start == secondary_end
        && is_grammar_labeled_headword(&multilingual.words, secondary_start);
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

fn word_confidence(words: &[AltoWord]) -> f32 {
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

fn join_words(words: &[AltoWord]) -> String {
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
        align_lines, classify_word_languages, fuse_multilingual_words, parse_alto,
        word_matches_language, write_alto,
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
    fn classifies_each_foreign_word_and_smooths_an_isolated_script_error() {
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
        assert!(classified.regions[0].lines[1]
            .words
            .iter()
            .all(|word| word.language.as_deref() == Some("heb")));
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
