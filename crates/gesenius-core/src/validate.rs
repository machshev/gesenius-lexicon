//! Corpus schema, Unicode, provenance, and assignment validation.

use crate::alto::{LineAssignment, ParsedPage};
use crate::language::profile_for_tag;
use crate::model::{stable_entry_id, CorpusEntry, Direction, ReviewState, TextSpan};
use crate::unicode::{classify_script, infer_direction, is_bidi_control, normalize_nfc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Validation issue severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Blocks a corpus release.
    Error,
    /// Requires attention but may remain in an OCR draft.
    Warning,
}

impl Severity {
    /// Whether this issue blocks release.
    #[must_use]
    pub const fn is_error(self) -> bool {
        matches!(self, Self::Error)
    }
}

/// Machine-readable validation issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Severity.
    pub severity: Severity,
    /// Stable issue code.
    pub code: String,
    /// Entry/span or artifact path.
    pub location: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Aggregate validation report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationReport {
    /// All issues in deterministic order.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// Number of release-blocking issues.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity.is_error())
            .count()
    }

    /// Number of warnings.
    #[must_use]
    pub fn warnings(&self) -> usize {
        self.issues.len() - self.errors()
    }

    /// True when no release-blocking issue exists.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors() == 0
    }
}

/// Validates a materialized corpus and optional run artifact root.
#[must_use]
pub fn validate_corpus(entries: &[CorpusEntry], run_root: Option<&Path>) -> ValidationReport {
    let mut report = ValidationReport::default();
    let mut entry_ids = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for entry in entries {
        if !entry_ids.insert(entry.id.clone()) {
            report.issues.push(error(
                "duplicate_entry_id",
                &entry.id,
                "entry ID occurs more than once",
            ));
        }
        for alias in &entry.aliases {
            if alias == &entry.id {
                report
                    .issues
                    .push(error("self_alias", &entry.id, "entry cannot alias itself"));
            }
            if let Some(previous) = aliases.insert(alias.clone(), entry.id.clone()) {
                report.issues.push(error(
                    "duplicate_alias",
                    alias,
                    &format!("alias belongs to both `{previous}` and `{}`", entry.id),
                ));
            }
        }
        report.issues.extend(validate_entry(entry));
    }
    if let Some(root) = run_root {
        validate_assignment_artifacts(root, &mut report);
    }
    report
        .issues
        .sort_by(|left, right| (&left.location, &left.code).cmp(&(&right.location, &right.code)));
    report
}

/// Validates one entry independently for review updates.
#[must_use]
pub fn validate_entry(entry: &CorpusEntry) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let expected_id = stable_entry_id(&entry.edition, &entry.printed_page, entry.entry_ordinal);
    if entry.id != expected_id && !entry.aliases.contains(&expected_id) {
        issues.push(error(
            "unstable_entry_id",
            &entry.id,
            &format!("expected source-derived ID `{expected_id}` or an alias for it"),
        ));
    }
    if entry.edition != entry.provenance.edition {
        issues.push(error(
            "edition_provenance_mismatch",
            &entry.id,
            "entry and provenance editions differ",
        ));
    }
    if !is_sha256(&entry.provenance.source_sha256) {
        issues.push(error(
            "invalid_source_hash",
            &entry.id,
            "source SHA-256 is not lowercase hexadecimal",
        ));
    }
    if entry.provenance.scan_id.trim().is_empty() || entry.provenance.pipeline_run.trim().is_empty()
    {
        issues.push(error(
            "incomplete_provenance",
            &entry.id,
            "scan ID and pipeline run are required",
        ));
    }
    if !(0.0..=1.0).contains(&entry.confidence) {
        issues.push(error(
            "invalid_confidence",
            &entry.id,
            "entry confidence must be between 0 and 1",
        ));
    }
    if entry.revision == 0 && entry.review_state != ReviewState::Machine {
        issues.push(error(
            "invalid_review_revision",
            &entry.id,
            "corrected or verified entries need a positive revision",
        ));
    }
    if entry.blocks.is_empty() {
        issues.push(error(
            "empty_entry",
            &entry.id,
            "entry has no ordered content blocks",
        ));
    }

    let mut span_ids = BTreeSet::new();
    for span in entry.spans() {
        if !span_ids.insert(span.id.clone()) {
            issues.push(error(
                "duplicate_span_id",
                &format!("{}#{}", entry.id, span.id),
                "span ID occurs more than once in the entry",
            ));
        }
        validate_span(entry, span, &mut issues);
    }
    issues
}

