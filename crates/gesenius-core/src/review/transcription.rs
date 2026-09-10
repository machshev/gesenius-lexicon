//! Source-checked draft review, separate from corpus correction patches.

use super::{content_type_header, respond_error, respond_html, respond_json, security_header};
use crate::benchmark::GoldBenchmark;
use crate::page_splits::{PageSplits, Partition};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::path::{Path, PathBuf};
use tiny_http::{Method, Request, Response, StatusCode};
use unicode_normalization::UnicodeNormalization;

#[derive(Deserialize)]
struct Manifest {
    partition: String,
    #[serde(default)]
    kind: Option<String>,
    printed_page: String,
    lines: Vec<Crop>,
    unresolved: Vec<Uncertainty>,
}

#[derive(Deserialize)]
struct Crop {
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    context_sha256: Option<String>,
    line_id: String,
    crop: String,
    crop_sha256: String,
    #[serde(default)]
    rectangle: Option<[u32; 4]>,
    #[serde(default)]
    crop_commands: Vec<Vec<String>>,
}

#[derive(Deserialize)]
struct Uncertainty {
    line_id: String,
    detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum State {
    Reading,
    Resolved,
    Unresolved,
    NotHeadword,
    Excluded,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReviewMethod {
    // Records written before draft-prefilled review used the source-first workflow.
    #[default]
    LegacySourceFirst,
    DraftAssisted,
    CropCorrectedFromReviewedContext,
    SourceManifestMigration,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Direction {
    Ltr,
    Rtl,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TextRun {
    language: String,
    direction: Direction,
    text: String,
}

fn validate_runs(runs: &[TextRun], text: &str) -> Result<()> {
    if runs.is_empty() || runs.len() > 256 {
        bail!("a transcription must have between 1 and 256 text runs");
    }
    for run in runs {
        if ![
            "en", "he", "arc", "ar", "fa", "syc", "grc", "la", "gez", "phn", "und",
        ]
        .contains(&run.language.as_str())
        {
            bail!("unsupported text-run language");
        }
        if run.text.chars().any(|c| matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
            bail!("remove hidden bidi controls; use text-run direction instead");
        }
    }
    if runs.iter().map(|run| run.text.as_str()).collect::<String>() != text {
        bail!("text runs must concatenate exactly to the transcription");
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Record {
    sample: String,
    line_id: String,
    source_digest: String,
    revision: u64,
    reviewer: String,
    #[serde(default)]
    independent_reading: Option<String>,
    #[serde(default)]
    review_method: ReviewMethod,
    #[serde(default)]
    displayed_draft: Option<String>,
    text: String,
    #[serde(default)]
    runs: Option<Vec<TextRun>>,
    state: State,
    comment: String,
    reviewed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supersedes_source_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reviewed_context_sha256: Option<String>,
}

#[derive(Deserialize)]
struct Update {
    sample: String,
    line_id: String,
    source_digest: String,
    base_revision: u64,
    reviewer: String,
    text: String,
    #[serde(default)]
    runs: Option<Vec<TextRun>>,
    state: State,
    comment: String,
}

#[derive(Deserialize)]
struct CropUpdate {
    sample: String,
    line_id: String,
    source_digest: String,
    rectangle: [u32; 4],
}

#[derive(Debug, Serialize)]
struct Line {
    context: Option<String>,
    edition: String,
    source_sha256: String,
    crop_sha256: String,
    kind: String,
    sample: String,
    line_id: String,
    printed_page: String,
    source_page: u32,
    partition: String,
    source_digest: String,
    crop: String,
    source_image: Option<String>,
    rectangle: Option<[u32; 4]>,
    // Draft and source-check notes are visible from the first visit.
    draft: Option<String>,
    uncertainties: Vec<String>,
    review: Option<Record>,
}

pub(super) struct TranscriptionStore {
    pub root: PathBuf,
    pub journal: PathBuf,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl TranscriptionStore {
    fn records(&self) -> Result<Vec<Record>> {
        if !self.journal.exists() {
            return Ok(Vec::new());
        }
        BufReader::new(File::open(&self.journal)?)
            .lines()
            .filter_map(|line| match line {
                Ok(line) if line.trim().is_empty() => None,
                line => Some(line),
            })
            .map(|line| Ok(serde_json::from_str(&line?)?))
            .collect()
    }

    fn lines(&self) -> Result<Vec<Line>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let records = self.records()?;
        let root = self.root.canonicalize()?;
        let mut directories: Vec<_> = fs::read_dir(&root)?
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.join("draft.json").is_file())
            .collect();
        directories.sort();
        let mut result = Vec::new();
        for directory in directories {
            let directory = directory.canonicalize()?;
            if !directory.starts_with(&root) {
                bail!("sample directory escapes transcription root");
            }
            let sample = directory
                .file_name()
                .context("missing sample name")?
                .to_string_lossy()
                .into_owned();
            let draft_path = directory.join("draft.json");
            let benchmark = GoldBenchmark::load(&draft_path)?;
            let manifest_bytes = fs::read(directory.join("review.json"))?;
            let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
            if !matches!(
                manifest.partition.as_str(),
                "training" | "development" | "validation"
            ) {
                continue;
            }
            let mut identity = fs::read(&draft_path)?;
            identity.extend_from_slice(&manifest_bytes);
            let source_digest = digest(&identity);
            for gold in benchmark.lines {
                let crop = manifest
                    .lines
                    .iter()
                    .find(|crop| crop.line_id == gold.line_id)
                    .context("missing crop for draft line")?;
                let crop_path = directory.join(&crop.crop).canonicalize()?;
                if !crop_path.starts_with(&directory)
                    || crop_path.extension().is_none_or(|ext| ext != "png")
                {
                    bail!("crop must be a PNG within its sample directory");
                }
                if digest(&fs::read(&crop_path)?) != crop.crop_sha256 {
                    bail!("crop hash mismatch for {}", gold.line_id);
                }
                let context = if let Some(context) = &crop.context {
                    let path = directory.join(context).canonicalize()?;
                    if !path.starts_with(&directory)
                        || path.extension().is_none_or(|e| e != "png")
                        || crop.context_sha256.as_deref()
                            != Some(digest(&fs::read(&path)?).as_str())
                    {
                        bail!("invalid or stale context crop");
                    }
                    Some(path.to_string_lossy().into_owned())
                } else {
                    None
                };
                let review = records
                    .iter()
                    .rev()
                    .find(|record| {
                        record.sample == sample
                            && record.line_id == gold.line_id
                            && record.source_digest == source_digest
                    })
                    .cloned();
                result.push(Line {
                    context,
                    edition: benchmark.edition.clone(),
                    source_sha256: benchmark.source_sha256.clone(),
                    crop_sha256: crop.crop_sha256.clone(),
                    kind: manifest.kind.clone().unwrap_or_else(|| "line".to_owned()),
                    sample: sample.clone(),
                    line_id: gold.line_id.clone(),
                    printed_page: manifest.printed_page.clone(),
                    source_page: benchmark.source_page,
                    partition: manifest.partition.clone(),
                    source_digest: source_digest.clone(),
                    crop: crop_path.to_string_lossy().into_owned(),
                    source_image: crop
                        .crop_commands
                        .first()
                        .and_then(|command| command.get(1))
                        .and_then(|path| fs::canonicalize(path).ok())
                        .map(|path| path.to_string_lossy().into_owned()),
                    rectangle: crop.rectangle,
                    draft: Some(gold.text),
                    uncertainties: manifest
                        .unresolved
                        .iter()
                        .filter(|item| item.line_id == gold.line_id)
                        .map(|item| item.detail.clone())
                        .collect(),
                    review,
                });
            }
        }
        Ok(result)
    }

    fn apply(&self, update: Update) -> Result<Record> {
        if update.reviewer.trim().is_empty()
            || (!matches!(update.state, State::NotHeadword | State::Excluded)
                && update.text.trim().is_empty())
        {
            bail!("reviewer is required, and transcription is required for a headword");
        }
        if update.text.chars().any(|c| matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
            bail!("remove hidden bidi controls; use text-run direction instead");
        }
        if let Some(runs) = &update.runs {
            validate_runs(runs, &update.text)?;
        }
        if update.state == State::Reading {
            bail!("the draft is now visible; reload and save a resolved or unresolved review");
        }
        if matches!(
            update.state,
            State::Unresolved | State::NotHeadword | State::Excluded
        ) && update.comment.trim().is_empty()
        {
            bail!("describe the uncertainty before saving an unresolved line");
        }
        if let Some(parent) = self.journal.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.journal)?;
        file.lock_exclusive()?;
        let result = (|| {
            let lines = self.lines()?;
            let line = lines
                .iter()
                .find(|line| line.sample == update.sample && line.line_id == update.line_id)
                .context("unknown transcription line")?;
            let current_revision = line.review.as_ref().map_or(0, |record| record.revision);
            if line.source_digest != update.source_digest
                || current_revision != update.base_revision
            {
                bail!("revision conflict: source or review changed; reload before saving");
            }
            if line.kind != "headword"
                && line
                    .review
                    .as_ref()
                    .is_some_and(|previous| previous.reviewer != update.reviewer.trim())
            {
                bail!("continue with the reviewer who started this line review");
            }
            let record = Record {
                sample: update.sample,
                line_id: update.line_id,
                source_digest: update.source_digest,
                revision: current_revision + 1,
                reviewer: update.reviewer.trim().to_owned(),
                independent_reading: line
                    .review
                    .as_ref()
                    .filter(|record| record.reviewer == update.reviewer.trim())
                    .and_then(|record| record.independent_reading.clone()),
                review_method: ReviewMethod::DraftAssisted,
                displayed_draft: line.draft.clone(),
                text: update.text,
                runs: update.runs,
                state: update.state,
                comment: update.comment,
                reviewed_at: Utc::now(),
                supersedes_source_digest: None,
                reviewed_context_sha256: None,
            };
            serde_json::to_writer(&mut file, &record)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(record)
        })();
        let _ = FileExt::unlock(&file);
        result
    }

    fn adjust_crop(&self, update: CropUpdate) -> Result<()> {
        let [x, y, width, height] = update.rectangle;
        if width < 2 || height < 2 || width > 5000 || height > 5000 {
            bail!("crop dimensions are invalid");
        }
        let current = self
            .lines()?
            .into_iter()
            .find(|line| line.sample == update.sample && line.line_id == update.line_id)
            .context("unknown transcription line")?;
        if current.kind != "headword" || current.source_digest != update.source_digest {
            bail!("revision conflict: source or review changed; reload before editing the crop");
        }
        let sample = self.root.canonicalize()?.join(&update.sample);
        if !sample.starts_with(self.root.canonicalize()?) {
            bail!("sample escapes transcription root");
        }
        let review_path = sample.join("review.json");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&review_path)?;
        file.lock_exclusive()?;
        let result = (|| {
            let mut manifest_bytes = Vec::new();
            file.read_to_end(&mut manifest_bytes)?;
            let mut identity = fs::read(sample.join("draft.json"))?;
            identity.extend_from_slice(&manifest_bytes);
            if digest(&identity) != update.source_digest {
                bail!(
                    "revision conflict: source or review changed; reload before editing the crop"
                );
            }
            let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
            let line = manifest["lines"]
                .as_array_mut()
                .context("review lines must be an array")?
                .iter_mut()
                .find(|line| line["line_id"] == update.line_id)
                .context("crop line is missing from review manifest")?;
            let commands = line["crop_commands"]
                .as_array()
                .context("crop commands are required for source-based editing")?
                .clone();
            let processed = commands
                .first()
                .and_then(|command| command.get(1))
                .and_then(|path| path.as_str())
                .context("processed source command is missing")?;
            let original = commands
                .get(2)
                .and_then(|command| command.get(1))
                .and_then(|path| path.as_str());
            let processed =
                fs::canonicalize(processed).context("processed source raster is unavailable")?;
            let original = original
                .map(fs::canonicalize)
                .transpose()
                .context("original source raster is unavailable")?;
            let suffix = format!("{}-{}x{}+{}+{}", update.line_id, width, height, x, y);
            let crop_relative = format!("crops/{suffix}.png");
            let original_relative = format!("crops/{suffix}-original.png");
            let crop_output = sample.join(&crop_relative);
            let original_output = sample.join(&original_relative);
            run_crop(&processed, &crop_output, update.rectangle)?;
            if let Some(original) = &original {
                if let Err(error) = run_crop(original, &original_output, update.rectangle) {
                    let _ = fs::remove_file(&crop_output);
                    return Err(error);
                }
            }
            line["crop"] = crop_relative.clone().into();
            line["crop_sha256"] = digest(&fs::read(&crop_output)?).into();
            line["rectangle"] = serde_json::to_value(update.rectangle)?;
            if original.is_some() {
                line["original_crop"] = original_relative.clone().into();
                line["original_crop_sha256"] = digest(&fs::read(&original_output)?).into();
            }
            let geometry = format!("{width}x{height}+{x}+{y}");
            let mut new_commands = commands;
            new_commands[0] = serde_json::json!([
                "magick",
                processed,
                "-crop",
                geometry,
                "+repage",
                sample.join(&crop_relative)
            ]);
            if let (Some(source), Some(command)) = (&original, new_commands.get_mut(2)) {
                *command = serde_json::json!([
                    "magick",
                    source,
                    "-crop",
                    geometry,
                    "+repage",
                    sample.join(&original_relative)
                ]);
            }
            line["crop_commands"] = new_commands.into();
            let bytes = serde_json::to_vec_pretty(&manifest)?;
            file.set_len(0)?;
            file.rewind()?;
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(())
        })();
        let _ = FileExt::unlock(&file);
        result
    }
}

fn run_crop(source: &Path, output: &Path, [x, y, width, height]: [u32; 4]) -> Result<()> {
    let geometry = format!("{width}x{height}+{x}+{y}");
    let status = std::process::Command::new("magick")
        .arg(source)
        .arg("-crop")
        .arg(geometry)
        .arg("+repage")
        .arg(output)
        .status()
        .context("failed to start ImageMagick")?;
    if !status.success() {
        bail!("ImageMagick failed to create the adjusted crop");
    }
    Ok(())
}

pub(super) fn handle(mut request: Request, store: &TranscriptionStore) -> Result<()> {
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
    if request.method() == &Method::Get && path == "/transcription-fonts/OFL.txt" {
        request.respond(
            Response::from_string(include_str!("fonts/OFL.txt"))
                .with_header(content_type_header("text/plain; charset=utf-8"))
                .with_header(security_header()),
        )?;
        return Ok(());
    }
    let font: Option<&[u8]> = match path.as_str() {
        "/transcription-fonts/estrangela.ttf" => Some(include_bytes!("fonts/estrangela.ttf")),
        "/transcription-fonts/serto.ttf" => Some(include_bytes!("fonts/serto.ttf")),
        "/transcription-fonts/eastern.ttf" => Some(include_bytes!("fonts/eastern.ttf")),
        _ => None,
    };
    if let (true, Some(bytes)) = (request.method() == &Method::Get, font) {
        request.respond(
            Response::from_data(bytes)
                .with_header(content_type_header("font/ttf"))
                .with_header(security_header()),
        )?;
        return Ok(());
    }
    if request.method() == &Method::Get
        && matches!(
            path.as_str(),
            "/transcription-keyboard.js" | "/transcription-runs.js"
        )
    {
        request.respond(
            Response::from_string(if path == "/transcription-runs.js" {
                include_str!("transcription-runs.js")
            } else {
                include_str!("transcription-keyboard.js")
            })
            .with_header(content_type_header("text/javascript; charset=utf-8"))
            .with_header(security_header()),
        )?;
        return Ok(());
    }
    if request.method() == &Method::Get && path == "/transcriptions" {
        return respond_html(request, include_str!("transcription.html"));
    }
    if request.method() == &Method::Get && path == "/api/transcriptions" {
        return match store.lines() {
            Ok(lines) => respond_json(request, StatusCode(200), &lines),
            Err(error) => respond_error(request, StatusCode(422), &format!("{error:#}")),
        };
    }
    if request.method() == &Method::Post && path == "/api/transcriptions" {
        let result = (|| {
            let mut body = String::new();
            request
                .as_reader()
                .take(128 * 1024 + 1)
                .read_to_string(&mut body)?;
            if body.len() > 128 * 1024 {
                bail!("review request too large");
            }
            store.apply(serde_json::from_str(&body).context("invalid transcription review JSON")?)
        })();
        return match result {
            Ok(record) => respond_json(request, StatusCode(200), &record),
            Err(error) => respond_error(
                request,
                StatusCode(if error.to_string().contains("revision conflict") {
                    409
                } else {
                    422
                }),
                &format!("{error:#}"),
            ),
        };
    }
    if request.method() == &Method::Post && path == "/api/transcription-crop" {
        let result = (|| {
            let mut body = String::new();
            request
                .as_reader()
                .take(64 * 1024 + 1)
                .read_to_string(&mut body)?;
            if body.len() > 64 * 1024 {
                bail!("crop request too large");
            }
            store.adjust_crop(serde_json::from_str(&body).context("invalid crop JSON")?)
        })();
        return match result {
            Ok(()) => respond_json(
                request,
                StatusCode(200),
                &serde_json::json!({"updated": true}),
            ),
            Err(error) => respond_error(request, StatusCode(422), &format!("{error:#}")),
        };
    }
    respond_error(request, StatusCode(404), "not found")
}

/// Deliberately promotes resolved headword reviews into auditable Kraken pairs.
/// Development and final-test samples never enter fitting manifests.
pub fn export_headwords(
    root: &Path,
    journal: &Path,
    splits_path: &Path,
    output: &Path,
) -> Result<usize> {
    let splits = PageSplits::load(splits_path)?;
    let store = TranscriptionStore {
        root: root.into(),
        journal: journal.into(),
    };
    // Keep one journal snapshot for the whole export.
    let lock = OpenOptions::new().read(true).open(journal)?;
    lock.lock_shared()?;
    let records = store.records()?;
    let lines = store.lines()?;
    let mut selected = Vec::new();
    let mut crops = std::collections::BTreeSet::new();
    for line in lines.iter().filter(|l| l.kind == "headword") {
        let partition = splits.partition(&line.edition, &line.printed_page)?;
        let split = match partition {
            Partition::Training => "train",
            Partition::Validation => "validation",
            Partition::Development | Partition::FinalTest => continue,
        };
        if line.partition
            != (if split == "train" {
                "training"
            } else {
                "validation"
            })
            || line.source_sha256 != splits.source_sha256
        {
            bail!("review source/partition mismatch");
        }
        let review = records
            .iter()
            .rev()
            .find(|r| r.sample == line.sample && r.line_id == line.line_id)
            .context("headword has no human review")?;
        if review.source_digest == line.source_digest
            && matches!(review.state, State::NotHeadword | State::Excluded)
        {
            continue;
        }
        if review.source_digest != line.source_digest
            || review.state != State::Resolved
            || review.reviewer.trim().is_empty()
            || review.text.trim().is_empty()
            || review.revision == 0
        {
            bail!(
                "stale or unresolved headword review: {}/{}",
                line.sample,
                line.line_id
            );
        }
        if review.text.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
            bail!("headword contains controls");
        }
        let mut reviewers = std::collections::BTreeSet::new();
        let second = records
            .iter()
            .rev()
            .filter(|r| {
                r.sample == line.sample
                    && r.line_id == line.line_id
                    && r.source_digest == review.source_digest
                    && reviewers.insert(&r.reviewer)
            })
            .find(|r| {
                r.state == State::Resolved
                    && r.reviewer != review.reviewer
                    && !r.reviewer.trim().is_empty()
                    && r.text.nfc().eq(review.text.nfc())
            });
        if !crops.insert(&line.crop_sha256) {
            bail!("duplicate headword crop, including across splits");
        }
        selected.push((line, review, second, split));
    }
    if selected.is_empty() {
        bail!("no resolved training/validation headword reviews; collect human labels first");
    }
    if output.exists() {
        bail!("export destination already exists; use a new versioned directory");
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = tempfile::tempdir_in(parent)?;
    let final_root = parent.canonicalize()?.join(
        output
            .file_name()
            .context("output needs a directory name")?,
    );
    let mut manifest = String::new();
    let mut alphabet =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, usize>>::new();
    for (index, (line, review, second, split)) in selected.iter().enumerate() {
        let name = format!("{split}/headword-{index:06}");
        fs::create_dir_all(temporary.path().join(split))?;
        let bytes = fs::read(&line.crop)?;
        if digest(&bytes) != line.crop_sha256 {
            bail!("crop changed during export");
        }
        let nfc: String = review.text.nfc().collect();
        fs::write(temporary.path().join(format!("{name}.png")), bytes)?;
        fs::write(
            temporary.path().join(format!("{name}.gt.txt")),
            format!("{nfc}\n"),
        )?;
        for c in nfc.chars() {
            *alphabet
                .entry((*split).into())
                .or_default()
                .entry(format!("U+{:04X}", c as u32))
                .or_default() += 1;
        }
        let value = serde_json::json!({
            "edition": line.edition, "printed_page": line.printed_page, "source_page": line.source_page,
            "line_id": line.line_id, "split": split, "sample_kind": "headword",
            "image": final_root.join(format!("{name}.png")),
            "ground_truth": final_root.join(format!("{name}.gt.txt")),
            "source_span": format!("{}#{}", line.sample, line.line_id),
            "source_sha256": line.source_sha256, "crop_sha256": line.crop_sha256,
            "ground_truth_sha256": digest(format!("{nfc}\n").as_bytes()),
            "diplomatic": review.text, "nfc": nfc, "review": review,
            "second_review": second, "split_sha256": digest(&fs::read(splits_path)?),
            "normalization": "NFC; logical order; no bidi controls"
        });
        manifest.push_str(&serde_json::to_string(&value)?);
        manifest.push('\n');
    }
    fs::write(temporary.path().join("ground-truth.jsonl"), manifest)?;
    fs::write(
        temporary.path().join("alphabet-audit.json"),
        serde_json::to_vec_pretty(&alphabet)?,
    )?;
    fs::copy(splits_path, temporary.path().join("page-splits.toml"))?;
    fs::rename(temporary.path(), &final_root)?;
    Ok(selected.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, TranscriptionStore) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("drafts");
        let sample = root.join("sample");
        fs::create_dir_all(sample.join("crops")).unwrap();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/transcription-drafts/robinson-1854-p050");
        for name in ["draft.json", "review.json"] {
            fs::copy(source.join(name), sample.join(name)).unwrap();
        }
        for crop in fs::read_dir(source.join("crops")).unwrap() {
            let crop = crop.unwrap();
            fs::copy(crop.path(), sample.join("crops").join(crop.file_name())).unwrap();
        }
        let store = TranscriptionStore {
            root,
            journal: temp.path().join("reviews.jsonl"),
        };
        (temp, store)
    }

    fn update(line: &Line, state: State) -> Update {
        Update {
            sample: line.sample.clone(),
            line_id: line.line_id.clone(),
            source_digest: line.source_digest.clone(),
            base_revision: line.review.as_ref().map_or(0, |record| record.revision),
            reviewer: "Test reviewer".to_owned(),
            text: "Independent source reading אֵל".to_owned(),
            runs: None,
            state,
            comment: "Source checked".to_owned(),
        }
    }

    fn headword_fixture(
        partition: &str,
        page: &str,
    ) -> (tempfile::TempDir, TranscriptionStore, PathBuf) {
        let (temp, store) = fixture();
        let sample = store.root.join("sample");
        let mut draft: serde_json::Value =
            serde_json::from_slice(&fs::read(sample.join("draft.json")).unwrap()).unwrap();
        draft["lines"].as_array_mut().unwrap().truncate(1);
        fs::write(
            sample.join("draft.json"),
            serde_json::to_vec(&draft).unwrap(),
        )
        .unwrap();
        let mut review: serde_json::Value =
            serde_json::from_slice(&fs::read(sample.join("review.json")).unwrap()).unwrap();
        review["kind"] = "headword".into();
        review["partition"] = partition.into();
        review["printed_page"] = page.into();
        review["lines"].as_array_mut().unwrap().truncate(1);
        fs::write(
            sample.join("review.json"),
            serde_json::to_vec(&review).unwrap(),
        )
        .unwrap();
        let splits = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/sample-inventory/robinson-1854-splits.toml");
        (temp, store, splits)
    }

    #[test]
    fn headword_export_preserves_audit_and_rejects_stale_or_unresolved_labels() {
        let (temp, store, splits) = headword_fixture("training", "11");
        let line = store.lines().unwrap().remove(0);
        let mut change = update(&line, State::Unresolved);
        change.text = "אָב".into();
        store.apply(change).unwrap();
        let output = temp.path().join("export");
        assert!(export_headwords(&store.root, &store.journal, &splits, &output).is_err());
        assert!(!output.exists());
        let line = store.lines().unwrap().remove(0);
        let mut change = update(&line, State::Resolved);
        change.text = "אָב".into();
        store.apply(change).unwrap();
        assert_eq!(
            export_headwords(&store.root, &store.journal, &splits, &output).unwrap(),
            1
        );
        let manifest: serde_json::Value = serde_json::from_str(
            fs::read_to_string(output.join("ground-truth.jsonl"))
                .unwrap()
                .trim(),
        )
        .unwrap();
        assert_eq!(manifest["diplomatic"], "אָב");
        assert_eq!(manifest["review"]["revision"], 2);
        assert_eq!(manifest["split"], "train");
        assert_eq!(
            fs::read_to_string(manifest["ground_truth"].as_str().unwrap()).unwrap(),
            "אָב\n"
        );
        assert!(export_headwords(&store.root, &store.journal, &splits, &output).is_err());
        let draft = store.root.join("sample/draft.json");
        let bytes = fs::read_to_string(&draft).unwrap();
        fs::write(draft, format!("{bytes}\n")).unwrap();
        assert!(export_headwords(
            &store.root,
            &store.journal,
            &splits,
            &temp.path().join("stale")
        )
        .is_err());
    }

    #[test]
    fn rejected_headword_candidate_is_audited_and_not_exported() {
        let (temp, store, splits) = headword_fixture("training", "11");
        let line = store.lines().unwrap().remove(0);
        let mut rejected = update(&line, State::NotHeadword);
        rejected.text.clear();
        rejected.comment = "This crop is body text".to_owned();
        let saved = store.apply(rejected).unwrap();
        assert_eq!(saved.state, State::NotHeadword);
        assert_eq!(saved.comment, "This crop is body text");
        let error = export_headwords(
            &store.root,
            &store.journal,
            &splits,
            &temp.path().join("export"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("no resolved"));
    }

    #[test]
    fn unusable_headword_crop_is_audited_and_not_exported() {
        let (temp, store, splits) = headword_fixture("training", "11");
        let line = store.lines().unwrap().remove(0);
        let mut excluded = update(&line, State::Excluded);
        excluded.text.clear();
        excluded.runs = None;
        excluded.comment = "Source segmentation clips the final letter".to_owned();
        let saved = store.apply(excluded).unwrap();
        assert_eq!(saved.state, State::Excluded);
        let error = export_headwords(
            &store.root,
            &store.journal,
            &splits,
            &temp.path().join("export"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("no resolved"));
    }

    #[test]
    fn adjusted_crop_is_versioned_and_invalidates_the_previous_review() {
        let (_temp, store, _splits) = headword_fixture("training", "11");
        let review_path = store.root.join("sample/review.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&review_path).unwrap()).unwrap();
        let source = store
            .root
            .join("sample")
            .join(manifest["lines"][0]["crop"].as_str().unwrap());
        manifest["lines"][0]["rectangle"] = serde_json::json!([0, 0, 3, 3]);
        manifest["lines"][0]["crop_commands"] = serde_json::json!([
            ["magick", source, "-crop", "3x3+0+0", "+repage", "unused"],
            [
                "magick",
                source,
                "-crop",
                "3x3+0+0",
                "+repage",
                "unused-context"
            ],
            [
                "magick",
                source,
                "-crop",
                "3x3+0+0",
                "+repage",
                "unused-original"
            ]
        ]);
        fs::write(&review_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

        let line = store.lines().unwrap().remove(0);
        store.apply(update(&line, State::Resolved)).unwrap();
        let line = store.lines().unwrap().remove(0);
        store
            .adjust_crop(CropUpdate {
                sample: line.sample,
                line_id: line.line_id,
                source_digest: line.source_digest,
                rectangle: [0, 0, 2, 2],
            })
            .unwrap();

        let adjusted = store.lines().unwrap().remove(0);
        assert_eq!(adjusted.rectangle, Some([0, 0, 2, 2]));
        assert!(adjusted.crop.ends_with("p0050-right-001-2x2+0+0.png"));
        assert!(Path::new(&adjusted.crop).is_file());
        assert!(adjusted.review.is_none());
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(review_path).unwrap()).unwrap();
        assert_eq!(
            manifest["lines"][0]["crop_commands"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn validation_headwords_accept_one_resolved_human_review() {
        let (temp, store, splits) = headword_fixture("validation", "175");
        let line = store.lines().unwrap().remove(0);
        store.apply(update(&line, State::Resolved)).unwrap();
        let output = temp.path().join("export");
        assert_eq!(
            export_headwords(&store.root, &store.journal, &splits, &output).unwrap(),
            1
        );
        let manifest = fs::read_to_string(output.join("ground-truth.jsonl")).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&manifest).unwrap()["second_review"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn draft_review_approves_directly_and_rejects_stale_writes() {
        let (_temp, store) = fixture();
        let lines = store.lines().unwrap();
        assert_eq!(lines.len(), 12);
        assert!(lines.iter().all(|line| line.draft.is_some()));
        assert!(!lines[0].uncertainties.is_empty());
        assert!(store.apply(update(&lines[0], State::Reading)).is_err());
        let first = store.apply(update(&lines[0], State::Resolved)).unwrap();
        assert_eq!(first.review_method, ReviewMethod::DraftAssisted);
        assert!(first.independent_reading.is_none());
        assert_eq!(first.displayed_draft, lines[0].draft);
        assert_eq!(first.revision, 1);
        assert!(store
            .apply(update(&lines[0], State::Resolved))
            .unwrap_err()
            .to_string()
            .contains("revision conflict"));
        let compared = store.lines().unwrap();
        assert!(compared[0].draft.is_some());
        assert!(!compared[0].uncertainties.is_empty());
        assert!(compared[1].draft.is_some());
        let mut correction = update(&compared[0], State::Resolved);
        correction.text = "Corrected source reading".to_owned();
        let second = store.apply(correction).unwrap();
        assert!(second.independent_reading.is_none());
        assert_eq!(second.displayed_draft, first.displayed_draft);
        assert_ne!(second.text, first.text);
        assert_eq!(second.revision, 2);
        assert_eq!(store.records().unwrap().len(), 2);
        let reopened = TranscriptionStore {
            root: store.root.clone(),
            journal: store.journal.clone(),
        };
        assert_eq!(
            reopened.lines().unwrap()[0].review.as_ref().unwrap().state,
            State::Resolved
        );
        let mut wrong_reviewer = update(&reopened.lines().unwrap()[0], State::Resolved);
        wrong_reviewer.reviewer = "Different reviewer".to_owned();
        assert!(store.apply(wrong_reviewer).is_err());
    }

    #[test]
    fn legacy_independent_reading_survives_draft_assisted_updates() {
        let (_temp, store) = fixture();
        let line = store.lines().unwrap().remove(0);
        let legacy = serde_json::json!({
            "sample": line.sample, "line_id": line.line_id,
            "source_digest": line.source_digest, "revision": 1,
            "reviewer": "Test reviewer", "independent_reading": "Original blind reading",
            "text": "Original blind reading", "state": "reading", "comment": "",
            "reviewed_at": "2026-09-05T20:00:00Z"
        });
        let original = format!("{}\n", serde_json::to_string(&legacy).unwrap());
        fs::write(&store.journal, &original).unwrap();
        let line = store.lines().unwrap().remove(0);
        assert_eq!(
            line.review.as_ref().unwrap().review_method,
            ReviewMethod::LegacySourceFirst
        );
        let saved = store.apply(update(&line, State::Resolved)).unwrap();
        assert_eq!(
            saved.independent_reading.as_deref(),
            Some("Original blind reading")
        );
        assert_eq!(saved.review_method, ReviewMethod::DraftAssisted);
        assert!(fs::read_to_string(&store.journal)
            .unwrap()
            .starts_with(&original));
    }

    #[test]
    fn initial_uncertain_review_requires_a_note_and_is_not_resolved() {
        let (_temp, store) = fixture();
        let line = store.lines().unwrap().remove(0);
        let mut uncertain = update(&line, State::Unresolved);
        uncertain.comment.clear();
        assert!(store.apply(uncertain).is_err());
        let saved = store.apply(update(&line, State::Unresolved)).unwrap();
        assert_eq!(saved.state, State::Unresolved);
        assert!(saved.independent_reading.is_none());
    }

    #[test]
    fn language_runs_preserve_exact_text_and_reject_hidden_direction_controls() {
        let (_temp, store) = fixture();
        let line = store.lines().unwrap().remove(0);
        let mut change = update(&line, State::Resolved);
        change.text = "Plur. אֵלִם 1. mighty ones, heroes;".to_owned();
        change.runs = Some(vec![
            TextRun {
                language: "en".to_owned(),
                direction: Direction::Ltr,
                text: "Plur. ".to_owned(),
            },
            TextRun {
                language: "he".to_owned(),
                direction: Direction::Rtl,
                text: "אֵלִם".to_owned(),
            },
            TextRun {
                language: "en".to_owned(),
                direction: Direction::Ltr,
                text: " 1. mighty ones, heroes;".to_owned(),
            },
        ]);
        let saved = store.apply(change).unwrap();
        let runs = saved.runs.unwrap();
        assert_eq!(runs[1].language, "he");
        assert_eq!(runs[1].direction, Direction::Rtl);
        assert_eq!(
            store.lines().unwrap()[0]
                .review
                .as_ref()
                .unwrap()
                .runs
                .as_ref()
                .unwrap()[1]
                .text,
            "אֵלִם"
        );
        assert!(validate_runs(&runs, "different text").is_err());
        assert!(validate_runs(
            &[TextRun {
                language: "he".to_owned(),
                direction: Direction::Rtl,
                text: "\u{202e}א".to_owned()
            }],
            "\u{202e}א"
        )
        .is_err());
        assert!(validate_runs(
            &[TextRun {
                language: "unknown".to_owned(),
                direction: Direction::Ltr,
                text: "a".to_owned()
            }],
            "a"
        )
        .is_err());
    }

    #[test]
    fn changed_source_cannot_reuse_old_review_and_crops_are_verified() {
        let (_temp, store) = fixture();
        let initial = store.lines().unwrap();
        store.apply(update(&initial[0], State::Resolved)).unwrap();
        let compared = store.lines().unwrap();
        let path = store.root.join("sample/draft.json");
        let mut draft: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        draft["lines"][0]["text"] = "changed draft".into();
        fs::write(&path, serde_json::to_vec(&draft).unwrap()).unwrap();
        let changed = store.lines().unwrap();
        assert!(changed[0].review.is_none());
        assert_eq!(changed[0].draft.as_deref(), Some("changed draft"));
        assert!(store
            .apply(update(&compared[0], State::Resolved))
            .unwrap_err()
            .to_string()
            .contains("revision conflict"));
        fs::write(&changed[0].crop, b"tampered crop").unwrap();
        assert!(store
            .lines()
            .unwrap_err()
            .to_string()
            .contains("crop hash mismatch"));
    }

    #[test]
    fn held_out_samples_and_invalid_decisions_are_not_accepted() {
        let (_temp, store) = fixture();
        let line = store.lines().unwrap().remove(0);
        let mut blank = update(&line, State::Reading);
        blank.reviewer.clear();
        assert!(store.apply(blank).is_err());
        store.apply(update(&line, State::Resolved)).unwrap();
        let line = store.lines().unwrap().remove(0);
        let mut unresolved = update(&line, State::Unresolved);
        unresolved.comment.clear();
        assert!(store.apply(unresolved).is_err());
        assert!(store.apply(update(&line, State::Reading)).is_err());
        let path = store.root.join("sample/review.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["partition"] = "final-test".into();
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(store.lines().unwrap().is_empty());
    }
}
