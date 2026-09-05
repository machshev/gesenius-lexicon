//! OCR, layout, and entry-boundary evaluation metrics.

use crate::model::Point;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
use unicode_script::{Script, UnicodeScript};

/// OCR error rates overall and per supported script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecognitionMetrics {
    /// Character edit distance divided by reference characters.
    pub cer: f64,
    /// Word edit distance divided by reference words.
    pub wer: f64,
    /// Character error rate per ISO 15924 script.
    pub cer_by_script: BTreeMap<String, f64>,
    /// Reference scalars contributing to each script diagnostic.
    ///
    /// Combining marks inherit the preceding base character's script.
    pub reference_characters_by_script: BTreeMap<String, usize>,
    /// Reference character count.
    pub reference_characters: usize,
    /// Reference word count.
    pub reference_words: usize,
    /// Canonically decomposed reference alphabetic base characters used by the diagnostic.
    pub reference_base_letters: usize,
    /// Canonically decomposed reference combining marks used by the diagnostic.
    pub reference_combining_marks: usize,
    /// Canonical-decomposition CER over alphabetic base characters only.
    ///
    /// This is diagnostic and does not replace the exact-scalar `cer`.
    pub base_letter_cer: f64,
    /// Canonical-decomposition CER over combining marks associated with aligned bases.
    ///
    /// This is diagnostic and does not replace the exact-scalar `cer`.
    pub combining_mark_cer: f64,
}

/// Precision/recall pair.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrecisionRecall {
    /// True positives divided by predicted positives.
    pub precision: f64,
    /// True positives divided by expected positives.
    pub recall: f64,
}

/// Calculates exact Unicode-scalar CER, whitespace-token WER, and diagnostics.
///
/// The base and combining-mark diagnostics use canonical decomposition so a
/// precomposed character and its decomposed equivalent do not look like an OCR
/// error there. `cer` remains the diplomatic, exact-scalar score.
#[must_use]
pub fn recognition_metrics(reference: &str, hypothesis: &str) -> RecognitionMetrics {
    let reference_characters: Vec<char> = reference.chars().collect();
    let hypothesis_characters: Vec<char> = hypothesis.chars().collect();
    let reference_words: Vec<&str> = reference.split_whitespace().collect();
    let hypothesis_words: Vec<&str> = hypothesis.split_whitespace().collect();
    let mut cer_by_script = BTreeMap::new();
    let mut reference_characters_by_script = BTreeMap::new();
    for script in observed_scripts(reference) {
        let code = script.short_name();
        let reference_script = script_scalars(reference, script);
        let hypothesis_script = script_scalars(hypothesis, script);
        if !reference_script.is_empty() {
            reference_characters_by_script.insert(code.to_owned(), reference_script.len());
            cer_by_script.insert(
                code.to_owned(),
                rate(
                    edit_distance(&reference_script, &hypothesis_script),
                    reference_script.len(),
                ),
            );
        }
    }

    let reference_clusters = base_clusters(reference);
    let hypothesis_clusters = base_clusters(hypothesis);
    let reference_bases: Vec<_> = reference_clusters
        .iter()
        .map(|cluster| cluster.base)
        .collect();
    let hypothesis_bases: Vec<_> = hypothesis_clusters
        .iter()
        .map(|cluster| cluster.base)
        .collect();
    let reference_marks = reference_clusters
        .iter()
        .map(|cluster| cluster.marks.len())
        .sum();

    RecognitionMetrics {
        cer: rate(
            edit_distance(&reference_characters, &hypothesis_characters),
            reference_characters.len(),
        ),
        wer: rate(
            edit_distance(&reference_words, &hypothesis_words),
            reference_words.len(),
        ),
        cer_by_script,
        reference_characters_by_script,
        reference_characters: reference_characters.len(),
        reference_words: reference_words.len(),
        reference_base_letters: reference_bases.len(),
        reference_combining_marks: reference_marks,
        base_letter_cer: rate(
            edit_distance(&reference_bases, &hypothesis_bases),
            reference_bases.len(),
        ),
        combining_mark_cer: rate(
            aligned_mark_errors(&reference_clusters, &hypothesis_clusters),
            reference_marks,
        ),
    }
}