fn validate_span(entry: &CorpusEntry, span: &TextSpan, issues: &mut Vec<ValidationIssue>) {
    let location = format!("{}#{}", entry.id, span.id);
    if span.normalized != normalize_nfc(&span.diplomatic) {
        issues.push(error(
            "normalization_mismatch",
            &location,
            "normalized text must be NFC of diplomatic text",
        ));
    }
    if span.script != classify_script(&span.normalized) {
        issues.push(error(
            "script_mismatch",
            &location,
            "ISO 15924 script does not match text",
        ));
    }
    validate_languages(span, &location, issues);
    let inferred = infer_direction(&span.normalized);
    if span.direction != inferred
        && !(span.direction == Direction::Mixed && inferred != Direction::Mixed)
    {
        issues.push(error(
            "direction_mismatch",
            &location,
            "direction does not match stored logical-order text",
        ));
    }
    if span.diplomatic.chars().any(is_bidi_control) {
        issues.push(error(
            "embedded_bidi_control",
            &location,
            "embedded bidi controls are forbidden; use span direction metadata",
        ));
    }
    if span
        .diplomatic
        .chars()
        .any(|character| character == '\u{fffd}')
    {
        issues.push(error(
            "replacement_character",
            &location,
            "replacement characters must be resolved before release",
        ));
    }
    if !(0.0..=1.0).contains(&span.confidence) {
        issues.push(error(
            "invalid_confidence",
            &location,
            "span confidence must be between 0 and 1",
        ));
    }
    if span.coordinates.is_empty() {
        issues.push(error(
            "missing_coordinates",
            &location,
            "every span needs source coordinates",
        ));
    }
    for coordinate in &span.coordinates {
        if coordinate.source_page == 0
            || coordinate.region_id.is_empty()
            || coordinate.line_id.is_empty()
            || coordinate.transform_id.is_empty()
            || coordinate.page_image.is_empty()
            || coordinate.polygon.len() < 3
        {
            issues.push(error(
                "incomplete_coordinates",
                &location,
                "source page, region, line, polygon, transform, and image are required",
            ));
        }
    }
    if span.review_state == ReviewState::Machine && span.hypotheses.is_empty() {
        issues.push(error(
            "missing_ocr_provenance",
            &location,
            "machine spans need at least one OCR hypothesis",
        ));
    }
    for hypothesis in &span.hypotheses {
        if hypothesis.engine.is_empty()
            || hypothesis.engine_version.is_empty()
            || hypothesis.model.is_empty()
            || !is_sha256(&hypothesis.model_hash)
            || !(0.0..=1.0).contains(&hypothesis.confidence)
        {
            issues.push(error(
                "incomplete_ocr_provenance",
                &location,
                "OCR engine, version, model, SHA-256, and confidence are required",
            ));
        }
    }
    if span
        .hypotheses
        .windows(2)
        .any(|pair| pair[0].engine == pair[1].engine && pair[0].model == pair[1].model)
    {
        issues.push(warning(
            "duplicate_ocr_model",
            &location,
            "adjacent hypotheses name the same OCR engine and model",
        ));
    }
}

