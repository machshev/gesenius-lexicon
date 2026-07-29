//! Ground-truth preparation, page-level splits, and Kraken fine-tuning.

use crate::metrics::{recognition_metrics, RecognitionMetrics};
use crate::model::{CorpusEntry, Point, ReviewState};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixed pilot specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PilotCatalogue {
    /// Format version.
    pub pilot_version: u32,
    /// Per-edition pages.
    pub editions: Vec<PilotEdition>,
}

impl PilotCatalogue {
    /// Loads a fixed pilot and enforces exactly 24 distinct pages per edition.
    pub fn load(path: &Path) -> Result<Self> {
        let input = fs::read_to_string(path)
            .with_context(|| format!("failed to read pilot {}", path.display()))?;
        let pilot: Self =
            toml::from_str(&input).with_context(|| format!("invalid pilot {}", path.display()))?;
        if pilot.pilot_version != 1 {
            bail!("unsupported pilot version {}", pilot.pilot_version);
        }
        for edition in &pilot.editions {
            if edition.pages.len() != 24 {
                bail!(
                    "pilot edition `{}` has {} pages; exactly 24 are required",
                    edition.edition,
                    edition.pages.len()
                );
            }
            let distinct: BTreeSet<_> = edition
                .pages
                .iter()
                .map(|page| &page.printed_page)
                .collect();
            if distinct.len() != 24 {
                bail!("pilot edition `{}` repeats a printed page", edition.edition);
            }
        }
        Ok(pilot)
    }
}

/// Pilot selection for one edition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PilotEdition {
    /// Registered edition.
    pub edition: String,
    /// Exactly 24 representative pages.
    pub pages: Vec<PilotPage>,
}

/// One representative page and its selection rationale.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PilotPage {
    /// Printed page label, stable across scans.
    pub printed_page: String,
    /// Selection class such as `early`, `damaged`, or `index`.
    pub category: String,
    /// Scripts expected to be meaningfully represented.
    pub scripts: Vec<String>,
    /// Human-readable selection rationale.
    pub note: String,
}

/// Page-level train, validation, or test partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    /// Model fitting.
    Train,
    /// Epoch/model selection.
    Validation,
    /// Held-out final reporting only.
    Test,
}

impl Split {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Validation => "validation",
            Self::Test => "test",
        }
    }
}

/// One generated line-level ground-truth record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundTruthRecord {
    /// Edition.
    pub edition: String,
    /// Printed page.
    pub printed_page: String,
    /// PDF page.
    pub source_page: u32,
    /// ALTO line identifier.
    pub line_id: String,
    /// Page-level partition.
    pub split: Split,
    /// Cropped image.
    pub image: PathBuf,
    /// UTF-8 Kraken ground truth.
    pub ground_truth: PathBuf,
    /// Entry and span that supplied the correction.
    pub source_span: String,
}

/// Training preparation output.
#[derive(Debug, Clone, Serialize)]
pub struct TrainingResult {
    /// Created ground-truth lines.
    pub lines: usize,
    /// Counts by page-level partition.
    pub split_counts: BTreeMap<String, usize>,
    /// JSONL ground-truth manifest.
    pub manifest_path: PathBuf,
    /// Baseline benchmark metrics keyed by OCR engine.
    pub metrics_path: PathBuf,
}

/// Prepares reviewed line crops and `.gt.txt` files for Kraken.
pub fn prepare(
    entries: &[CorpusEntry],
    pilot: &PilotCatalogue,
    output_root: &Path,
) -> Result<TrainingResult> {
    fs::create_dir_all(output_root)?;
    let selected: BTreeSet<_> = pilot
        .editions
        .iter()
        .flat_map(|edition| {
            edition
                .pages
                .iter()
                .map(move |page| (edition.edition.clone(), page.printed_page.clone()))
        })
        .collect();
    let mut seen_lines = BTreeSet::new();
    let mut records = Vec::new();
    let mut benchmark = BTreeMap::<String, (String, String)>::new();

    for entry in entries {
        if !selected.contains(&(entry.edition.clone(), entry.printed_page.clone())) {
            continue;
        }
        for span in entry.blocks.iter().flat_map(|block| block.spans.iter()) {
            if span.review_state == ReviewState::Machine {
                continue;
            }
            let Some(coordinate) = span.coordinates.first() else {
                continue;
            };
            let line_key = (
                entry.edition.clone(),
                coordinate.source_page,
                coordinate.line_id.clone(),
            );
            if !seen_lines.insert(line_key) {
                continue;
            }
            let split = page_split(&entry.edition, coordinate.source_page);
            let name = safe_name(&format!(
                "{}-p{:04}-{}",
                entry.edition, coordinate.source_page, coordinate.line_id
            ));
            let directory = output_root.join(split.as_str());
            fs::create_dir_all(&directory)?;
            let image = directory.join(format!("{name}.png"));
            let ground_truth = directory.join(format!("{name}.gt.txt"));
            crop_line(
                Path::new(&coordinate.page_image),
                &image,
                &coordinate.polygon,
            )?;
            fs::write(&ground_truth, format!("{}\n", span.diplomatic))?;
            for hypothesis in &span.hypotheses {
                let pair = benchmark
                    .entry(hypothesis.engine.clone())
                    .or_insert_with(|| (String::new(), String::new()));
                pair.0.push_str(&span.diplomatic);
                pair.0.push('\n');
                pair.1.push_str(&hypothesis.text);
                pair.1.push('\n');
            }
            records.push(GroundTruthRecord {
                edition: entry.edition.clone(),
                printed_page: entry.printed_page.clone(),
                source_page: coordinate.source_page,
                line_id: coordinate.line_id.clone(),
                split,
                image,
                ground_truth,
                source_span: format!("{}#{}", entry.id, span.id),
            });
        }
    }
    if records.is_empty() {
        bail!("no corrected or verified pilot lines are available for training");
    }
    records.sort_by(|left, right| left.image.cmp(&right.image));
    let manifest_path = output_root.join("ground-truth.jsonl");
    let mut manifest = String::new();
    for record in &records {
        manifest.push_str(&serde_json::to_string(record)?);
        manifest.push('\n');
    }
    fs::write(&manifest_path, manifest)?;

    let benchmark_metrics: BTreeMap<String, RecognitionMetrics> = benchmark
        .into_iter()
        .map(|(engine, (reference, hypothesis))| {
            (engine, recognition_metrics(&reference, &hypothesis))
        })
        .collect();
    let metrics_path = output_root.join("baseline-metrics.json");
    fs::write(
        &metrics_path,
        format!("{}\n", serde_json::to_string_pretty(&benchmark_metrics)?),
    )?;
    let mut split_counts = BTreeMap::new();
    for record in &records {
        *split_counts
            .entry(record.split.as_str().to_owned())
            .or_insert(0) += 1;
    }
    Ok(TrainingResult {
        lines: records.len(),
        split_counts,
        manifest_path,
        metrics_path,
    })
}

