//! Source-checked draft review, separate from corpus correction patches.

use super::{content_type_header, respond_error, respond_html, respond_json, security_header};
use crate::benchmark::GoldBenchmark;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use tiny_http::{Method, Request, Response, StatusCode};

#[derive(Deserialize)]
struct Manifest {
    partition: String,
    printed_page: String,
    lines: Vec<Crop>,
    unresolved: Vec<Uncertainty>,
}

#[derive(Deserialize)]
struct Crop {
    line_id: String,
    crop: String,
    crop_sha256: String,
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
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReviewMethod {
    // Records written before draft-prefilled review used the source-first workflow.
    #[default]
    LegacySourceFirst,
    DraftAssisted,
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

#[derive(Debug, Serialize)]
struct Line {
    sample: String,
    line_id: String,
    printed_page: String,
    source_page: u32,
    partition: String,
    source_digest: String,
    crop: String,
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
            if !matches!(manifest.partition.as_str(), "development" | "validation") {
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
                    sample: sample.clone(),
                    line_id: gold.line_id.clone(),
                    printed_page: manifest.printed_page.clone(),
                    source_page: benchmark.source_page,
                    partition: manifest.partition.clone(),
                    source_digest: source_digest.clone(),
                    crop: crop_path.to_string_lossy().into_owned(),
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
        if update.reviewer.trim().is_empty() || update.text.trim().is_empty() {
            bail!("reviewer and transcription are required");
        }
        if let Some(runs) = &update.runs {
            validate_runs(runs, &update.text)?;
        }
        if update.state == State::Reading {
            bail!("the draft is now visible; reload and save a resolved or unresolved review");
        }
        if update.state == State::Unresolved && update.comment.trim().is_empty() {
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
            if let Some(previous) = &line.review {
                if previous.reviewer != update.reviewer.trim() {
                    bail!("continue with the reviewer who started this line review");
                }
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
                    .and_then(|record| record.independent_reading.clone()),
                review_method: ReviewMethod::DraftAssisted,
                displayed_draft: line.draft.clone(),
                text: update.text,
                runs: update.runs,
                state: update.state,
                comment: update.comment,
                reviewed_at: Utc::now(),
            };
            serde_json::to_writer(&mut file, &record)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(record)
        })();
        let _ = FileExt::unlock(&file);
        result
    }
}

pub(super) fn handle(mut request: Request, store: &TranscriptionStore) -> Result<()> {
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
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
    respond_error(request, StatusCode(404), "not found")
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
