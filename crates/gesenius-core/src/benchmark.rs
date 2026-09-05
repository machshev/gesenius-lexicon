//! Immutable OCR gold fixtures and reproducible recognition evaluation.

use crate::alto::AltoPage;
use crate::metrics::{polygon_iou, recognition_metrics, RecognitionMetrics};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Human- or frontier-transcribed source lines used only for evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldBenchmark {
    /// Stable benchmark identifier.
    pub id: String,
    /// Edition containing the transcribed lines.
    pub edition: String,
    /// One-based source PDF page.
    pub source_page: u32,
    /// SHA-256 of the immutable source PDF.
    pub source_sha256: String,
    /// How the gold transcription was produced and checked.
    pub authority: String,
    /// Dimensions of the immutable page image used to draw source anchors.
    ///
    /// Required when the fixture uses coordinate alignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_image: Option<SourceImageGeometry>,
    /// Gold lines keyed to stable source-layout line identifiers.
    pub lines: Vec<GoldLine>,
}

impl GoldBenchmark {
    /// Loads and performs structural validation on a gold fixture.
    pub fn load(path: &Path) -> Result<Self> {
        let input = fs::read_to_string(path)
            .with_context(|| format!("failed to read benchmark {}", path.display()))?;
        let benchmark: Self = serde_json::from_str(&input)
            .with_context(|| format!("invalid benchmark {}", path.display()))?;
        benchmark.validate()?;
        Ok(benchmark)
    }

    fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty()
            || self.edition.trim().is_empty()
            || self.authority.trim().is_empty()
        {
            bail!("benchmark identity and authority must not be empty");
        }
        if self.source_page == 0 {
            bail!("benchmark source page must be one-based");
        }
        if self.source_sha256.len() != 64
            || !self
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("benchmark source_sha256 must be a SHA-256 digest");
        }
        if self.lines.is_empty() {
            bail!("benchmark must contain at least one line");
        }
        let mut seen = BTreeMap::new();
        let anchored_lines = self
            .lines
            .iter()
            .filter(|line| line.source.is_some())
            .count();
        if anchored_lines != 0 && anchored_lines != self.lines.len() {
            bail!("benchmark must anchor every line or use legacy line IDs throughout");
        }
        if anchored_lines != 0 && self.source_image.is_none() {
            bail!("coordinate-aligned benchmark requires source_image dimensions");
        }
        if self.source_image.as_ref().is_some_and(|image| {
            image.width == 0 || image.height == 0 || image.coordinate_frame.trim().is_empty()
        }) {
            bail!("benchmark source_image dimensions must be non-zero");
        }
        let source_image = self.source_image.as_ref();
        for line in &self.lines {
            if line.line_id.trim().is_empty() || line.text.trim().is_empty() {
                bail!("benchmark line IDs and text must not be empty");
            }
            if seen.insert(&line.line_id, ()).is_some() {
                bail!("benchmark repeats line ID `{}`", line.line_id);
            }
            if let Some(source) = &line.source {
                if source.source_page != self.source_page || !valid_anchor(source, source_image) {
                    bail!(
                        "benchmark source anchor for `{}` must be finite, non-degenerate bounds within the fixture page image",
                        line.line_id
                    );
                }
            }
        }
        Ok(())
    }
}

/// Pixel dimensions for the source-page image coordinate system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceImageGeometry {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Caller-defined identifier for this image coordinate frame and transform.
    pub coordinate_frame: String,
}

/// One exact diplomatic line transcription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldLine {
    /// Legacy ALTO line identifier used only when coordinate anchors are absent.
    pub line_id: String,
    /// Exact Unicode transcription in logical order.
    pub text: String,
    /// Source-page geometry for alignment across engine segmentation changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<GoldLineSource>,
}

/// Immutable rectangular source-page anchor for a gold line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldLineSource {
    /// One-based source PDF page containing this transcription.
    pub source_page: u32,
    /// Axis-aligned line bounds in the asserted source-image coordinate frame.
    pub bounds: SourceBounds,
}

/// Axis-aligned rectangle in source-image pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SourceBounds {
    /// Left edge in pixels.
    pub x: f32,
    /// Top edge in pixels.
    pub y: f32,
    /// Positive width in pixels.
    pub width: f32,
    /// Positive height in pixels.
    pub height: f32,
}

