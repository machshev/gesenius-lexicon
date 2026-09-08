//! Lightweight source-anchored entry index, independent of definition review.

use crate::model::CorpusEntry;
use crate::source::SourceRecord;
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Builds a draft index from materialized entries without claiming full-page coverage.
pub fn build_index(entries: &[CorpusEntry], source: &SourceRecord) -> Result<Value> {
    let mut selected: Vec<_> = entries
        .iter()
        .filter(|e| e.edition == source.edition)
        .collect();
    let mut pages = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for entry in &selected {
        ensure!(ids.insert(&entry.id), "duplicate entry ID {}", entry.id);
        ensure!(
            entry.provenance.source_sha256 == source.sha256,
            "source mismatch for {}",
            entry.id
        );
        for coordinate in entry.spans().flat_map(|span| &span.coordinates) {
            ensure!(
                coordinate.source_page > 0,
                "invalid source page for {}",
                entry.id
            );
            pages.insert(coordinate.source_page);
        }
    }
    selected.sort_by_key(|e| {
        (
            e.spans()
                .flat_map(|s| &s.coordinates)
                .map(|s| s.source_page)
                .min(),
            e.entry_ordinal,
            &e.id,
        )
    });
    let rows: Vec<_> = selected
        .iter()
        .map(|entry| {
            let mut coordinates = Vec::new();
            for coordinate in entry.spans().flat_map(|span| &span.coordinates) {
                if !coordinates.contains(&coordinate) {
                    coordinates.push(coordinate);
                }
            }
            json!({
                "id": entry.id,
                "aliases": entry.aliases,
                "headword": entry.headword.as_ref().map(|h| &h.diplomatic),
                "headword_nfc": entry.headword.as_ref().map(|h| &h.normalized),
                "headword_review_state": entry.headword.as_ref().map(|h| h.review_state),
                "boundary_review_state": "unreviewed",
                "homograph": entry.homograph,
                "printed_page": entry.printed_page,
                "source_regions": coordinates,
                "corpus_revision": entry.revision,
                "pipeline_run": entry.provenance.pipeline_run,
            })
        })
        .collect();
    Ok(json!({
        "schema_version": 1,
        "edition": source.edition,
        "source_sha256": source.sha256,
        "scan_id": source.scan_id,
        "source_page_count": source.page_count,
        "coverage": {
            "pages_with_entry_regions": pages,
            "complete_document_verified": false,
            "entry_count": rows.len(),
            "missing_headword_count": selected.iter().filter(|e| e.headword.is_none()).count(),
        },
        "entries": rows,
    }))
}