fn observed_scripts(text: &str) -> Vec<Script> {
    let mut scripts = BTreeMap::new();
    for script in text
        .chars()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| character.script())
        .filter(|script| !matches!(script, Script::Common | Script::Inherited | Script::Unknown))
    {
        scripts.insert(script.short_name(), script);
    }
    scripts.into_values().collect()
}

fn script_scalars(text: &str, target: Script) -> Vec<char> {
    let mut inherited_script = None;
    text.chars()
        .filter(|character| {
            if is_combining_mark(*character) {
                inherited_script == Some(target)
            } else {
                inherited_script = Some(character.script());
                character.script() == target
            }
        })
        .collect()
}

#[derive(Debug)]
struct BaseCluster {
    base: char,
    marks: Vec<char>,
}

fn base_clusters(text: &str) -> Vec<BaseCluster> {
    let mut clusters: Vec<BaseCluster> = Vec::new();
    let mut current_base: Option<usize> = None;
    for character in text.nfd() {
        if is_combining_mark(character) {
            if let Some(index) = current_base {
                clusters[index].marks.push(character);
            }
        } else if character.is_alphabetic() {
            clusters.push(BaseCluster {
                base: character,
                marks: Vec::new(),
            });
            current_base = Some(clusters.len() - 1);
        } else {
            current_base = None;
        }
    }
    clusters
}

fn aligned_mark_errors(reference: &[BaseCluster], hypothesis: &[BaseCluster]) -> usize {
    // Each cell carries base edit distance and mark errors. On a tie we select
    // substitution, deletion, then insertion, matching the former traceback.
    let mut previous = vec![(0, 0); hypothesis.len() + 1];
    for (index, cluster) in hypothesis.iter().enumerate() {
        previous[index + 1] = (
            previous[index].0 + 1,
            previous[index].1 + cluster.marks.len(),
        );
    }
    for reference_cluster in reference {
        let mut current = vec![(0, 0); hypothesis.len() + 1];
        current[0] = (
            previous[0].0 + 1,
            previous[0].1 + reference_cluster.marks.len(),
        );
        for (hypothesis_index, hypothesis_cluster) in hypothesis.iter().enumerate() {
            let substitution = (
                previous[hypothesis_index].0
                    + usize::from(reference_cluster.base != hypothesis_cluster.base),
                previous[hypothesis_index].1
                    + edit_distance(&reference_cluster.marks, &hypothesis_cluster.marks),
            );
            let deletion = (
                previous[hypothesis_index + 1].0 + 1,
                previous[hypothesis_index + 1].1 + reference_cluster.marks.len(),
            );
            let insertion = (
                current[hypothesis_index].0 + 1,
                current[hypothesis_index].1 + hypothesis_cluster.marks.len(),
            );
            current[hypothesis_index + 1] = [substitution, deletion, insertion]
                .into_iter()
                .min_by_key(|cost| cost.0)
                .expect("three edit operations are always available");
        }
        previous = current;
    }
    previous[hypothesis.len()].1
}

/// Normalized character disagreement, capped at 1.
#[must_use]
pub fn normalized_disagreement(left: &str, right: &str) -> f64 {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let denominator = left.len().max(right.len());
    rate(edit_distance(&left, &right), denominator).min(1.0)
}

/// Computes boundary precision and recall with exact source-line boundary IDs.
#[must_use]
pub fn boundary_precision_recall(expected: &[String], predicted: &[String]) -> PrecisionRecall {
    let true_positives = predicted
        .iter()
        .filter(|boundary| expected.contains(boundary))
        .count();
    PrecisionRecall {
        precision: rate_complement(
            predicted.len().saturating_sub(true_positives),
            predicted.len(),
        ),
        recall: rate_complement(
            expected.len().saturating_sub(true_positives),
            expected.len(),
        ),
    }
}

/// Calculates axis-aligned intersection-over-union for two polygons.
#[must_use]
pub fn polygon_iou(left: &[Point], right: &[Point]) -> f64 {
    let Some((left_x1, left_y1, left_x2, left_y2)) = bounds(left) else {
        return 0.0;
    };
    let Some((right_x1, right_y1, right_x2, right_y2)) = bounds(right) else {
        return 0.0;
    };
    let intersection_width = (left_x2.min(right_x2) - left_x1.max(right_x1)).max(0.0);
    let intersection_height = (left_y2.min(right_y2) - left_y1.max(right_y1)).max(0.0);
    let intersection = intersection_width * intersection_height;
    let left_area = (left_x2 - left_x1) * (left_y2 - left_y1);
    let right_area = (right_x2 - right_x1) * (right_y2 - right_y1);
    let union = left_area + right_area - intersection;
    if union <= f32::EPSILON {
        0.0
    } else {
        f64::from(intersection / union)
    }
}

