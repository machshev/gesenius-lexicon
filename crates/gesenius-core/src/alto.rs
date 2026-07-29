//! Minimal ALTO 4 reader/writer and conservative entry boundary parser.

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
/// A line beginning with a Hebrew-script word starts an entry. Lines before the
/// first detected entry are explicitly `unparsed`; known front matter is never
/// coerced into lexical entries.
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
    let aligned_hypotheses: Vec<_> = hypotheses
        .iter()
        .map(|(page, _)| align_lines(canonical, page))
        .collect();
    let mut entries: Vec<CorpusEntry> = continuation.into_iter().collect();
    let mut assignments = Vec::new();
    let mut page_entry_count = 0_usize;

    for (line_index, (region, line)) in flatten_lines(canonical).enumerate() {
        if context.front_matter {
            assignments.push((
                region.id.clone(),
                line.id.clone(),
                LineAssignment::FrontMatter,
            ));
            continue;
        }
        if is_margin_line(line, canonical.height) {
            assignments.push((region.id.clone(), line.id.clone(), LineAssignment::Unparsed));
            continue;
        }

        let starts_entry = begins_with_hebrew(&line.text);
        if starts_entry {
            page_entry_count += 1;
            let ordinal = u32::try_from(page_entry_count).unwrap_or(u32::MAX);
            let entry_id = stable_entry_id(context.edition, context.printed_page, ordinal);
            entries.push(new_entry(&entry_id, ordinal, context));
        }
        let Some(entry) = entries.last_mut() else {
            assignments.push((region.id.clone(), line.id.clone(), LineAssignment::Unparsed));
            continue;
        };

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
        let span = make_span(
            entry,
            line,
            region,
            line_hypotheses,
            context,
            entry.blocks.len(),
        );
        if starts_entry && entry.headword.is_none() {
            entry.headword = extract_headword(&span);
        }
        entry.blocks.push(EntryBlock {
            id: format!("{}:block:{:04}", entry.id, entry.blocks.len() + 1),
            kind: BlockKind::Unclassified,
            spans: vec![span],
        });
        entry.confidence =
            aggregate_confidence(entry.blocks.iter().flat_map(|block| block.spans.iter()));
        assignments.push((
            region.id.clone(),
            line.id.clone(),
            LineAssignment::Entry(entry.id.clone()),
        ));
    }

    ParsedPage {
        entries,
        assignments,
    }
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
    let mut started = false;
    let headword: String = line_span
        .diplomatic
        .chars()
        .skip_while(|character| {
            !matches!(
                character.script(),
                unicode_script::Script::Hebrew
                    | unicode_script::Script::Inherited
                    | unicode_script::Script::Common
            )
        })
        .take_while(|character| {
            let script = character.script();
            let keep = matches!(
                script,
                unicode_script::Script::Hebrew | unicode_script::Script::Inherited
            ) || (started && (*character == '-' || *character == '־'));
            if matches!(script, unicode_script::Script::Hebrew) {
                started = true;
            }
            keep
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
    if primary_confidence >= 0.65
        && primary.words[primary_start..=primary_end]
            .iter()
            .all(|word| is_common_english_word(&word.text))
    {
        return;
    }
    if primary_confidence >= 0.86 && secondary_confidence < primary_confidence + 0.05 {
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

fn is_common_english_word(text: &str) -> bool {
    let word = text
        .trim_matches(|character: char| !character.is_ascii_alphabetic())
        .to_ascii_lowercase();
    matches!(
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
    minimum_y < page_height as f32 * 0.025 || maximum_y > page_height as f32 * 0.975
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
    use super::{align_lines, fuse_multilingual_words, parse_alto, write_alto};

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
}
