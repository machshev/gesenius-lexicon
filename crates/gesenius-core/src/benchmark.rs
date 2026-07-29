//! Immutable OCR gold fixtures and reproducible recognition evaluation.

use crate::alto::AltoPage;
use crate::metrics::{recognition_metrics, RecognitionMetrics};
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
        for line in &self.lines {
            if line.line_id.trim().is_empty() || line.text.trim().is_empty() {
                bail!("benchmark line IDs and text must not be empty");
            }
            if seen.insert(&line.line_id, ()).is_some() {
                bail!("benchmark repeats line ID `{}`", line.line_id);
            }
        }
        Ok(())
    }
}

/// One exact diplomatic line transcription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldLine {
    /// Stable ALTO line identifier.
    pub line_id: String,
    /// Exact Unicode transcription in logical order.
    pub text: String,
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
}

/// Evaluates only explicitly gold-transcribed lines, ignoring all other OCR.
#[must_use]
pub fn evaluate_alto(benchmark: &GoldBenchmark, page: &AltoPage) -> BenchmarkResult {
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
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate_alto, GoldBenchmark, GoldLine};
    use crate::alto::{AltoLine, AltoPage, AltoRegion};

    #[test]
    fn evaluates_only_named_gold_lines() {
        let benchmark = GoldBenchmark {
            id: "fixture".to_owned(),
            edition: "edition".to_owned(),
            source_page: 1,
            source_sha256: "a".repeat(64),
            authority: "human checked".to_owned(),
            lines: vec![GoldLine {
                line_id: "line-1".to_owned(),
                text: "The name Aleph".to_owned(),
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
    }
}
