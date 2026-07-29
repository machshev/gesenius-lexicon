//! Edition completeness, OCR quality, parsing, and editorial comparison reports.

use crate::model::{AccuracyMetrics, CorpusEntry, ReviewState};
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// Machine-readable edition comparison.
#[derive(Debug, Serialize)]
pub struct ComparisonReport {
    /// Corpus editions.
    pub editions: BTreeMap<String, EditionStatistics>,
    /// Headwords only found in one edition.
    pub edition_only_headwords: BTreeMap<String, Vec<String>>,
    /// Shared headwords with differing diplomatic content.
    pub editorial_differences: Vec<EditorialDifference>,
    /// External pilot benchmark metrics, if supplied.
    pub pilot_metrics: AccuracyMetrics,
    /// Important interpretation note.
    pub canonical_decision: String,
}

/// Edition-level coverage and quality proxies.
#[derive(Debug, Serialize)]
pub struct EditionStatistics {
    /// Materialized lexical entries.
    pub entries: usize,
    /// Distinct processed source pages.
    pub processed_pages: usize,
    /// Distinct printed pages.
    pub printed_pages: usize,
    /// Mean entry OCR confidence.
    pub mean_confidence: f64,
    /// Machine entries below 80 percent confidence.
    pub low_confidence_entries: usize,
    /// Entries in each review state.
    pub review_states: BTreeMap<String, usize>,
    /// Script spans in this edition.
    pub spans_by_script: BTreeMap<String, usize>,
    /// Entries without a confidently detected headword.
    pub unparsed_headwords: usize,
}

/// Same headword with non-identical edition text.
#[derive(Debug, Serialize)]
pub struct EditorialDifference {
    /// NFC headword and homograph identity.
    pub headword: String,
    /// Entry IDs keyed by edition.
    pub entries: BTreeMap<String, String>,
    /// Diplomatic entry text keyed by edition.
    pub texts: BTreeMap<String, String>,
}

/// Creates comparison statistics without selecting a canonical edition.
#[must_use]
pub fn compare_editions(
    entries: &[CorpusEntry],
    pilot_metrics: AccuracyMetrics,
) -> ComparisonReport {
    let edition_names: BTreeSet<_> = entries.iter().map(|entry| entry.edition.clone()).collect();
    let mut editions = BTreeMap::new();
    for edition in &edition_names {
        let edition_entries: Vec<_> = entries
            .iter()
            .filter(|entry| &entry.edition == edition)
            .collect();
        let source_pages: BTreeSet<_> = edition_entries
            .iter()
            .flat_map(|entry| entry.spans())
            .flat_map(|span| span.coordinates.iter())
            .map(|coordinate| coordinate.source_page)
            .collect();
        let printed_pages: BTreeSet<_> = edition_entries
            .iter()
            .map(|entry| &entry.printed_page)
            .collect();
        let mut review_states = BTreeMap::new();
        let mut spans_by_script = BTreeMap::new();
        for entry in &edition_entries {
            *review_states
                .entry(entry.review_state.as_str().to_owned())
                .or_insert(0) += 1;
            for span in entry.spans() {
                *spans_by_script.entry(span.script.clone()).or_insert(0) += 1;
            }
        }
        let confidence_sum: f64 = edition_entries
            .iter()
            .map(|entry| f64::from(entry.confidence))
            .sum();
        editions.insert(
            edition.clone(),
            EditionStatistics {
                entries: edition_entries.len(),
                processed_pages: source_pages.len(),
                printed_pages: printed_pages.len(),
                mean_confidence: if edition_entries.is_empty() {
                    0.0
                } else {
                    confidence_sum / edition_entries.len() as f64
                },
                low_confidence_entries: edition_entries
                    .iter()
                    .filter(|entry| {
                        entry.review_state == ReviewState::Machine && entry.confidence < 0.8
                    })
                    .count(),
                review_states,
                spans_by_script,
                unparsed_headwords: edition_entries
                    .iter()
                    .filter(|entry| entry.headword.is_none())
                    .count(),
            },
        );
    }

    let mut by_headword = BTreeMap::<String, Vec<&CorpusEntry>>::new();
    for entry in entries {
        if let Some(headword) = &entry.headword {
            let key = format!(
                "{}#{}",
                headword.normalized,
                entry.homograph.unwrap_or_default()
            );
            by_headword.entry(key).or_default().push(entry);
        }
    }
    let mut edition_only_headwords = BTreeMap::<String, Vec<String>>::new();
    let mut editorial_differences = Vec::new();
    for (headword, matched) in by_headword {
        let represented: BTreeSet<_> = matched.iter().map(|entry| entry.edition.clone()).collect();
        if represented.len() == 1 && edition_names.len() > 1 {
            edition_only_headwords
                .entry(matched[0].edition.clone())
                .or_default()
                .push(headword);
        } else if represented.len() > 1 {
            let texts: BTreeMap<_, _> = matched
                .iter()
                .map(|entry| (entry.edition.clone(), entry_text(entry)))
                .collect();
            let unique_texts: BTreeSet<_> = texts.values().collect();
            if unique_texts.len() > 1 {
                editorial_differences.push(EditorialDifference {
                    headword,
                    entries: matched
                        .iter()
                        .map(|entry| (entry.edition.clone(), entry.id.clone()))
                        .collect(),
                    texts,
                });
            }
        }
    }
    for headwords in edition_only_headwords.values_mut() {
        headwords.sort();
    }
    editorial_differences.sort_by(|left, right| left.headword.cmp(&right.headword));

    ComparisonReport {
        editions,
        edition_only_headwords,
        editorial_differences,
        pilot_metrics,
        canonical_decision:
            "No canonical edition is selected automatically; the project owner decides after pilot review."
                .to_owned(),
    }
}

/// Writes JSON and a concise Markdown companion.
pub fn write_report(output_directory: &Path, report: &ComparisonReport) -> Result<()> {
    fs::create_dir_all(output_directory)?;
    fs::write(
        output_directory.join("edition-comparison.json"),
        format!("{}\n", serde_json::to_string_pretty(report)?),
    )?;
    let mut markdown = String::from(
        "# Gesenius edition comparison\n\n\
         This report does not select a canonical edition. Counts describe the currently processed corpus, not the full printed books.\n\n",
    );
    markdown.push_str("| Edition | Entries | Source pages | Mean confidence | Low-confidence machine entries | Missing headwords |\n");
    markdown.push_str("|---|---:|---:|---:|---:|---:|\n");
    for (edition, statistics) in &report.editions {
        let _ = writeln!(
            markdown,
            "| {edition} | {} | {} | {:.3} | {} | {} |",
            statistics.entries,
            statistics.processed_pages,
            statistics.mean_confidence,
            statistics.low_confidence_entries,
            statistics.unparsed_headwords
        );
    }
    let _ = write!(
        markdown,
        "\nShared headwords with differing entry text: {}.\n\n",
        report.editorial_differences.len()
    );
    for (edition, headwords) in &report.edition_only_headwords {
        let _ = writeln!(
            markdown,
            "- Headwords currently unique to {edition}: {}",
            headwords.len()
        );
    }
    fs::write(output_directory.join("edition-comparison.md"), markdown)?;
    Ok(())
}

fn entry_text(entry: &CorpusEntry) -> String {
    entry
        .blocks
        .iter()
        .flat_map(|block| block.spans.iter())
        .map(|span| span.diplomatic.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