fn validate_languages(span: &TextSpan, location: &str, issues: &mut Vec<ValidationIssue>) {
    let known_tag = |language: &str| {
        matches!(language, "mul" | "und" | "zxx") || profile_for_tag(language).is_some()
    };
    if let Some(language) = &span.language {
        if !known_tag(language) {
            issues.push(error(
                "unknown_language_tag",
                location,
                &format!("language `{language}` is not in the edition catalogue"),
            ));
        }
    } else if span.normalized.chars().any(char::is_alphabetic) {
        issues.push(warning(
            "missing_language_metadata",
            location,
            "linguistic text has no BCP 47 language metadata",
        ));
    }

    let characters = span.normalized.chars().collect::<Vec<_>>();
    let mut previous_end = 0_usize;
    let mut languages = BTreeSet::new();
    for run in &span.language_runs {
        if run.start < previous_end || run.start >= run.end || run.end > characters.len() {
            issues.push(error(
                "invalid_language_run",
                location,
                "language runs must be ordered, non-overlapping, and within the normalized text",
            ));
            continue;
        }
        previous_end = run.end;
        if !known_tag(&run.language) || matches!(run.language.as_str(), "mul" | "zxx") {
            issues.push(error(
                "invalid_language_run_tag",
                location,
                &format!("language run has invalid concrete tag `{}`", run.language),
            ));
        }
        let text = characters[run.start..run.end].iter().collect::<String>();
        if classify_script(&text) != run.script {
            issues.push(error(
                "language_run_script_mismatch",
                location,
                "language run script does not match its normalized text range",
            ));
        }
        languages.insert(run.language.as_str());
    }
    let expected = match languages.len() {
        0 if span.normalized.chars().any(char::is_alphabetic) => None,
        0 => Some("zxx"),
        1 => languages.first().copied(),
        _ => Some("mul"),
    };
    if !span.language_runs.is_empty() && span.language.as_deref() != expected {
        issues.push(error(
            "language_summary_mismatch",
            location,
            "span language must summarize its concrete language runs",
        ));
    }
}

fn validate_assignment_artifacts(root: &Path, report: &mut ValidationReport) {
    let Ok(directories) = fs::read_dir(root) else {
        report.issues.push(error(
            "missing_run_root",
            &root.display().to_string(),
            "run artifact root is not readable",
        ));
        return;
    };
    for path in directories
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
    {
        if path.is_dir() {
            validate_assignment_artifacts(&path, report);
        } else if path.file_name().is_some_and(|name| name == "parsed.json") {
            match fs::read(&path)
                .ok()
                .and_then(|content| serde_json::from_slice::<ParsedPage>(&content).ok())
            {
                Some(page) => {
                    let mut lines = BTreeSet::new();
                    for (region, line, assignment) in &page.assignments {
                        if region.is_empty() || line.is_empty() {
                            report.issues.push(error(
                                "empty_line_identity",
                                &path.display().to_string(),
                                "assignment has an empty region or line ID",
                            ));
                        }
                        if !lines.insert((region, line)) {
                            report.issues.push(error(
                                "duplicate_line_assignment",
                                &path.display().to_string(),
                                &format!("line `{region}/{line}` is assigned more than once"),
                            ));
                        }
                        if let LineAssignment::Entry(entry_id) = assignment {
                            if !page.entries.iter().any(|entry| &entry.id == entry_id) {
                                report.issues.push(error(
                                    "missing_assigned_entry",
                                    &path.display().to_string(),
                                    &format!("line refers to missing entry `{entry_id}`"),
                                ));
                            }
                        }
                    }
                    for entry in &page.entries {
                        for coordinate in entry
                            .blocks
                            .iter()
                            .flat_map(|block| block.spans.iter())
                            .flat_map(|span| span.coordinates.iter())
                        {
                            if page.source_page != 0 && coordinate.source_page != page.source_page {
                                continue;
                            }
                            if !lines.contains(&(&coordinate.region_id, &coordinate.line_id)) {
                                report.issues.push(error(
                                    "unassigned_recognized_line",
                                    &path.display().to_string(),
                                    &format!(
                                        "entry span line `{}/{}` has no assignment",
                                        coordinate.region_id, coordinate.line_id
                                    ),
                                ));
                            }
                        }
                    }
                }
                None => report.issues.push(error(
                    "invalid_parse_artifact",
                    &path.display().to_string(),
                    "parsed page artifact is not valid JSON",
                )),
            }
        }
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn error(code: &str, location: &str, message: &str) -> ValidationIssue {
    ValidationIssue {
        severity: Severity::Error,
        code: code.to_owned(),
        location: location.to_owned(),
        message: message.to_owned(),
    }
}

fn warning(code: &str, location: &str, message: &str) -> ValidationIssue {
    ValidationIssue {
        severity: Severity::Warning,
        code: code.to_owned(),
        location: location.to_owned(),
        message: message.to_owned(),
    }
}
