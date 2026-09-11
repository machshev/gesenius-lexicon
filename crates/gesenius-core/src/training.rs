//! Ground-truth preparation, page-level splits, and Kraken fine-tuning.

use crate::metrics::{recognition_metrics, RecognitionMetrics};
use crate::model::{CorpusEntry, Point, ReviewState};
use crate::page_splits::{PageSplits, Partition};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use unicode_normalization::UnicodeNormalization;

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
    /// SHA-256 of the exact cropped PNG.
    pub crop_sha256: String,
    /// SHA-256 of the UTF-8 ground-truth file, including its final newline.
    pub ground_truth_sha256: String,
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
    /// Observed and missing code points for the requested Hebrew alphabet.
    pub alphabet_path: PathBuf,
}

/// Code-point coverage in the prepared ground truth. Hebrew marks are kept as
/// individual code points so a missing mark cannot be hidden by normalization.
#[derive(Debug, Clone, Serialize)]
pub struct AlphabetAudit {
    /// Frequency of each code point in prepared transcriptions.
    pub observed: BTreeMap<u32, usize>,
    /// Requested Hebrew marks/letters absent from prepared transcriptions.
    pub missing_hebrew: Vec<u32>,
}

fn hebrew_target_alphabet() -> impl Iterator<Item = u32> {
    (0x0591..=0x05c7).chain(0x05d0..=0x05ea)
}

/// Prepares reviewed line crops and `.gt.txt` files for Kraken.
pub fn prepare(
    entries: &[CorpusEntry],
    pilot: &PilotCatalogue,
    output_root: &Path,
    splits: &PageSplits,
    headwords_only: bool,
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
    let mut seen_samples = BTreeSet::new();
    let mut records = Vec::new();
    let mut benchmark = BTreeMap::<String, (String, String)>::new();
    let mut observed = BTreeMap::<u32, usize>::new();

    for entry in entries {
        if !selected.contains(&(entry.edition.clone(), entry.printed_page.clone())) {
            continue;
        }
        for (sample_kind, span) in entry.headword.iter().map(|span| ("headword", span)).chain(
            entry
                .blocks
                .iter()
                .flat_map(|block| block.spans.iter())
                .map(|span| ("line", span)),
        ) {
            if (headwords_only && sample_kind != "headword")
                || span.review_state == ReviewState::Machine
            {
                continue;
            }
            if entry.provenance.source_sha256 != splits.source_sha256 {
                bail!("entry source does not match split manifest");
            }
            let split = match splits.partition(&entry.edition, &entry.printed_page)? {
                Partition::Training => Split::Train,
                Partition::Validation => Split::Validation,
                Partition::FinalTest => continue,
                Partition::Development => continue,
            };

            let Some(coordinate) = span.coordinates.first() else {
                continue;
            };
            let sample_key = (
                entry.edition.clone(),
                coordinate.source_page,
                if sample_kind == "headword" {
                    span.id.clone()
                } else {
                    coordinate.line_id.clone()
                },
                sample_kind,
            );
            if !seen_samples.insert(sample_key) {
                continue;
            }
            if span.coordinates.len() != 1
                || coordinate.printed_page.as_deref() != Some(entry.printed_page.as_str())
            {
                bail!("training sample must belong to exactly one source page");
            }
            let name = safe_name(&format!(
                "{}-p{:04}-{}-{}",
                entry.edition,
                coordinate.source_page,
                if sample_kind == "headword" {
                    span.id.as_str()
                } else {
                    coordinate.line_id.as_str()
                },
                sample_kind
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
            fs::write(
                &ground_truth,
                format!("{}\n", span.diplomatic.nfc().collect::<String>()),
            )?;
            if split == Split::Train {
                for codepoint in span.diplomatic.nfc().map(u32::from) {
                    *observed.entry(codepoint).or_insert(0) += 1;
                }
            }
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
                image: image.clone(),
                ground_truth: ground_truth.clone(),
                source_span: format!("{}#{}", entry.id, span.id),
                crop_sha256: crate::headwords::digest(&fs::read(&image)?),
                ground_truth_sha256: crate::headwords::digest(&fs::read(&ground_truth)?),
            });
        }
    }
    if records.is_empty() {
        bail!("no corrected or verified pilot lines are available for training");
    }
    validate_training_records(&records, splits)?;
    records.sort_by(|left, right| left.image.cmp(&right.image));
    let manifest_path = output_root.join("ground-truth.jsonl");
    let mut manifest = String::new();
    for record in &records {
        manifest.push_str(&serde_json::to_string(record)?);
        manifest.push('\n');
    }
    fs::write(&manifest_path, manifest)?;
    fs::write(
        output_root.join("page-splits.toml"),
        toml::to_string_pretty(splits)?,
    )?;

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
    let missing_hebrew = hebrew_target_alphabet()
        .filter(|codepoint| !observed.contains_key(codepoint))
        .collect();
    let alphabet_path = output_root.join("alphabet-audit.json");
    fs::write(
        &alphabet_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&AlphabetAudit {
                observed,
                missing_hebrew,
            })?
        ),
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
        alphabet_path,
    })
}

