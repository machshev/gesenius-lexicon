//! Deterministic minimum-edit alignment with linear working memory.

/// Paired original indices; `None` represents an insertion or deletion.
pub(super) type Pair = (Option<usize>, Option<usize>);

pub(super) fn align<T: PartialEq>(reference: &[T], hypothesis: &[T]) -> Vec<Pair> {
    let mut pairs = Vec::with_capacity(reference.len() + hypothesis.len());
    divide(reference, hypothesis, 0, 0, &mut pairs);
    pairs
}

fn divide<T: PartialEq>(
    reference: &[T],
    hypothesis: &[T],
    reference_offset: usize,
    hypothesis_offset: usize,
    pairs: &mut Vec<Pair>,
) {
    if reference.is_empty() {
        pairs.extend((0..hypothesis.len()).map(|index| (None, Some(hypothesis_offset + index))));
    } else if hypothesis.is_empty() {
        pairs.extend((0..reference.len()).map(|index| (Some(reference_offset + index), None)));
    } else if reference.len() == 1 || hypothesis.len() == 1 {
        // At most two rows or columns, so this traceback table remains linear.
        let width = hypothesis.len() + 1;
        let mut costs = vec![0; (reference.len() + 1) * width];
        for i in 0..=reference.len() {
            costs[i * width] = i;
        }
        for (j, cost) in costs.iter_mut().take(width).enumerate() {
            *cost = j;
        }
        for i in 1..=reference.len() {
            for j in 1..=hypothesis.len() {
                costs[i * width + j] = (costs[(i - 1) * width + j - 1]
                    + usize::from(reference[i - 1] != hypothesis[j - 1]))
                .min(costs[(i - 1) * width + j] + 1)
                .min(costs[i * width + j - 1] + 1);
            }
        }
        let (mut i, mut j) = (reference.len(), hypothesis.len());
        let start = pairs.len();
        while i > 0 || j > 0 {
            if i > 0
                && j > 0
                && costs[i * width + j]
                    == costs[(i - 1) * width + j - 1]
                        + usize::from(reference[i - 1] != hypothesis[j - 1])
            {
                i -= 1;
                j -= 1;
                pairs.push((Some(reference_offset + i), Some(hypothesis_offset + j)));
            } else if i > 0 && costs[i * width + j] == costs[(i - 1) * width + j] + 1 {
                i -= 1;
                pairs.push((Some(reference_offset + i), None));
            } else {
                j -= 1;
                pairs.push((None, Some(hypothesis_offset + j)));
            }
        }
        pairs[start..].reverse();
    } else {
        let middle = reference.len() / 2;
        let split = {
            let forward = cost_row(&reference[..middle], hypothesis, false);
            let backward = cost_row(&reference[middle..], hypothesis, true);
            // First minimum fixes ambiguous alignments reproducibly. Release both
            // rows before recursing so retained working storage stays linear.
            (0..=hypothesis.len())
                .min_by_key(|&j| forward[j] + backward[hypothesis.len() - j])
                .expect("there is always a split at zero")
        };
        divide(
            &reference[..middle],
            &hypothesis[..split],
            reference_offset,
            hypothesis_offset,
            pairs,
        );
        divide(
            &reference[middle..],
            &hypothesis[split..],
            reference_offset + middle,
            hypothesis_offset + split,
            pairs,
        );
    }
}

fn cost_row<T: PartialEq>(reference: &[T], hypothesis: &[T], reverse: bool) -> Vec<usize> {
    let mut previous: Vec<_> = (0..=hypothesis.len()).collect();
    let mut current = vec![0; hypothesis.len() + 1];
    for i in 0..reference.len() {
        current[0] = i + 1;
        let left = &reference[if reverse { reference.len() - 1 - i } else { i }];
        for j in 0..hypothesis.len() {
            let right = &hypothesis[if reverse { hypothesis.len() - 1 - j } else { j }];
            current[j + 1] = (previous[j] + usize::from(left != right))
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut current, &mut previous);
    }
    previous
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::edit_distance;

    #[test]
    fn alignments_preserve_every_item_and_minimum_cost_exhaustively() {
        let mut inputs = vec![Vec::new()];
        for length in 1..=4 {
            for bits in 0..2_usize.pow(length) {
                inputs.push((0..length).map(|shift| (bits >> shift) & 1).collect());
            }
        }
        for reference in &inputs {
            for hypothesis in &inputs {
                let pairs = align(reference, hypothesis);
                assert_eq!(
                    pairs.iter().filter_map(|pair| pair.0).collect::<Vec<_>>(),
                    (0..reference.len()).collect::<Vec<_>>()
                );
                assert_eq!(
                    pairs.iter().filter_map(|pair| pair.1).collect::<Vec<_>>(),
                    (0..hypothesis.len()).collect::<Vec<_>>()
                );
                let errors = pairs
                    .iter()
                    .filter(|(a, b)| match (a, b) {
                        (Some(a), Some(b)) => reference[*a] != hypothesis[*b],
                        _ => true,
                    })
                    .count();
                assert_eq!(errors, edit_distance(reference, hypothesis));
            }
        }
    }

    #[test]
    fn asymmetric_inputs_and_ties_are_deterministic() {
        assert_eq!(
            align(&['a'], &['a', 'a']),
            vec![(None, Some(0)), (Some(0), Some(1))]
        );
        assert_eq!(
            align(&['a', 'a'], &['a', 'a', 'a']),
            vec![(Some(0), Some(0)), (None, Some(1)), (Some(1), Some(2))]
        );
        assert_eq!(align(&vec!['a'; 4096], &['b']).len(), 4096);
        assert_eq!(align(&['a'], &vec!['b'; 4096]).len(), 4096);
    }
}
