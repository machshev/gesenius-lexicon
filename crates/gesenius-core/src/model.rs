//! Authoritative corpus data model.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version of the JSON corpus schema implemented by this crate.
pub const CORPUS_SCHEMA_VERSION: &str = "1.1.0";

/// Version of the generated SQLite schema.
pub const SQLITE_SCHEMA_VERSION: u32 = 1;

/// Review state for machine- or human-produced text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Produced by OCR and not yet reviewed.
    #[default]
    Machine,
    /// Corrected by a reviewer but not independently verified.
    Corrected,
    /// Human-verified.
    Verified,
}

impl ReviewState {
    /// Returns the stable serialized representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::Corrected => "corrected",
            Self::Verified => "verified",
        }
    }
}

/// Unicode bidirectional class at the semantic span level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Left-to-right text.
    Ltr,
    /// Right-to-left text.
    Rtl,
    /// Mixed content whose child spans provide direction.
    Mixed,
}

/// A point in the original image coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate in pixels.
    pub x: f32,
    /// Vertical coordinate in pixels.
    pub y: f32,
}

/// Source geometry for a recognized span.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCoordinate {
    /// One-based PDF page number.
    pub source_page: u32,
    /// Printed page label when present.
    pub printed_page: Option<String>,
    /// ALTO region identifier.
    pub region_id: String,
    /// ALTO line identifier.
    pub line_id: String,
    /// Polygon in original-page coordinates.
    pub polygon: Vec<Point>,
    /// Identifier for the reversible preprocessing transform.
    pub transform_id: String,
    /// Content-addressed page image relative to the project root.
    pub page_image: String,
}

/// One OCR engine's unmodified hypothesis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrHypothesis {
    /// OCR engine name, such as `tesseract` or `kraken`.
    pub engine: String,
    /// Exact engine version reported by the executable.
    pub engine_version: String,
    /// Recognition model name.
    pub model: String,
    /// SHA-256 of the model bytes, or a composite digest for a model set.
    pub model_hash: String,
    /// Diplomatic hypothesis text.
    pub text: String,
    /// Confidence in the inclusive range 0 through 1.
    pub confidence: f32,
}

/// Unresolved or suspicious code point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnicodeWarning {
    /// Unicode scalar value rendered as `U+XXXX`.
    pub code_point: String,
    /// Character offset within the span.
    pub character_offset: usize,
    /// Stable warning code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Smallest reviewable text unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSpan {
    /// Entry-local stable span identifier.
    pub id: String,
    /// Diplomatic transcription preserving the recognizer or reviewer input.
    pub diplomatic: String,
    /// NFC-normalized text used by search and interchange.
    pub normalized: String,
    /// BCP 47 language tag when known.
    pub language: Option<String>,
    /// ISO 15924 script code.
    pub script: String,
    /// Semantic text direction.
    pub direction: Direction,
    /// Aggregate confidence in the inclusive range 0 through 1.
    pub confidence: f32,
    /// Current review state.
    pub review_state: ReviewState,
    /// Unmodified engine outputs; hypotheses are never automatically spliced.
    pub hypotheses: Vec<OcrHypothesis>,
    /// One or more source locations supporting this span.
    pub coordinates: Vec<SourceCoordinate>,
    /// Unicode and script diagnostics requiring review.
    pub warnings: Vec<UnicodeWarning>,
}

/// Kind of an ordered entry block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    /// A displayed title or section heading.
    Heading,
    /// One continuous prose paragraph.
    Paragraph,
    /// Headword and form material.
    Form,
    /// Grammatical information.
    Grammar,
    /// Sense prose.
    Definition,
    /// Etymological discussion.
    Etymology,
    /// Quoted or cited text.
    Citation,
    /// Cross-reference text.
    CrossReference,
    /// Content not yet classified.
    Unclassified,
}

/// Ordered structured or fallback content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryBlock {
    /// Stable entry-local block identifier.
    pub id: String,
    /// Block classification.
    pub kind: BlockKind,
    /// Ordered inline spans.
    pub spans: Vec<TextSpan>,
}

/// A parsed dictionary sense.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sense {
    /// Entry-local identifier.
    pub id: String,
    /// Printed label such as `1` or `b`.
    pub label: Option<String>,
    /// Ordered content.
    pub blocks: Vec<EntryBlock>,
}

/// A biblical or bibliographic citation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Citation {
    /// Entry-local identifier.
    pub id: String,
    /// Machine-parsed target when confident.
    pub target: Option<String>,
    /// Diplomatic citation text.
    pub text: TextSpan,
}

/// Link to another lexical entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossReference {
    /// Entry-local identifier.
    pub id: String,
    /// Stable target entry ID when resolved.
    pub target_entry_id: Option<String>,
    /// Printed target text.
    pub text: TextSpan,
}

/// Provenance shared by all content in an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryProvenance {
    /// Catalogue edition identifier.
    pub edition: String,
    /// SHA-256 of the source PDF.
    pub source_sha256: String,
    /// Stable scan identifier.
    pub scan_id: String,
    /// Pipeline content hash that produced the entry.
    pub pipeline_run: String,
}

