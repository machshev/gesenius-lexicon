//! Comparable stage scores with exact input fingerprints and signed deltas.

use super::{evaluate_alto_with_identity, BenchmarkResult, GoldBenchmark, SourceIdentity};
use crate::alto::parse_alto;
use crate::metrics::{RecognitionMetrics, ScriptCounts, WordCounts};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Inputs for one sample, in the caller's intended stage order.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageComparisonManifest {
    /// Gold fixture path, relative to this manifest unless absolute.
    pub gold: PathBuf,
    /// Name of the stage used for every baseline delta.
    pub baseline: String,
    /// At least two uniquely named stage artifacts.
    pub stages: Vec<StageInput>,
}

/// ALTO and separately asserted provenance for a named stage.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageInput {
    /// Caller-defined label such as `page`, `block`, or `fusion`.
    pub name: String,
    /// ALTO path, relative to the manifest unless absolute.
    pub alto: PathBuf,
    /// Source identity JSON path; required even for legacy gold fixtures.
    pub hypothesis_identity: PathBuf,
}

/// Content fingerprint of bytes actually parsed during this evaluation.
#[derive(Debug, Serialize)]
pub struct InputFingerprint {
    /// Resolved input path (not necessarily canonicalized).
    pub path: PathBuf,
    /// SHA-256 of the exact bytes read, before parsing.
    pub sha256: String,
}

/// Scores on a single unchanged gold sample, with no automatic adoption claim.
#[derive(Debug, Serialize)]
pub struct StageComparison {
    /// Report format version.
    pub schema_version: u32,
    /// Library package version; the CLI separately fingerprints its executable.
    pub evaluator_version: String,
    /// Manifest bytes used to resolve and order the inputs.
    pub manifest: InputFingerprint,
    /// Exact gold input shared by every stage.
    pub gold: InputFingerprint,
    /// How the reference text was produced and accepted.
    pub authority: String,
    /// Named baseline, which may appear anywhere in stage order.
    pub baseline: String,
    /// Results in manifest order, without averaging rates across samples.
    pub stages: Vec<StageScore>,
}

/// Full stage result and changes relative to explicit comparison stages.
#[derive(Debug, Serialize)]
pub struct StageScore {
    /// Manifest stage name.
    pub name: String,
    /// Evaluated ALTO bytes.
    pub alto: InputFingerprint,
    /// Parsed identity file bytes.
    pub hypothesis_identity: InputFingerprint,
    /// Caller assertion, checked against gold; not proof of ALTO generation history.
    pub asserted_identity: SourceIdentity,
    /// Unabridged recognition, support, alignment and segmentation measurements.
    pub result: BenchmarkResult,
    /// This stage minus the named baseline; negative error deltas are improvements.
    pub against_baseline: MetricDelta,
    /// This stage minus its predecessor in manifest order; absent for the first.
    pub against_previous: Option<MetricDelta>,
}

/// Signed changes on identical reference support; rates are differences, not ratios.
#[derive(Debug, Serialize)]
pub struct MetricDelta {
    /// Stage being subtracted.
    pub from_stage: String,
    /// Diplomatic character error rate difference.
    pub cer: f64,
    /// Diplomatic word error rate difference.
    pub wer: f64,
    /// Canonically decomposed base-letter diagnostic difference.
    pub base_letter_cer: f64,
    /// Aligned combining-mark diagnostic difference.
    pub combining_mark_cer: f64,
    /// NFC character error rate difference, separate from diplomatic error.
    pub nfc_cer: f64,
    /// NFC word error rate difference.
    pub nfc_wer: f64,
    /// Change in missed exact foreign-containing tokens, counted once per token.
    pub missed_foreign_words: i64,
    /// Change in source anchors without an overlapping OCR line.
    pub missing_lines: i64,
    /// Union of scripts in both stages, including hypothesis-only introductions.
    pub by_script: BTreeMap<String, ScriptDelta>,
}

