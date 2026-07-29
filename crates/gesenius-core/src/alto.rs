//! Minimal ALTO 4 reader/writer and conservative entry boundary parser.

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
    /// Logical-order line content.
    pub text: String,
    /// Character-weighted word confidence.
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
                "<TextLine ID=\"{}\" HPOS=\"{x}\" VPOS=\"{y}\" WIDTH=\"{width}\" HEIGHT=\"{height}\">\
                 <String CONTENT=\"{}\" WC=\"{:.8}\" HPOS=\"{x}\" VPOS=\"{y}\" WIDTH=\"{width}\" HEIGHT=\"{height}\"/>\
                 </TextLine>",
                xml_escape(&line.id),
                xml_escape(&line.text),
                line.confidence
            );
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
    let secondary_lines: Vec<&AltoLine> = secondary
        .map(|(page, _)| flatten_lines(page).map(|(_, line)| line).collect())
        .unwrap_or_default();
    let mut entries: Vec<CorpusEntry> = continuation.into_iter().collect();
    let mut assignments = Vec::new();
    let mut page_entry_count = 0_usize;

    for (line_index, (region, line)) in flatten_lines(primary.0).enumerate() {
        if context.front_matter {
            assignments.push((
                region.id.clone(),
                line.id.clone(),
                LineAssignment::FrontMatter,
            ));
            continue;
        }
        if is_margin_line(line, primary.0.height) {
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

        let hypotheses = make_hypotheses(
            line,
            primary.1,
            secondary_lines.get(line_index).copied(),
            secondary.map(|(_, identity)| identity),
        );
        let span = make_span(entry, line, region, hypotheses, context, entry.blocks.len());
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

fn make_hypotheses(
    primary: &AltoLine,
    primary_identity: &EngineIdentity,
    secondary: Option<&AltoLine>,
    secondary_identity: Option<&EngineIdentity>,
) -> Vec<OcrHypothesis> {
    let mut result = vec![hypothesis(primary, primary_identity)];
    if let (Some(line), Some(identity)) = (secondary, secondary_identity) {
        result.push(hypothesis(line, identity));
    }
    result
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
    use super::{parse_alto, write_alto};

    const ALTO: &str = r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
<Layout><Page WIDTH="1200" HEIGHT="1800"><PrintSpace>
<TextBlock ID="b1" HPOS="10" VPOS="20" WIDTH="500" HEIGHT="100">
<TextLine ID="l1" HPOS="10" VPOS="20" WIDTH="500" HEIGHT="40">
<String CONTENT="אָב" WC="0.91" HPOS="10" VPOS="20" WIDTH="50" HEIGHT="40"/>
<SP WIDTH="5"/><String CONTENT="father" WC="0.99" HPOS="70" VPOS="20" WIDTH="100" HEIGHT="40"/>
</TextLine></TextBlock></PrintSpace></Page></Layout></alto>"#;

    #[test]
    fn reads_geometry_text_and_confidence() {
        let page = parse_alto(ALTO).unwrap();
        assert_eq!(page.width, 1200);
        assert_eq!(page.regions[0].lines[0].text, "אָב father");
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
        assert!(
            (reparsed.regions[0].lines[0].confidence - page.regions[0].lines[0].confidence).abs()
                < 0.000_001
        );
    }
}