/// Asserted source identity supplied by the caller for the ALTO page under evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    /// Edition identifier.
    pub edition: String,
    /// One-based source PDF page.
    pub source_page: u32,
    /// SHA-256 of the source PDF.
    pub source_sha256: String,
    /// Asserted image coordinate frame and transform, when coordinate scoring is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_frame: Option<String>,
}

/// Whether a caller supplied an asserted source identity matching the gold fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceIdentityVerification {
    /// The caller supplied an asserted identity matching the gold fixture exactly.
    Verified,
    /// Legacy callers supplied no source identity, so scores cannot prove page provenance.
    Unverified,
}

/// Method used to align gold text with an ALTO hypothesis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentMethod {
    /// All gold lines have source geometry and were aligned by overlap.
    SourceCoordinates,
    /// Legacy fixture without source geometry, aligned by engine-generated line IDs.
    LegacyLineIds,
}

/// Treatment of source-line boundaries before recognition metrics are calculated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextBoundaryPolicy {
    /// Legacy line-ID comparison preserves a newline after every gold line.
    ExactLineBreaks,
    /// Coordinate comparison replaces selected line boundaries with one space.
    CoordinateWhitespaceNormalized,
}

/// Recognition accuracy for one ALTO hypothesis against immutable gold lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Gold fixture identifier.
    pub benchmark_id: String,
    /// Aggregate metrics in fixture line order.
    pub metrics: RecognitionMetrics,
    /// Gold line IDs absent from the hypothesis.
    pub missing_lines: Vec<String>,
    /// Whether the caller supplied matching asserted edition, page, and source PDF identity.
    pub source_identity: SourceIdentityVerification,
    /// Alignment evidence used for this result.
    pub alignment: AlignmentMethod,
    /// How line boundaries were represented in the scored text streams.
    pub text_boundary_policy: TextBoundaryPolicy,
}

/// Evaluates a legacy fixture by engine line ID without caller-provided source identity.
///
/// The resulting score is explicitly marked unverified. New code should call
/// [`evaluate_alto_with_identity`] after resolving the ALTO artifact's source.
#[must_use]
pub fn evaluate_alto(benchmark: &GoldBenchmark, page: &AltoPage) -> BenchmarkResult {
    evaluate_with_line_ids(benchmark, page, SourceIdentityVerification::Unverified)
}

/// Evaluates only explicitly gold-transcribed regions after checking identity.
///
/// If every gold line has a source anchor, the evaluator concatenates each
/// anchored gold line and each overlapping ALTO line in page reading order.
/// This makes text scoring independent of engine line IDs and preserves one
/// hypothesis line when an engine merges lines or splits one into several.
pub fn evaluate_alto_with_identity(
    benchmark: &GoldBenchmark,
    page: &AltoPage,
    identity: Option<&SourceIdentity>,
) -> Result<BenchmarkResult> {
    benchmark.validate()?;
    let source_identity = verify_identity(benchmark, identity)?;
    if benchmark.lines.iter().all(|line| line.source.is_some()) {
        if source_identity != SourceIdentityVerification::Verified {
            bail!("coordinate-aligned benchmark requires an asserted matching source identity");
        }
        evaluate_with_coordinates(benchmark, page, source_identity)
    } else {
        Ok(evaluate_with_line_ids(benchmark, page, source_identity))
    }
}

fn valid_anchor(source: &GoldLineSource, source_image: Option<&SourceImageGeometry>) -> bool {
    let Some(source_image) = source_image else {
        return false;
    };
    let bounds = source.bounds;
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width <= 0.0
        || bounds.height <= 0.0
    {
        return false;
    }
    bounds.x >= 0.0
        && bounds.y >= 0.0
        && bounds.x + bounds.width <= source_image.width as f32
        && bounds.y + bounds.height <= source_image.height as f32
}

fn anchor_polygon(anchor: &GoldLineSource) -> Vec<crate::model::Point> {
    let bounds = anchor.bounds;
    vec![
        crate::model::Point {
            x: bounds.x,
            y: bounds.y,
        },
        crate::model::Point {
            x: bounds.x + bounds.width,
            y: bounds.y,
        },
        crate::model::Point {
            x: bounds.x + bounds.width,
            y: bounds.y + bounds.height,
        },
        crate::model::Point {
            x: bounds.x,
            y: bounds.y + bounds.height,
        },
    ]
}