/// Per-script support and error deltas from the full-stream alignments.
#[derive(Debug, Serialize)]
pub struct ScriptDelta {
    /// Shared exact-scalar reference support; zero is not measured accuracy.
    pub reference_characters: usize,
    /// Shared foreign-containing reference token support, possibly zero.
    pub reference_foreign_words: usize,
    /// Change in substitutions + deletions + insertions attributed to this script.
    pub character_errors: i64,
    /// Change in substitutions from this reference script to a different script.
    pub wrong_script_substitutions: i64,
    /// Change in reference foreign tokens without an exact match.
    pub missed_foreign_words: i64,
}

/// Reads and evaluates all stages atomically from the caller's perspective.
///
/// A missing input or inconsistent identity fails the comparison instead of
/// silently omitting a stage. Paths resolve against the manifest directory.
/// Every fingerprint is computed from the same bytes passed to the parser.
pub fn compare_manifest(path: &Path) -> Result<StageComparison> {
    let (bytes, manifest_fingerprint) = read_input(path)?;
    let manifest: StageComparisonManifest =
        serde_json::from_slice(&bytes).context("invalid stage comparison manifest")?;
    let mut names = BTreeSet::new();
    for stage in &manifest.stages {
        if stage.name.trim().is_empty() || !names.insert(stage.name.as_str()) {
            bail!("stage names must be non-empty and unique");
        }
    }
    if manifest.stages.len() < 2 || !names.contains(manifest.baseline.as_str()) {
        bail!("stage comparison requires at least two stages and a named baseline among them");
    }
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let (bytes, gold) = read_input(&root.join(&manifest.gold))?;
    let benchmark: GoldBenchmark =
        serde_json::from_slice(&bytes).context("invalid gold fixture")?;
    benchmark.validate()?;
    let mut evaluated = Vec::new();
    for stage in &manifest.stages {
        let evaluated_stage = (|| -> Result<_> {
            let (bytes, identity_fingerprint) = read_input(&root.join(&stage.hypothesis_identity))?;
            let identity: SourceIdentity =
                serde_json::from_slice(&bytes).context("invalid source identity")?;
            let (bytes, alto) = read_input(&root.join(&stage.alto))?;
            let xml = std::str::from_utf8(&bytes).context("ALTO must be UTF-8")?;
            let result =
                evaluate_alto_with_identity(&benchmark, &parse_alto(xml)?, Some(&identity))?;
            Ok((alto, identity_fingerprint, identity, result))
        })()
        .with_context(|| format!("failed to evaluate stage `{}`", stage.name))?;
        evaluated.push(evaluated_stage);
    }
    let baseline_index = manifest
        .stages
        .iter()
        .position(|stage| stage.name == manifest.baseline)
        .expect("baseline validated above");
    let deltas: Vec<_> = evaluated
        .iter()
        .enumerate()
        .map(|(index, (_, _, _, result))| {
            (
                metric_delta(&manifest.baseline, &evaluated[baseline_index].3, result),
                index.checked_sub(1).map(|previous| {
                    metric_delta(
                        &manifest.stages[previous].name,
                        &evaluated[previous].3,
                        result,
                    )
                }),
            )
        })
        .collect();
    let stages = manifest
        .stages
        .into_iter()
        .zip(evaluated)
        .zip(deltas)
        .map(
            |(
                (stage, (alto, hypothesis_identity, asserted_identity, result)),
                (baseline, previous),
            )| {
                StageScore {
                    name: stage.name,
                    alto,
                    hypothesis_identity,
                    asserted_identity,
                    result,
                    against_baseline: baseline,
                    against_previous: previous,
                }
            },
        )
        .collect();
    Ok(StageComparison {
        schema_version: 1,
        evaluator_version: env!("CARGO_PKG_VERSION").to_owned(),
        manifest: manifest_fingerprint,
        gold,
        authority: benchmark.authority,
        baseline: manifest.baseline,
        stages,
    })
}

fn read_input(path: &Path) -> Result<(Vec<u8>, InputFingerprint)> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let fingerprint = InputFingerprint {
        path: path.to_owned(),
        sha256: hex::encode(Sha256::digest(&bytes)),
    };
    Ok((bytes, fingerprint))
}

fn difference(after: usize, before: usize) -> i64 {
    after as i64 - before as i64
}