/// Executes `ketos train` using prepared page-separated data.
pub fn execute_kraken_training(
    output_root: &Path,
    output_model: &Path,
    base_model: Option<&Path>,
    epochs: Option<usize>,
    seed: Option<u64>,
    deterministic: bool,
) -> Result<()> {
    let records = read_ground_truth(&output_root.join("ground-truth.jsonl"))?;
    validate_training_records(
        &records,
        &PageSplits::load(&output_root.join("page-splits.toml"))?,
    )?;
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
    let training_list = output_root.join("training-paths.txt");
    fs::write(
        &training_list,
        training
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )?;
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
    let mut command = kraken_training_command(
        output_model,
        base_model,
        epochs,
        seed,
        deterministic,
        &training_list,
        &validation_list,
    )?;
    let status = command
        .status()
        .context("failed to execute ketos; enter `nix develop`")?;
    if !status.success() {
        bail!("Kraken training failed with status {status}");
    }
    Ok(())
}

fn kraken_training_command(
    output_model: &Path,
    base_model: Option<&Path>,
    epochs: Option<usize>,
    seed: Option<u64>,
    deterministic: bool,
    training_list: &Path,
    validation_list: &Path,
) -> Result<Command> {
    if epochs == Some(0) {
        bail!("training epochs must be greater than zero");
    }
    let mut command = Command::new("ketos");
    if let Some(seed) = seed {
        command.arg("--seed").arg(seed.to_string());
    }
    if deterministic {
        command.arg("--deterministic");
    }
    command.arg("train").args(["--output"]).arg(output_model);
    if let Some(base_model) = base_model {
        command.arg("--load").arg(base_model);
    }
    if let Some(epochs) = epochs {
        command
            .args(["--quit", "fixed", "--epochs"])
            .arg(epochs.to_string());
    }
    command.args([
        "--resize",
        "union",
        "--normalization",
        "NFC",
        "--reorder",
        "--base-dir",
        "auto",
        "--format-type",
        "path",
        "--training-data",
    ]);
    command.arg(training_list);
    command.arg("--evaluation-data").arg(validation_list);
    Ok(command)
}

fn validate_training_records(records: &[GroundTruthRecord], splits: &PageSplits) -> Result<()> {
    let mut crops = BTreeSet::new();
    let mut pages = BTreeMap::new();
    for record in records {
        let expected = match splits.partition(&record.edition, &record.printed_page)? {
            Partition::Training => Split::Train,
            Partition::Validation => Split::Validation,
            Partition::FinalTest => Split::Test,
            Partition::Development => bail!("development samples cannot enter training manifests"),
        };
        if record.split != expected {
            bail!("training record violates frozen page split");
        }
        let key = (&record.edition, record.source_page);
        if pages
            .insert(key, record.split)
            .is_some_and(|previous| previous != record.split)
        {
            bail!("PDF page occurs in more than one partition");
        }
        if crate::headwords::digest(&fs::read(&record.image)?) != record.crop_sha256
            || crate::headwords::digest(&fs::read(&record.ground_truth)?)
                != record.ground_truth_sha256
        {
            bail!("stale image/text pair: {}", record.source_span);
        }
        if !crops.insert(&record.crop_sha256) {
            bail!("duplicated crop in training manifest");
        }
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
    use super::*;

    #[test]
    fn training_rejects_tampering_and_duplicate_crops_before_execution() {
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("crop.png");
        let ground_truth = temp.path().join("crop.gt.txt");
        fs::write(&image, b"test crop").unwrap();
        fs::write(&ground_truth, "אָב\n").unwrap();
        let splits = PageSplits::load(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../benchmarks/sample-inventory/robinson-1854-splits.toml"),
        )
        .unwrap();
        let record = GroundTruthRecord {
            edition: "robinson-1854".into(),
            printed_page: "11".into(),
            source_page: 27,
            line_id: "word".into(),
            split: Split::Train,
            image,
            ground_truth: ground_truth.clone(),
            source_span: "test".into(),
            crop_sha256: crate::headwords::digest(b"test crop"),
            ground_truth_sha256: crate::headwords::digest("אָב\n".as_bytes()),
        };
        validate_training_records(std::slice::from_ref(&record), &splits).unwrap();
        assert!(validate_training_records(&[record.clone(), record.clone()], &splits).is_err());
        let mut reserved = record.clone();
        reserved.printed_page = "175".into();
        assert!(validate_training_records(&[reserved], &splits).is_err());
        fs::write(ground_truth, "אב\n").unwrap();
        assert!(validate_training_records(&[record], &splits).is_err());
    }

    #[test]
    fn hebrew_target_alphabet_covers_marks_and_letters() {
        let alphabet: Vec<_> = hebrew_target_alphabet().collect();
        assert_eq!(alphabet.first(), Some(&0x0591));
        assert_eq!(alphabet.last(), Some(&0x05ea));
        assert!(alphabet.contains(&0x05b8));
        assert!(alphabet.contains(&0x05d0));
        assert_eq!(
            alphabet.len(),
            (0x05c7 - 0x0591 + 1) + (0x05ea - 0x05d0 + 1)
        );
    }

    #[test]
    fn kraken_training_command_places_reproducibility_options_before_subcommand() {
        let command = kraken_training_command(
            Path::new("checkpoints"),
            None,
            None,
            Some(42),
            true,
            Path::new("train.txt"),
            Path::new("validation.txt"),
        )
        .unwrap();
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            &arguments[..4],
            &["--seed", "42", "--deterministic", "train"]
        );
    }
}