fn verify_identity(
    benchmark: &GoldBenchmark,
    identity: Option<&SourceIdentity>,
) -> Result<SourceIdentityVerification> {
    let Some(identity) = identity else {
        return Ok(SourceIdentityVerification::Unverified);
    };
    if identity.edition != benchmark.edition
        || identity.source_page != benchmark.source_page
        || identity.source_sha256 != benchmark.source_sha256
    {
        bail!(
            "ALTO source identity does not match benchmark `{}` (expected edition `{}`, page {}, SHA-256 {}; got edition `{}`, page {}, SHA-256 {})",
            benchmark.id,
            benchmark.edition,
            benchmark.source_page,
            benchmark.source_sha256,
            identity.edition,
            identity.source_page,
            identity.source_sha256,
        );
    }
    if let Some(source_image) = &benchmark.source_image {
        if identity.coordinate_frame.as_deref() != Some(source_image.coordinate_frame.as_str()) {
            bail!("ALTO coordinate frame does not match the coordinate-aligned benchmark");
        }
    }
    Ok(SourceIdentityVerification::Verified)
}

fn evaluate_with_line_ids(
    benchmark: &GoldBenchmark,
    page: &AltoPage,
    source_identity: SourceIdentityVerification,
) -> BenchmarkResult {
    let hypotheses: BTreeMap<_, _> = page
        .regions
        .iter()
        .flat_map(|region| region.lines.iter())
        .map(|line| (line.id.as_str(), line.text.as_str()))
        .collect();
    let mut reference = String::new();
    let mut hypothesis = String::new();
    let mut missing_lines = Vec::new();
    for line in &benchmark.lines {
        reference.push_str(&line.text);
        reference.push('\n');
        if let Some(text) = hypotheses.get(line.line_id.as_str()) {
            hypothesis.push_str(text);
        } else {
            missing_lines.push(line.line_id.clone());
        }
        hypothesis.push('\n');
    }
    BenchmarkResult {
        benchmark_id: benchmark.id.clone(),
        metrics: recognition_metrics(&reference, &hypothesis),
        missing_lines,
        source_identity,
        alignment: AlignmentMethod::LegacyLineIds,
        text_boundary_policy: TextBoundaryPolicy::ExactLineBreaks,
    }
}

fn evaluate_with_coordinates(
    benchmark: &GoldBenchmark,
    page: &AltoPage,
    source_identity: SourceIdentityVerification,
) -> Result<BenchmarkResult> {
    let source_image = benchmark
        .source_image
        .as_ref()
        .expect("coordinate mode requires validated source image dimensions");
    if page.width != source_image.width || page.height != source_image.height {
        bail!(
            "ALTO page dimensions {}x{} do not match benchmark source image {}x{}",
            page.width,
            page.height,
            source_image.width,
            source_image.height,
        );
    }
    let hypotheses: Vec<_> = page
        .regions
        .iter()
        .flat_map(|region| region.lines.iter())
        .collect();
    let mut selected = vec![false; hypotheses.len()];
    let mut missing_lines = Vec::new();
    for gold in &benchmark.lines {
        let anchor = gold.source.as_ref().expect("all sources checked above");
        let mut overlaps_anchor = false;
        for (index, hypothesis) in hypotheses.iter().enumerate() {
            if polygon_iou(&anchor_polygon(anchor), &hypothesis.polygon) > 0.0 {
                selected[index] = true;
                overlaps_anchor = true;
            }
        }
        if !overlaps_anchor {
            missing_lines.push(gold.line_id.clone());
        }
    }
    let reference = join_coordinate_lines(benchmark.lines.iter().map(|line| line.text.as_str()));
    let hypothesis = join_coordinate_lines(
        hypotheses
            .iter()
            .zip(selected)
            .filter_map(|(line, selected)| selected.then_some(line.text.as_str())),
    );
    Ok(BenchmarkResult {
        benchmark_id: benchmark.id.clone(),
        metrics: recognition_metrics(&reference, &hypothesis),
        missing_lines,
        source_identity,
        alignment: AlignmentMethod::SourceCoordinates,
        text_boundary_policy: TextBoundaryPolicy::CoordinateWhitespaceNormalized,
    })
}