/// Executes `ketos train` using prepared page-separated data.
pub fn execute_kraken_training(
    output_root: &Path,
    output_model: &Path,
    base_model: Option<&Path>,
) -> Result<()> {
    let records = read_ground_truth(&output_root.join("ground-truth.jsonl"))?;
    let training: Vec<_> = records
        .iter()
        .filter(|record| record.split == Split::Train)
        .map(|record| record.image.as_os_str())
        .collect();
    let validation: Vec<_> = records
        .iter()
        .filter(|record| record.split == Split::Validation)
        .map(|record| record.image.as_os_str())
        .collect();
    if training.is_empty() || validation.is_empty() {
        bail!("training and validation partitions must both contain reviewed lines");
    }
    let validation_list = output_root.join("validation-paths.txt");
    fs::write(
        &validation_list,
        validation
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )?;
    let mut command = Command::new("ketos");
    command.arg("train").args(["--output"]).arg(output_model);
    if let Some(base_model) = base_model {
        command.arg("--load").arg(base_model);
    }
    command.args([
        "--resize",
        "add",
        "--normalization",
        "NFC",
        "--no-reorder",
        "--format-type",
        "path",
    ]);
    command.args(&training);
    command.arg("--evaluation-data").arg(validation_list);
    let output = command
        .output()
        .context("failed to execute ketos; enter `nix develop`")?;
    if !output.status.success() {
        bail!(
            "Kraken training failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn read_ground_truth(path: &Path) -> Result<Vec<GroundTruthRecord>> {
    fs::read_to_string(path)?
        .lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("invalid ground-truth line {}", index + 1))
        })
        .collect()
}

fn crop_line(source: &Path, destination: &Path, polygon: &[Point]) -> Result<()> {
    let (x, y, width, height) = polygon_bounds(polygon).context("empty line polygon")?;
    let status = Command::new("magick")
        .arg(source)
        .args(["-crop", &format!("{width}x{height}+{x}+{y}"), "+repage"])
        .arg(destination)
        .status()
        .context("failed to execute magick; enter `nix develop`")?;
    if !status.success() {
        bail!("failed to crop training line from {}", source.display());
    }
    Ok(())
}

fn polygon_bounds(points: &[Point]) -> Option<(u32, u32, u32, u32)> {
    let first = points.first()?;
    let (min_x, min_y, max_x, max_y) = points.iter().skip(1).fold(
        (first.x, first.y, first.x, first.y),
        |(min_x, min_y, max_x, max_y), point| {
            (
                min_x.min(point.x),
                min_y.min(point.y),
                max_x.max(point.x),
                max_y.max(point.y),
            )
        },
    );
    Some((
        min_x.floor().max(0.0) as u32,
        min_y.floor().max(0.0) as u32,
        (max_x - min_x).ceil().max(1.0) as u32,
        (max_y - min_y).ceil().max(1.0) as u32,
    ))
}

fn page_split(edition: &str, source_page: u32) -> Split {
    let mut hasher = Sha256::new();
    hasher.update(edition.as_bytes());
    hasher.update(source_page.to_le_bytes());
    match hasher.finalize()[0] % 10 {
        0..=6 => Split::Train,
        7..=8 => Split::Validation,
        _ => Split::Test,
    }
}

fn safe_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{page_split, Split};

    #[test]
    fn page_split_is_deterministic_and_page_level() {
        assert_eq!(
            page_split("robinson-1854", 17),
            page_split("robinson-1854", 17)
        );
        assert!(matches!(
            page_split("robinson-1854", 17),
            Split::Train | Split::Validation | Split::Test
        ));
    }
}