/// One authoritative JSONL record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusEntry {
    /// Stable source-derived identifier.
    pub id: String,
    /// Historical identifiers retained after merges or splits.
    pub aliases: Vec<String>,
    /// Edition identifier.
    pub edition: String,
    /// Printed page label.
    pub printed_page: String,
    /// One-based entry ordinal on the printed page.
    pub entry_ordinal: u32,
    /// Primary headword when confidently detected.
    pub headword: Option<TextSpan>,
    /// Optional homograph number.
    pub homograph: Option<u32>,
    /// Grammatical labels in source order.
    pub grammatical_labels: Vec<TextSpan>,
    /// All content in reading order, including unclassified fallback blocks.
    pub blocks: Vec<EntryBlock>,
    /// Parsed senses.
    pub senses: Vec<Sense>,
    /// Parsed citations.
    pub citations: Vec<Citation>,
    /// Parsed cross-references.
    pub cross_references: Vec<CrossReference>,
    /// Etymological material when detected with sufficient confidence.
    pub etymology: Vec<EntryBlock>,
    /// Source provenance.
    pub provenance: EntryProvenance,
    /// Aggregate entry confidence.
    pub confidence: f32,
    /// Aggregate review state.
    pub review_state: ReviewState,
    /// Monotonically increasing optimistic-lock revision.
    pub revision: u64,
}

impl CorpusEntry {
    /// Iterates over every text span exactly once.
    pub fn spans(&self) -> impl Iterator<Item = &TextSpan> {
        self.headword
            .iter()
            .chain(self.grammatical_labels.iter())
            .chain(self.blocks.iter().flat_map(|block| block.spans.iter()))
            .chain(
                self.senses
                    .iter()
                    .flat_map(|sense| sense.blocks.iter())
                    .flat_map(|block| block.spans.iter()),
            )
            .chain(self.citations.iter().map(|citation| &citation.text))
            .chain(
                self.cross_references
                    .iter()
                    .map(|reference| &reference.text),
            )
            .chain(self.etymology.iter().flat_map(|block| block.spans.iter()))
    }

    /// Visits every editable span exactly once.
    pub fn for_each_span_mut(&mut self, mut visitor: impl FnMut(&mut TextSpan)) {
        if let Some(headword) = &mut self.headword {
            visitor(headword);
        }
        for span in &mut self.grammatical_labels {
            visitor(span);
        }
        for block in &mut self.blocks {
            for span in &mut block.spans {
                visitor(span);
            }
        }
        for sense in &mut self.senses {
            for block in &mut sense.blocks {
                for span in &mut block.spans {
                    visitor(span);
                }
            }
        }
        for citation in &mut self.citations {
            visitor(&mut citation.text);
        }
        for reference in &mut self.cross_references {
            visitor(&mut reference.text);
        }
        for block in &mut self.etymology {
            for span in &mut block.spans {
                visitor(span);
            }
        }
    }
}

/// Accuracy figures attached to an export.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccuracyMetrics {
    /// Character error rate overall.
    pub cer: Option<f64>,
    /// Word error rate overall.
    pub wer: Option<f64>,
    /// Per-script character error rates keyed by ISO 15924.
    pub cer_by_script: BTreeMap<String, f64>,
    /// Layout region intersection accuracy.
    pub layout_accuracy: Option<f64>,
    /// Entry-boundary precision.
    pub entry_boundary_precision: Option<f64>,
    /// Entry-boundary recall.
    pub entry_boundary_recall: Option<f64>,
}

/// Reproducibility and provenance manifest accompanying every export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusManifest {
    /// Corpus release version.
    pub corpus_version: String,
    /// Authoritative corpus schema version.
    pub schema_version: String,
    /// Pipeline Git commit or `dirty`.
    pub pipeline_commit: String,
    /// Stable generation timestamp. Use `SOURCE_DATE_EPOCH` for release builds.
    pub generated_at: DateTime<Utc>,
    /// Source PDF hashes keyed by edition.
    pub source_hashes: BTreeMap<String, String>,
    /// OCR model hashes keyed by model identity.
    pub model_hashes: BTreeMap<String, String>,
    /// Aggregate benchmark results.
    pub metrics: AccuracyMetrics,
    /// Explicit release maturity marker.
    pub draft: bool,
}

/// Creates a stable, source-derived entry identifier.
#[must_use]
pub fn stable_entry_id(edition: &str, printed_page: &str, ordinal: u32) -> String {
    fn component(value: &str) -> String {
        value
            .trim()
            .to_lowercase()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_owned()
    }

    format!(
        "{}:p{}:e{ordinal:04}",
        component(edition),
        component(printed_page)
    )
}

#[cfg(test)]
mod tests {
    use super::stable_entry_id;

    #[test]
    fn stable_ids_are_source_derived_and_sanitized() {
        assert_eq!(
            stable_entry_id("Robinson 1854", "xii / 3", 7),
            "robinson_1854:pxii___3:e0007"
        );
    }
}