fn edit_distance<T: PartialEq>(left: &[T], right: &[T]) -> usize {
    if left.is_empty() {
        return right.len();
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_item) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_item) in right.iter().enumerate() {
            let substitution = previous[right_index] + usize::from(left_item != right_item);
            current[right_index + 1] = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn rate(errors: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        f64::from(errors != 0)
    } else {
        errors as f64 / denominator as f64
    }
}

fn rate_complement(errors: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        1.0 - errors as f64 / denominator as f64
    }
}

fn bounds(points: &[Point]) -> Option<(f32, f32, f32, f32)> {
    let first = points.first()?;
    Some(points.iter().skip(1).fold(
        (first.x, first.y, first.x, first.y),
        |(min_x, min_y, max_x, max_y), point| {
            (
                min_x.min(point.x),
                min_y.min(point.y),
                max_x.max(point.x),
                max_y.max(point.y),
            )
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{boundary_precision_recall, normalized_disagreement, recognition_metrics};

    #[test]
    fn exact_text_has_zero_error() {
        let metrics = recognition_metrics("אָב father", "אָב father");
        assert_eq!(metrics.cer, 0.0);
        assert_eq!(metrics.wer, 0.0);
        assert_eq!(metrics.cer_by_script["Hebr"], 0.0);
        assert_eq!(metrics.reference_characters_by_script["Hebr"], 3);
    }

    #[test]
    fn diagnostics_count_pointing_and_attribute_it_to_its_base_script() {
        let metrics = recognition_metrics("אֶ", "א");
        assert_eq!(metrics.reference_characters_by_script["Hebr"], 2);
        assert_eq!(metrics.reference_base_letters, 1);
        assert_eq!(metrics.reference_combining_marks, 1);
        assert_eq!(metrics.base_letter_cer, 0.0);
        assert_eq!(metrics.combining_mark_cer, 1.0);
        assert_eq!(metrics.cer_by_script["Hebr"], 0.5);
    }

    #[test]
    fn diagnostics_treat_canonical_composition_as_equivalent() {
        let metrics = recognition_metrics("é", "e\u{301}");
        assert!(metrics.cer > 0.0);
        assert_eq!(metrics.base_letter_cer, 0.0);
        assert_eq!(metrics.combining_mark_cer, 0.0);
    }

    #[test]
    fn mark_diagnostic_keeps_marks_with_their_base_letters() {
        let metrics = recognition_metrics("אָב", "אבָ");
        assert_eq!(metrics.base_letter_cer, 0.0);
        assert_eq!(metrics.combining_mark_cer, 2.0);
    }

    #[test]
    fn orphan_marks_do_not_attach_across_nonletters() {
        for separator in [' ', '.', '1'] {
            let reference = format!("a{separator}\u{301}");
            let metrics = recognition_metrics(&reference, "á");
            assert_eq!(metrics.reference_combining_marks, 0);
            assert!(metrics.combining_mark_cer > 0.0);
            assert!(metrics.cer > 0.0);
        }
    }

    #[test]
    fn dynamic_script_samples_show_wrong_script_substitution() {
        let metrics = recognition_metrics("𐤀 α", "A a");
        assert_eq!(metrics.reference_characters_by_script["Phnx"], 1);
        assert_eq!(metrics.reference_characters_by_script["Grek"], 1);
        assert_eq!(metrics.cer_by_script["Phnx"], 1.0);
        assert_eq!(metrics.cer_by_script["Grek"], 1.0);
    }

    #[test]
    fn disagreement_is_symmetric_for_equal_lengths() {
        assert_eq!(
            normalized_disagreement("אָב", "אב"),
            normalized_disagreement("אב", "אָב")
        );
    }

    #[test]
    fn boundary_metrics_count_exact_ids() {
        let result = boundary_precision_recall(
            &["l1".to_owned(), "l3".to_owned()],
            &["l1".to_owned(), "l2".to_owned()],
        );
        assert_eq!(result.precision, 0.5);
        assert_eq!(result.recall, 0.5);
    }
}