fn join_coordinate_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    lines
        .into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_alto, evaluate_alto_with_identity, AlignmentMethod, GoldBenchmark, GoldLine,
        GoldLineSource, SourceBounds, SourceIdentity, SourceIdentityVerification,
        SourceImageGeometry, TextBoundaryPolicy,
    };
    use crate::alto::{AltoLine, AltoPage, AltoRegion};
    use crate::model::Point;

    #[test]
    fn evaluates_only_named_gold_lines() {
        let benchmark = GoldBenchmark {
            id: "fixture".to_owned(),
            edition: "edition".to_owned(),
            source_page: 1,
            source_sha256: "a".repeat(64),
            authority: "human checked".to_owned(),
            source_image: None,
            lines: vec![GoldLine {
                line_id: "line-1".to_owned(),
                text: "The name Aleph".to_owned(),
                source: None,
            }],
        };
        let page = AltoPage {
            width: 1,
            height: 1,
            regions: vec![AltoRegion {
                id: "region".to_owned(),
                polygon: Vec::new(),
                lines: vec![
                    AltoLine {
                        id: "line-1".to_owned(),
                        polygon: Vec::new(),
                        words: Vec::new(),
                        text: "Tue name Aleph".to_owned(),
                        confidence: 0.8,
                    },
                    AltoLine {
                        id: "not-gold".to_owned(),
                        polygon: Vec::new(),
                        words: Vec::new(),
                        text: "ignored".to_owned(),
                        confidence: 1.0,
                    },
                ],
            }],
        };

        let result = evaluate_alto(&benchmark, &page);
        assert!(result.metrics.cer > 0.0);
        assert!(result.missing_lines.is_empty());
        assert_eq!(result.metrics.reference_words, 3);
        assert_eq!(
            result.source_identity,
            SourceIdentityVerification::Unverified
        );
        assert_eq!(result.alignment, AlignmentMethod::LegacyLineIds);
    }

    #[test]
    fn coordinate_alignment_handles_split_merged_and_renumbered_lines_once() {
        let benchmark = coordinate_benchmark();
        let page = AltoPage {
            width: 100,
            height: 100,
            regions: vec![AltoRegion {
                id: "renumbered".to_owned(),
                polygon: rectangle(0.0, 0.0, 100.0, 100.0),
                lines: vec![
                    alto_line("split-a", "first", 0.0, 0.0, 50.0, 10.0),
                    alto_line("split-b", "line", 50.0, 0.0, 100.0, 10.0),
                    alto_line("merged", "second line", 0.0, 10.0, 100.0, 30.0),
                ],
            }],
        };
        let identity = fixture_identity();

        let result = evaluate_alto_with_identity(&benchmark, &page, Some(&identity)).unwrap();

        assert_eq!(result.metrics.cer, 0.0);
        assert!(result.missing_lines.is_empty());
        assert_eq!(result.source_identity, SourceIdentityVerification::Verified);
        assert_eq!(result.alignment, AlignmentMethod::SourceCoordinates);
        assert_eq!(
            result.text_boundary_policy,
            TextBoundaryPolicy::CoordinateWhitespaceNormalized
        );
    }

    #[test]
    fn coordinate_alignment_rejects_wrong_identity_or_image_dimensions() {
        let benchmark = coordinate_benchmark();
        let identity = SourceIdentity {
            source_page: 2,
            ..fixture_identity()
        };
        let page = AltoPage {
            width: 99,
            height: 100,
            regions: Vec::new(),
        };

        let identity_error = evaluate_alto_with_identity(&benchmark, &page, Some(&identity))
            .unwrap_err()
            .to_string();
        assert!(identity_error.contains("does not match"));
        let dimensions_error =
            evaluate_alto_with_identity(&benchmark, &page, Some(&fixture_identity()))
                .unwrap_err()
                .to_string();
        assert!(dimensions_error.contains("dimensions"));
    }

    #[test]
    fn coordinate_alignment_counts_a_true_merged_line_once() {
        let benchmark = coordinate_benchmark();
        let page = page_with_lines(vec![alto_line(
            "both-lines",
            "first line second line",
            0.0,
            0.0,
            100.0,
            20.0,
        )]);
        let result =
            evaluate_alto_with_identity(&benchmark, &page, Some(&fixture_identity())).unwrap();
        assert_eq!(result.metrics.cer, 0.0);
        assert_eq!(result.metrics.wer, 0.0);
        assert!(result.missing_lines.is_empty());
    }

    #[test]
    fn coordinate_alignment_scores_missing_and_extra_text() {
        let benchmark = coordinate_benchmark();
        let mut page = page_with_lines(vec![alto_line(
            "first-only",
            "first line",
            0.0,
            0.0,
            100.0,
            10.0,
        )]);
        let missing =
            evaluate_alto_with_identity(&benchmark, &page, Some(&fixture_identity())).unwrap();
        assert_eq!(missing.missing_lines, vec!["old-2"]);
        assert_eq!(missing.metrics.wer, 0.5);
        page.regions[0].lines = vec![alto_line(
            "extra-word",
            "first line unexpected second line",
            0.0,
            0.0,
            100.0,
            20.0,
        )];
        let extra =
            evaluate_alto_with_identity(&benchmark, &page, Some(&fixture_identity())).unwrap();
        assert!(extra.missing_lines.is_empty());
        assert_eq!(extra.metrics.wer, 0.25);
    }

    #[test]
    fn coordinate_alignment_rejects_unasserted_or_different_transform() {
        for coordinate_frame in [None, Some("same-size-deskewed-image".to_owned())] {
            let identity = SourceIdentity {
                coordinate_frame,
                ..fixture_identity()
            };
            let error = evaluate_alto_with_identity(
                &coordinate_benchmark(),
                &page_with_lines(Vec::new()),
                Some(&identity),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("coordinate frame"));
        }
    }

    fn page_with_lines(lines: Vec<AltoLine>) -> AltoPage {
        AltoPage {
            width: 100,
            height: 100,
            regions: vec![AltoRegion {
                id: "region".to_owned(),
                polygon: rectangle(0.0, 0.0, 100.0, 100.0),
                lines,
            }],
        }
    }

    #[test]
    fn coordinate_alignment_rejects_missing_identity_and_invalid_anchor() {
        let benchmark = coordinate_benchmark();
        let page = AltoPage {
            width: 100,
            height: 100,
            regions: Vec::new(),
        };
        let identity_error = evaluate_alto_with_identity(&benchmark, &page, None)
            .unwrap_err()
            .to_string();
        assert!(identity_error.contains("requires an asserted matching source identity"));

        let mut invalid = coordinate_benchmark();
        invalid.lines[0].source.as_mut().unwrap().bounds.x = f32::NAN;
        let anchor_error = evaluate_alto_with_identity(&invalid, &page, Some(&fixture_identity()))
            .unwrap_err()
            .to_string();
        assert!(anchor_error.contains("finite, non-degenerate"));
    }

    fn coordinate_benchmark() -> GoldBenchmark {
        GoldBenchmark {
            id: "fixture".to_owned(),
            edition: "edition".to_owned(),
            source_page: 1,
            source_sha256: "a".repeat(64),
            authority: "human checked".to_owned(),
            source_image: Some(SourceImageGeometry {
                width: 100,
                height: 100,
                coordinate_frame: "source-image-v1".to_owned(),
            }),
            lines: vec![
                GoldLine {
                    line_id: "old-1".to_owned(),
                    text: "first line".to_owned(),
                    source: Some(GoldLineSource {
                        source_page: 1,
                        bounds: bounds(0.0, 0.0, 100.0, 10.0),
                    }),
                },
                GoldLine {
                    line_id: "old-2".to_owned(),
                    text: "second line".to_owned(),
                    source: Some(GoldLineSource {
                        source_page: 1,
                        bounds: bounds(0.0, 10.0, 100.0, 20.0),
                    }),
                },
            ],
        }
    }

    fn fixture_identity() -> SourceIdentity {
        SourceIdentity {
            edition: "edition".to_owned(),
            source_page: 1,
            source_sha256: "a".repeat(64),
            coordinate_frame: Some("source-image-v1".to_owned()),
        }
    }

    fn alto_line(id: &str, text: &str, x1: f32, y1: f32, x2: f32, y2: f32) -> AltoLine {
        AltoLine {
            id: id.to_owned(),
            polygon: rectangle(x1, y1, x2, y2),
            words: Vec::new(),
            text: text.to_owned(),
            confidence: 1.0,
        }
    }

    fn rectangle(x1: f32, y1: f32, x2: f32, y2: f32) -> Vec<Point> {
        vec![
            Point { x: x1, y: y1 },
            Point { x: x2, y: y1 },
            Point { x: x2, y: y2 },
            Point { x: x1, y: y2 },
        ]
    }

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> SourceBounds {
        SourceBounds {
            x,
            y,
            width,
            height,
        }
    }
}
