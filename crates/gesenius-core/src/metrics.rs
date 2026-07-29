//! OCR, layout, and entry-boundary evaluation metrics.

use crate::model::Point;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// Reference character count.
    pub reference_characters: usize,
    /// Reference word count.
    pub reference_words: usize,
}

/// Precision/recall pair.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrecisionRecall {
    /// True positives divided by predicted positives.
    pub precision: f64,
    /// True positives divided by expected positives.
    pub recall: f64,
}

/// Calculates Unicode-scalar CER, whitespace-token WER, and per-script CER.
#[must_use]
pub fn recognition_metrics(reference: &str, hypothesis: &str) -> RecognitionMetrics {
    let reference_characters: Vec<char> = reference.chars().collect();
    let hypothesis_characters: Vec<char> = hypothesis.chars().collect();
    let reference_words: Vec<&str> = reference.split_whitespace().collect();
    let hypothesis_words: Vec<&str> = hypothesis.split_whitespace().collect();
    let mut cer_by_script = BTreeMap::new();
    for (code, script) in [
        ("Hebr", Script::Hebrew),
        ("Arab", Script::Arabic),
        ("Syrc", Script::Syriac),
        ("Grek", Script::Greek),
        ("Latn", Script::Latin),
    ] {
        let reference_script: Vec<char> = reference_characters
            .iter()
            .copied()
            .filter(|character| character.script() == script)
            .collect();
        let hypothesis_script: Vec<char> = hypothesis_characters
            .iter()
            .copied()
            .filter(|character| character.script() == script)
            .collect();
        if !reference_script.is_empty() {
            cer_by_script.insert(
                code.to_owned(),
                rate(
                    edit_distance(&reference_script, &hypothesis_script),
                    reference_script.len(),
                ),
            );
        }
    }

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
        reference_characters: reference_characters.len(),
        reference_words: reference_words.len(),
    }
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
