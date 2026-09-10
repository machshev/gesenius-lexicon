//! Frozen headword inventory and exact, detection-aware benchmark reports.
use crate::page_splits::{PageSplits, Partition};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// A frozen inventory includes every true headword on a fully inventoried page.
#[derive(Debug, Serialize, Deserialize)]
pub struct HeadwordBenchmark {
    /// Schema version.
    pub version: u32,
    /// Edition identifier.
    pub edition: String,
    /// Immutable source PDF hash.
    pub source_sha256: String,
    /// Explicit partition; final test requires an explicit CLI opt-in.
    pub partition: Partition,
    /// True only after a human has inventoried all headwords on these pages.
    pub complete_page_inventory: bool,
    /// Label authority; regression evidence is not human-approved training gold.
    pub authority: String,
    /// True headwords, including missing detections.
    pub headwords: Vec<Headword>,
}
/// One source-anchored headword.
#[derive(Debug, Serialize, Deserialize)]
pub struct Headword {
    /// Stable source identifier.
    pub id: String,
    /// Printed page.
    pub printed_page: String,
    /// PDF page.
    pub source_page: u32,
    /// Logical-order diplomatic transcription.
    pub diplomatic: String,
    /// Processed crop relative to manifest directory.
    pub crop: String,
    /// Crop hash.
    pub crop_sha256: String,
    /// Processed-page bounding box, x/y/width/height.
    pub rectangle: [u32; 4],
}
/// A detection matched to a source anchor by an audited matching stage.
#[derive(Debug, Serialize, Deserialize)]
pub struct Prediction {
    /// None denotes a false entry start; duplicate IDs are extra detections.
    pub gold_id: Option<String>,
    /// Printed page, including for false detections.
    pub printed_page: String,
    /// Final extracted headword, preserving raw Unicode.
    pub text: String,
}
/// Frozen recognition outputs and run identities.
#[derive(Debug, Serialize, Deserialize)]
pub struct HeadwordPredictions {
    /// Schema version.
    pub version: u32,
    /// Exact benchmark file hash.
    pub benchmark_sha256: String,
    /// Reproducible code/model/config identities and candidate evidence.
    pub provenance: serde_json::Value,
    /// All detections, including false starts.
    pub predictions: Vec<Prediction>,
}
/// Point counts attached to matching base letters. Extras are false positives.
#[derive(Debug, Default, Serialize)]
pub struct MarkCounts {
    /// Correctly associated marks.
    pub correct: usize,
    /// Missing marks.
    pub omitted: usize,
    /// Extra marks, including substitutions.
    pub extra: usize,
}
/// Counts retain denominators even on tiny regression fixtures.
#[derive(Debug, Default, Serialize)]
pub struct Counts {
    /// True source headwords.
    pub truth: usize,
    /// All proposed entry starts.
    pub predictions: usize,
    /// Unique matched detections.
    pub detected: usize,
    /// Detected and NFC-exact headwords.
    pub exact: usize,
    /// Detected headwords with exact bases (points removed).
    pub consonant_exact: usize,
    /// Per-scalar base-aligned point counts.
    pub marks: BTreeMap<String, MarkCounts>,
    /// Qametz absent with no replacement mark on the matching base.
    pub qametz_omissions: usize,
    /// Qametz absent with a replacement mark on the matching base.
    pub qametz_substitutions: usize,
}
/// Report separates conditional recognition from full-page detection quality.
#[derive(Debug, Serialize)]
pub struct HeadwordReport {
    /// Exact input identities and the recorded recognition run provenance.
    pub provenance: serde_json::Value,
    /// Aggregate counts.
    pub counts: Counts,
    /// Page-level counts; use pages, not independent words, as sampling units.
    pub pages: BTreeMap<String, Counts>,
    /// Conditional exact accuracy on detected headwords.
    pub exact_accuracy: Option<f64>,
    /// Conditional base-only exact accuracy.
    pub consonant_accuracy: Option<f64>,
    /// Detection metrics are unavailable for incomplete inventories.
    pub detection_precision: Option<f64>,
    /// True detected / all true headwords.
    pub detection_recall: Option<f64>,
    /// Detected AND exact / all true headwords.
    pub end_to_end_accuracy: Option<f64>,
    /// Base-aligned point precision.
    pub mark_precision: Option<f64>,
    /// Base-aligned point recall.
    pub mark_recall: Option<f64>,
    /// Page bootstrap interval for end-to-end accuracy, absent below two pages.
    pub page_bootstrap_95: Option<[f64; 2]>,
    /// Evidence limitation.
    pub limitation: String,
}
fn ratio(n: usize, d: usize) -> Option<f64> {
    (d != 0).then(|| n as f64 / d as f64)
}
/// SHA-256 identity for frozen files.
pub fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}
fn clusters(text: &str) -> Vec<(char, Vec<char>)> {
    let mut result: Vec<(char, Vec<char>)> = Vec::new();
    for c in text.nfd() {
        if is_combining_mark(c) {
            if result.is_empty() {
                result.push(('\0', Vec::new()));
            }
            result.last_mut().unwrap().1.push(c);
        } else {
            result.push((c, Vec::new()));
        }
    }
    result
}
fn add_word(counts: &mut Counts, gold: &str, prediction: Option<&str>) {
    counts.truth += 1;
    if let Some(text) = prediction {
        counts.detected += 1;
        counts.exact += usize::from(gold.nfc().eq(text.nfc()));
        counts.consonant_exact += usize::from(
            gold.nfd()
                .filter(|c| !is_combining_mark(*c))
                .eq(text.nfd().filter(|c| !is_combining_mark(*c))),
        );
    }
    let reference = clusters(gold);
    let hypothesis = clusters(prediction.unwrap_or(""));
    let rb: Vec<_> = reference.iter().map(|x| x.0).collect();
    let hb: Vec<_> = hypothesis.iter().map(|x| x.0).collect();
    for (r, h) in crate::metrics::alignment::align(&rb, &hb) {
        let rm = r.map(|i| reference[i].1.as_slice()).unwrap_or(&[]);
        let hm = h.map(|i| hypothesis[i].1.as_slice()).unwrap_or(&[]);
        let bases_match = r.zip(h).is_some_and(|(i, j)| rb[i] == hb[j]);
        let mut remaining = hm.to_vec();
        for mark in rm {
            let matched = bases_match
                .then(|| remaining.iter().position(|c| c == mark))
                .flatten();
            let count = counts
                .marks
                .entry(format!("U+{:04X}", *mark as u32))
                .or_default();
            if let Some(i) = matched {
                count.correct += 1;
                remaining.remove(i);
            } else {
                count.omitted += 1;
            }
        }
        if rm.contains(&'\u{05b8}') && (!bases_match || !hm.contains(&'\u{05b8}')) {
            if bases_match && !remaining.is_empty() {
                counts.qametz_substitutions += 1;
            } else {
                counts.qametz_omissions += 1;
            }
        }
        for mark in remaining {
            counts
                .marks
                .entry(format!("U+{:04X}", mark as u32))
                .or_default()
                .extra += 1;
        }
    }
}
/// Evaluates a manifest and pinned predictions; validates crops and page isolation first.
pub fn evaluate(
    manifest: &Path,
    predictions: &Path,
    splits: &PageSplits,
    allow_final_test: bool,
) -> Result<HeadwordReport> {
    let bytes = fs::read(manifest)?;
    let gold: HeadwordBenchmark = serde_json::from_slice(&bytes)?;
    let output: HeadwordPredictions = serde_json::from_slice(&fs::read(predictions)?)?;
    if gold.version != 1
        || output.version != 1
        || output.benchmark_sha256 != digest(&bytes)
        || gold.source_sha256 != splits.source_sha256
        || gold.authority.trim().is_empty()
        || gold.headwords.is_empty()
    {
        bail!("invalid benchmark/source/output identity");
    }
    if gold.partition == Partition::FinalTest && !allow_final_test {
        bail!("final test is sealed; freeze policy and pass --allow-final-test deliberately");
    }
    let root = manifest.parent().unwrap_or(Path::new(".")).canonicalize()?;
    let mut ids = BTreeSet::new();
    let mut crops = BTreeSet::new();
    let mut pages = BTreeMap::<String, Counts>::new();
    for word in &gold.headwords {
        if splits.partition(&gold.edition, &word.printed_page)? != gold.partition
            || !ids.insert(word.id.as_str())
            || !crops.insert(&word.crop_sha256)
            || word.diplomatic.trim().is_empty()
            || word.source_page == 0
            || word.rectangle[2..].contains(&0)
        {
            bail!("invalid, duplicate or mispartitioned headword {}", word.id);
        }
        let crop = root.join(&word.crop).canonicalize()?;
        if !crop.starts_with(&root) || digest(&fs::read(crop)?) != word.crop_sha256 {
            bail!("stale/invalid crop for {}", word.id);
        }
        pages.entry(word.printed_page.clone()).or_default();
    }
    let mut matched = BTreeMap::new();
    let mut counts = Counts::default();
    for prediction in &output.predictions {
        let page = pages
            .get_mut(&prediction.printed_page)
            .ok_or_else(|| anyhow::anyhow!("prediction outside inventory pages"))?;
        page.predictions += 1;
        counts.predictions += 1;
        if let Some(id) = &prediction.gold_id {
            let word = gold
                .headwords
                .iter()
                .find(|w| &w.id == id)
                .ok_or_else(|| anyhow::anyhow!("unknown gold ID"))?;
            if word.printed_page != prediction.printed_page {
                bail!("prediction matched across pages");
            }
            // First emitted candidate is the selected detection; duplicates remain false positives.
            matched
                .entry(id.as_str())
                .or_insert(prediction.text.as_str());
        }
    }
    for word in &gold.headwords {
        let text = matched.get(word.id.as_str()).copied();
        add_word(&mut counts, &word.diplomatic, text);
        add_word(
            pages.get_mut(&word.printed_page).unwrap(),
            &word.diplomatic,
            text,
        );
    }
    let correct = counts.marks.values().map(|m| m.correct).sum::<usize>();
    let extra = counts.marks.values().map(|m| m.extra).sum::<usize>();
    let omitted = counts.marks.values().map(|m| m.omitted).sum::<usize>();
    let complete = gold.complete_page_inventory;
    let page_bootstrap_95 = if complete && pages.len() > 1 {
        let pages: Vec<_> = pages.values().collect();
        let mut state = 1854_u64;
        let mut scores = Vec::new();
        for _ in 0..2000 {
            let (mut exact, mut truth) = (0, 0);
            for _ in 0..pages.len() {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let page = pages[(state >> 32) as usize % pages.len()];
                exact += page.exact;
                truth += page.truth;
            }
            scores.push(exact as f64 / truth as f64);
        }
        scores.sort_by(f64::total_cmp);
        Some([scores[49], scores[1949]])
    } else {
        None
    };
    Ok(HeadwordReport {
        provenance: serde_json::json!({
            "benchmark_sha256": digest(&bytes),
            "predictions_sha256": digest(&fs::read(predictions)?),
            "split_sha256": splits.manifest_sha256,
            "partition": gold.partition,
            "recognition": output.provenance,
            "normalization": "NFC",
            "mark_alignment": "NFD base-letter minimum-edit alignment; multiset marks per matching base",
            "evaluator_version": 1
        }),
        exact_accuracy: ratio(counts.exact, counts.detected),
        consonant_accuracy: ratio(counts.consonant_exact, counts.detected),
        detection_precision: complete.then(|| ratio(counts.detected, counts.predictions)).flatten(),
        detection_recall: complete.then(|| ratio(counts.detected, counts.truth)).flatten(),
        end_to_end_accuracy: complete.then(|| ratio(counts.exact, counts.truth)).flatten(),
        mark_precision: ratio(correct, correct + extra), mark_recall: ratio(correct, correct + omitted),
        page_bootstrap_95, counts, pages,
        limitation: if complete { "Page bootstrap uses 2,000 fixed-seed resamples; sparse marks remain unverified." } else { "Partial development regression, not a complete inventory: detection and end-to-end rates are unavailable." }.into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_page17_regression_preserves_the_missing_qametz() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks");
        let splits =
            PageSplits::load(&root.join("sample-inventory/robinson-1854-splits.toml")).unwrap();
        let report = evaluate(
            &root.join("headwords/page17/manifest.json"),
            &root.join("headwords/page17/predictions.json"),
            &splits,
            false,
        )
        .unwrap();
        assert_eq!(report.counts.truth, 1);
        assert_eq!(report.exact_accuracy, Some(0.0));
        assert_eq!(report.consonant_accuracy, Some(1.0));
        assert_eq!(report.counts.qametz_omissions, 1);
        assert!(report.detection_recall.is_none());
        assert!(report.page_bootstrap_95.is_none());
    }
    #[test]
    fn complete_inventory_penalizes_missing_and_false_detections() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks");
        let splits =
            PageSplits::load(&root.join("sample-inventory/robinson-1854-splits.toml")).unwrap();
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("one.png"), b"first test crop").unwrap();
        fs::write(temp.path().join("two.png"), b"second test crop").unwrap();
        let gold = HeadwordBenchmark {
            version: 1,
            edition: splits.edition.clone(),
            source_sha256: splits.source_sha256.clone(),
            partition: Partition::Development,
            complete_page_inventory: true,
            authority: "synthetic test".into(),
            headwords: vec![
                Headword {
                    id: "one".into(),
                    printed_page: "1".into(),
                    source_page: 17,
                    diplomatic: "אָב".into(),
                    crop: "one.png".into(),
                    crop_sha256: digest(b"first test crop"),
                    rectangle: [1, 1, 10, 10],
                },
                Headword {
                    id: "two".into(),
                    printed_page: "50".into(),
                    source_page: 66,
                    diplomatic: "אב".into(),
                    crop: "two.png".into(),
                    crop_sha256: digest(b"second test crop"),
                    rectangle: [1, 1, 10, 10],
                },
            ],
        };
        let manifest = temp.path().join("manifest.json");
        let predictions = temp.path().join("predictions.json");
        let bytes = serde_json::to_vec(&gold).unwrap();
        fs::write(&manifest, &bytes).unwrap();
        let output = HeadwordPredictions {
            version: 1,
            benchmark_sha256: digest(&bytes),
            provenance: serde_json::json!({"test":true}),
            predictions: vec![
                Prediction {
                    gold_id: Some("one".into()),
                    printed_page: "1".into(),
                    text: "אָב".into(),
                },
                Prediction {
                    gold_id: None,
                    printed_page: "50".into(),
                    text: "extra".into(),
                },
            ],
        };
        fs::write(&predictions, serde_json::to_vec(&output).unwrap()).unwrap();
        let report = evaluate(&manifest, &predictions, &splits, false).unwrap();
        assert_eq!(report.exact_accuracy, Some(1.0));
        assert_eq!(report.detection_recall, Some(0.5));
        assert_eq!(report.detection_precision, Some(0.5));
        assert_eq!(report.end_to_end_accuracy, Some(0.5));
        assert!(report.page_bootstrap_95.is_some());
        fs::write(temp.path().join("one.png"), b"changed").unwrap();
        assert!(evaluate(&manifest, &predictions, &splits, false).is_err());
    }
    #[test]
    fn points_are_base_aligned_and_missing_entries_fail() {
        let mut c = Counts::default();
        add_word(&mut c, "אָב", Some("אַב"));
        add_word(&mut c, "אב", Some("אָב"));
        add_word(&mut c, "אָב", None);
        add_word(&mut c, "אָב", Some("אבָ"));
        assert_eq!(c.truth, 4);
        assert_eq!(c.detected, 3);
        assert_eq!(c.exact, 0);
        assert_eq!(c.consonant_exact, 3);
        assert_eq!(c.marks["U+05B8"].omitted, 3);
        assert_eq!(c.marks["U+05B8"].extra, 2);
        assert_eq!(c.qametz_substitutions, 1);
    }
    #[test]
    fn legitimate_unpointed_and_canonical_order_are_exact() {
        let mut c = Counts::default();
        add_word(&mut c, "אב", Some("אב"));
        add_word(&mut c, "ש\u{05c1}\u{05b8}", Some("ש\u{05b8}\u{05c1}"));
        assert_eq!(c.exact, 2);
        assert_eq!(c.marks["U+05B8"].correct, 1);
    }
}