fn metric_delta(
    from_stage: &str,
    before: &BenchmarkResult,
    after: &BenchmarkResult,
) -> MetricDelta {
    let a = &before.metrics;
    let b = &after.metrics;
    // These are freshly computed results, never deserialized legacy reports.
    let aa = a
        .aligned
        .as_ref()
        .expect("evaluator emits aligned diagnostics");
    let ba = b
        .aligned
        .as_ref()
        .expect("evaluator emits aligned diagnostics");
    let an = a
        .canonical_equivalence
        .as_ref()
        .expect("evaluator emits NFC diagnostics");
    let bn = b
        .canonical_equivalence
        .as_ref()
        .expect("evaluator emits NFC diagnostics");
    let scripts: BTreeSet<_> = aa
        .characters_by_script
        .keys()
        .chain(ba.characters_by_script.keys())
        .collect();
    let by_script = scripts
        .into_iter()
        .map(|script| {
            let empty_chars = ScriptCounts::default();
            let empty_words = WordCounts::default();
            let ac = aa.characters_by_script.get(script).unwrap_or(&empty_chars);
            let bc = ba.characters_by_script.get(script).unwrap_or(&empty_chars);
            let aw = aa
                .foreign_words
                .by_script
                .get(script)
                .unwrap_or(&empty_words);
            let bw = ba
                .foreign_words
                .by_script
                .get(script)
                .unwrap_or(&empty_words);
            let errors =
                |counts: &ScriptCounts| counts.substitutions + counts.deletions + counts.insertions;
            (
                script.clone(),
                ScriptDelta {
                    reference_characters: bc.reference_characters,
                    reference_foreign_words: bw.reference_words,
                    character_errors: difference(errors(bc), errors(ac)),
                    wrong_script_substitutions: difference(
                        wrong_script_count(b, script),
                        wrong_script_count(a, script),
                    ),
                    missed_foreign_words: difference(
                        bw.reference_words - bw.exact_matches,
                        aw.reference_words - aw.exact_matches,
                    ),
                },
            )
        })
        .collect();
    MetricDelta {
        from_stage: from_stage.to_owned(),
        cer: b.cer - a.cer,
        wer: b.wer - a.wer,
        base_letter_cer: b.base_letter_cer - a.base_letter_cer,
        combining_mark_cer: b.combining_mark_cer - a.combining_mark_cer,
        nfc_cer: bn.cer - an.cer,
        nfc_wer: bn.wer - an.wer,
        missed_foreign_words: difference(
            ba.foreign_words.overall.reference_words - ba.foreign_words.overall.exact_matches,
            aa.foreign_words.overall.reference_words - aa.foreign_words.overall.exact_matches,
        ),
        missing_lines: difference(after.missing_lines.len(), before.missing_lines.len()),
        by_script,
    }
}

fn wrong_script_count(metrics: &RecognitionMetrics, script: &str) -> usize {
    metrics
        .aligned
        .as_ref()
        .expect("evaluator emits aligned diagnostics")
        .substitutions_by_script
        .get(script)
        .into_iter()
        .flat_map(|targets| targets.iter())
        .filter(|(target, _)| target.as_str() != script)
        .map(|(_, count)| count)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alto::{write_alto, AltoLine, AltoPage, AltoRegion};
    use crate::model::Point;
    use serde_json::json;

    fn write_stage(root: &Path, name: &str, lines: &[(&str, f32, f32)]) {
        let page = AltoPage {
            width: 100,
            height: 100,
            regions: vec![AltoRegion {
                id: "region".to_owned(),
                polygon: Vec::new(),
                lines: lines
                    .iter()
                    .enumerate()
                    .map(|(index, (text, x, width))| AltoLine {
                        id: format!("{name}-{index}"),
                        text: (*text).to_owned(),
                        confidence: 1.0,
                        words: Vec::new(),
                        polygon: vec![
                            Point { x: *x, y: 0.0 },
                            Point {
                                x: x + width,
                                y: 0.0,
                            },
                            Point {
                                x: x + width,
                                y: 10.0,
                            },
                            Point { x: *x, y: 10.0 },
                        ],
                    })
                    .collect(),
            }],
        };
        fs::write(
            root.join(format!("{name}.xml")),
            write_alto(&page, "test.png"),
        )
        .unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let gold = json!({
            "id": "synthetic", "edition": "fixture", "source_page": 1,
            "source_sha256": "a".repeat(64), "authority": "synthetic test, not source gold",
            "source_image": {"width": 100, "height": 100, "coordinate_frame": "test-frame"},
            "lines": [{"line_id": "gold", "text": "abc α", "source": {
                "source_page": 1, "bounds": {"x": 0, "y": 0, "width": 100, "height": 10}
            }}]
        });
        fs::write(root.join("gold.json"), gold.to_string()).unwrap();
        let identity = json!({
            "edition": "fixture", "source_page": 1, "source_sha256": "a".repeat(64),
            "coordinate_frame": "test-frame"
        });
        fs::write(root.join("identity.json"), identity.to_string()).unwrap();
        write_stage(root, "page", &[("abc a", 0.0, 100.0)]);
        write_stage(root, "fusion", &[("xbc α ܐ", 0.0, 100.0)]);
        let manifest = json!({
            "gold": "gold.json", "baseline": "page", "stages": [
                {"name": "page", "alto": "page.xml", "hypothesis_identity": "identity.json"},
                {"name": "fusion", "alto": "fusion.xml", "hypothesis_identity": "identity.json"}
            ]
        });
        fs::write(root.join("stages.json"), manifest.to_string()).unwrap();
        directory
    }

    #[test]
    fn compares_scripts_with_signed_errors_and_retains_zero_support() {
        let directory = fixture();
        let report = compare_manifest(&directory.path().join("stages.json")).unwrap();
        assert_eq!(report.stages[0].against_baseline.cer, 0.0);
        assert!(report.stages[0].against_previous.is_none());
        let change = &report.stages[1].against_baseline;
        assert!(change.cer > 0.0);
        assert_eq!(change.by_script["Latn"].character_errors, 1);
        assert_eq!(change.by_script["Grek"].character_errors, -1);
        assert_eq!(change.by_script["Grek"].wrong_script_substitutions, -1);
        assert_eq!(change.by_script["Grek"].reference_foreign_words, 1);
        assert_eq!(change.by_script["Grek"].missed_foreign_words, -1);
        assert_eq!(change.missed_foreign_words, -1);
        assert_eq!(change.by_script["Syrc"].reference_characters, 0);
        assert_eq!(change.by_script["Syrc"].reference_foreign_words, 0);
        assert_eq!(change.by_script["Syrc"].character_errors, 1);
        assert_eq!(
            report.stages[1]
                .against_previous
                .as_ref()
                .unwrap()
                .from_stage,
            "page"
        );
        for artifact in [
            &report.manifest,
            &report.gold,
            &report.stages[1].alto,
            &report.stages[1].hypothesis_identity,
        ] {
            assert_eq!(
                artifact.sha256,
                crate::source::sha256_file(&artifact.path).unwrap()
            );
        }
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            serde_json::to_string(
                &compare_manifest(&directory.path().join("stages.json")).unwrap()
            )
            .unwrap()
        );
    }

    #[test]
    fn preserves_segmentation_evidence_for_identical_text_and_scores_missing_lines() {
        let directory = fixture();
        let root = directory.path();
        write_stage(root, "page", &[("abc α", 0.0, 100.0)]);
        write_stage(root, "fusion", &[("abc", 0.0, 50.0), ("α", 50.0, 50.0)]);
        let report = compare_manifest(&root.join("stages.json")).unwrap();
        assert_eq!(report.stages[1].against_baseline.cer, 0.0);
        assert_eq!(
            report.stages[1]
                .result
                .line_segmentation
                .as_ref()
                .unwrap()
                .split_candidates,
            vec!["gold"]
        );
        write_stage(root, "fusion", &[]);
        let missing = compare_manifest(&root.join("stages.json")).unwrap();
        assert_eq!(missing.stages[1].against_baseline.missing_lines, 1);
        assert_eq!(missing.stages[1].against_baseline.wer, 1.0);
    }

    #[test]
    fn distinguishes_nfc_gains_from_diplomatic_changes_and_baseline_from_previous() {
        let directory = fixture();
        let root = directory.path();
        let path = root.join("gold.json");
        let mut gold: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        gold["lines"][0]["text"] = json!("ἄ");
        fs::write(path, gold.to_string()).unwrap();
        write_stage(root, "page", &[("α", 0.0, 100.0)]);
        write_stage(root, "fusion", &[("α\u{313}\u{301}", 0.0, 100.0)]);
        let path = root.join("stages.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["baseline"] = json!("fusion");
        fs::write(&path, manifest.to_string()).unwrap();
        let report = compare_manifest(&path).unwrap();
        assert_eq!(report.stages[1].against_baseline.cer, 0.0);
        assert_eq!(report.stages[0].against_baseline.from_stage, "fusion");
        let previous = report.stages[1].against_previous.as_ref().unwrap();
        assert!(previous.cer > 0.0);
        assert_eq!(previous.nfc_cer, -1.0);
        assert_eq!(previous.combining_mark_cer, -1.0);
    }

    #[test]
    fn rejects_incomplete_or_ambiguous_manifests() {
        let directory = fixture();
        let path = directory.path().join("stages.json");
        let valid: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for case in 0..6 {
            let mut invalid = valid.clone();
            match case {
                0 => invalid["baseline"] = json!("missing"),
                1 => invalid["stages"][1]["name"] = json!("page"),
                2 => invalid["stages"][0]["name"] = json!(" "),
                3 => {
                    invalid["stages"].as_array_mut().unwrap().pop();
                }
                4 => {
                    invalid["stages"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("hypothesis_identity");
                }
                _ => invalid["stgaes"] = json!([]),
            }
            fs::write(&path, invalid.to_string()).unwrap();
            assert!(
                compare_manifest(&path).is_err(),
                "accepted invalid case {case}"
            );
        }
    }

    #[test]
    fn legacy_comparisons_keep_line_id_alignment_and_unmeasured_segmentation() {
        let directory = fixture();
        let root = directory.path();
        let path = root.join("gold.json");
        let mut gold: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        gold.as_object_mut().unwrap().remove("source_image");
        gold["lines"][0].as_object_mut().unwrap().remove("source");
        gold["lines"][0]["line_id"] = json!("page-0");
        fs::write(path, gold.to_string()).unwrap();
        let report = compare_manifest(&root.join("stages.json")).unwrap();
        assert_eq!(
            report.stages[0].result.alignment,
            super::super::AlignmentMethod::LegacyLineIds
        );
        assert_eq!(
            report.stages[0].result.source_identity,
            super::super::SourceIdentityVerification::Verified
        );
        assert!(report
            .stages
            .iter()
            .all(|stage| stage.result.line_segmentation.is_none()));
        // Renumbering is still a missing line under the explicitly legacy contract.
        assert_eq!(report.stages[1].against_baseline.missing_lines, 1);
    }

    #[test]
    fn refuses_partial_reports_and_wrong_stage_identity_or_dimensions() {
        let directory = fixture();
        let root = directory.path();
        let path = root.join("stages.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["stages"][1]["hypothesis_identity"] = json!("other.json");
        fs::write(&path, manifest.to_string()).unwrap();
        let identity: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("identity.json")).unwrap()).unwrap();
        for (field, value) in [
            ("edition", json!("other")),
            ("source_page", json!(2)),
            ("source_sha256", json!("b".repeat(64))),
            ("coordinate_frame", json!("deskewed")),
        ] {
            let mut wrong = identity.clone();
            wrong[field] = value;
            fs::write(root.join("other.json"), wrong.to_string()).unwrap();
            let error = compare_manifest(&path).unwrap_err();
            assert!(format!("{error:#}").contains("stage `fusion`"));
            assert!(format!("{error:#}").contains("does not match"));
        }
        fs::write(root.join("other.json"), identity.to_string()).unwrap();
        let alto_path = root.join("fusion.xml");
        let xml = fs::read_to_string(&alto_path).unwrap();
        fs::write(&alto_path, xml.replace("WIDTH=\"100\"", "WIDTH=\"99\"")).unwrap();
        assert!(format!("{:#}", compare_manifest(&path).unwrap_err()).contains("dimensions"));
        fs::remove_file(alto_path).unwrap();
        assert!(format!("{:#}", compare_manifest(&path).unwrap_err()).contains("failed to read"));
    }
}
