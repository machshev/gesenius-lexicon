//! Script and foreign-token diagnostics aligned before filtering by script.

use super::alignment::align;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::char::is_combining_mark;
use unicode_script::{Script, UnicodeScript};

/// Full-stream exact-scalar and whitespace-token alignment diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignedDiagnostics {
    /// Reproducible algorithm/tie policy; see `docs/ocr-metric-policy.md`.
    pub alignment_policy: String,
    /// Counts over exact scalars, including spaces, punctuation and orphan marks.
    pub characters_by_script: BTreeMap<String, ScriptCounts>,
    /// Substituted scalars counted by reference script, then hypothesis script.
    /// Includes same-script substitutions; matches and gaps are reported separately.
    pub substitutions_by_script: BTreeMap<String, BTreeMap<String, usize>>,
    /// Whole-token exactness for whitespace tokens containing non-Latin letters.
    pub foreign_words: ForeignWordMetrics,
}

/// Counts for one script in a full-text scalar alignment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptCounts {
    /// Scalars bearing this script in the reference, with inherited mark attribution.
    pub reference_characters: usize,
    /// Scalars bearing this script in the hypothesis, including introduced scripts.
    pub hypothesis_characters: usize,
    /// Equal aligned scalars credited to their reference script.
    pub matches: usize,
    /// Unequal aligned scalars charged to their reference script.
    pub substitutions: usize,
    /// Missing reference scalars charged to their reference script.
    pub deletions: usize,
    /// Unpaired hypothesis scalars charged to their hypothesis script.
    pub insertions: usize,
}

/// Whole-token support and exact matches, without a rate for absent reference data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WordCounts {
    /// Reference whitespace tokens containing a qualifying script letter.
    pub reference_words: usize,
    /// Hypothesis whitespace tokens containing a qualifying script letter.
    pub hypothesis_words: usize,
    /// Exact token matches in the complete, unfiltered word alignment.
    pub exact_matches: usize,
    /// Exact matches / reference words, or null if no reference words exist.
    pub accuracy: Option<f64>,
    /// Exact matches / hypothesis words, or null if no hypothesis words exist.
    pub precision: Option<f64>,
}

impl WordCounts {
    fn finish(&mut self) {
        self.accuracy = (self.reference_words > 0)
            .then(|| self.exact_matches as f64 / self.reference_words as f64);
        self.precision = (self.hypothesis_words > 0)
            .then(|| self.exact_matches as f64 / self.hypothesis_words as f64);
    }
}

/// Exactness of foreign-script-containing whitespace tokens, not dictionary words.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ForeignWordMetrics {
    /// Each foreign-containing token counted once, including mixed-script tokens.
    pub overall: WordCounts,
    /// A mixed-script token contributes once to each non-Latin script it contains.
    pub by_script: BTreeMap<String, WordCounts>,
}

pub(super) fn diagnostics(reference: &str, hypothesis: &str) -> AlignedDiagnostics {
    let left: Vec<_> = reference.chars().collect();
    let right: Vec<_> = hypothesis.chars().collect();
    let left_scripts = scalar_scripts(&left);
    let right_scripts = scalar_scripts(&right);
    let mut counts: BTreeMap<String, ScriptCounts> = BTreeMap::new();
    for script in &left_scripts {
        counts
            .entry(script.clone())
            .or_default()
            .reference_characters += 1;
    }
    for script in &right_scripts {
        counts
            .entry(script.clone())
            .or_default()
            .hypothesis_characters += 1;
    }
    let mut substitutions: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for (a, b) in align(&left, &right) {
        match (a, b) {
            (Some(a), Some(b)) if left[a] == right[b] => {
                counts.entry(left_scripts[a].clone()).or_default().matches += 1
            }
            (Some(a), Some(b)) => {
                counts
                    .entry(left_scripts[a].clone())
                    .or_default()
                    .substitutions += 1;
                *substitutions
                    .entry(left_scripts[a].clone())
                    .or_default()
                    .entry(right_scripts[b].clone())
                    .or_default() += 1;
            }
            (Some(a), None) => counts.entry(left_scripts[a].clone()).or_default().deletions += 1,
            (None, Some(b)) => {
                counts
                    .entry(right_scripts[b].clone())
                    .or_default()
                    .insertions += 1
            }
            (None, None) => unreachable!("alignment cannot pair two gaps"),
        }
    }
    AlignedDiagnostics {
        alignment_policy: "levenshtein_hirschberg_v1".to_owned(),
        characters_by_script: counts,
        substitutions_by_script: substitutions,
        foreign_words: foreign_words(reference, hypothesis),
    }
}

fn scalar_scripts(characters: &[char]) -> Vec<String> {
    let mut previous = None;
    characters
        .iter()
        .map(|&character| {
            let own = character.script();
            let script = if is_combining_mark(character) {
                previous.unwrap_or(own)
            } else {
                previous = Some(own);
                own
            };
            script.short_name().to_owned()
        })
        .collect()
}

fn foreign_scripts(word: &str) -> BTreeSet<String> {
    word.chars()
        .filter(|&character| character.is_alphabetic() && !is_combining_mark(character))
        .map(|character| character.script())
        .filter(|script| {
            !matches!(
                script,
                Script::Latin | Script::Common | Script::Inherited | Script::Unknown
            )
        })
        .map(|script| script.short_name().to_owned())
        .collect()
}

fn foreign_words(reference: &str, hypothesis: &str) -> ForeignWordMetrics {
    let left: Vec<_> = reference.split_whitespace().collect();
    let right: Vec<_> = hypothesis.split_whitespace().collect();
    let left_scripts: Vec<_> = left.iter().map(|word| foreign_scripts(word)).collect();
    let right_scripts: Vec<_> = right.iter().map(|word| foreign_scripts(word)).collect();
    let mut result = ForeignWordMetrics::default();
    for scripts in &left_scripts {
        result.overall.reference_words += usize::from(!scripts.is_empty());
        for script in scripts {
            result
                .by_script
                .entry(script.clone())
                .or_default()
                .reference_words += 1;
        }
    }
    for scripts in &right_scripts {
        result.overall.hypothesis_words += usize::from(!scripts.is_empty());
        for script in scripts {
            result
                .by_script
                .entry(script.clone())
                .or_default()
                .hypothesis_words += 1;
        }
    }
    for (a, b) in align(&left, &right) {
        if let (Some(a), Some(b)) = (a, b) {
            if left[a] == right[b] && !left_scripts[a].is_empty() {
                result.overall.exact_matches += 1;
                for script in &left_scripts[a] {
                    result
                        .by_script
                        .entry(script.clone())
                        .or_default()
                        .exact_matches += 1;
                }
            }
        }
    }
    result.overall.finish();
    for counts in result.by_script.values_mut() {
        counts.finish();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_scripts_are_visible_even_when_filtered_streams_match() {
        let result = diagnostics("α a", "a α");
        assert_eq!(result.substitutions_by_script["Grek"]["Latn"], 1);
        assert_eq!(result.substitutions_by_script["Latn"]["Grek"], 1);
        assert_eq!(result.foreign_words.overall.exact_matches, 0);
        assert_eq!(result.foreign_words.overall.accuracy, Some(0.0));
    }

    #[test]
    fn marks_rare_scripts_and_extra_scripts_remain_counted() {
        let result = diagnostics("אֶ 𐤀 ሀ", "א A ሀ ܐ");
        assert_eq!(result.characters_by_script["Hebr"].reference_characters, 2);
        assert_eq!(result.characters_by_script["Hebr"].deletions, 1);
        assert_eq!(result.substitutions_by_script["Phnx"]["Latn"], 1);
        assert_eq!(result.characters_by_script["Ethi"].matches, 1);
        assert_eq!(result.foreign_words.by_script["Syrc"].reference_words, 0);
        assert_eq!(result.foreign_words.by_script["Syrc"].accuracy, None);
        assert_eq!(result.foreign_words.by_script["Syrc"].precision, Some(0.0));
    }

    #[test]
    fn exact_foreign_tokens_include_points_punctuation_and_mixed_scripts() {
        let result = diagnostics("אָב, αܐ", "אב, αܐ");
        assert_eq!(result.foreign_words.overall.reference_words, 2);
        assert_eq!(result.foreign_words.overall.exact_matches, 1);
        assert_eq!(result.foreign_words.by_script["Grek"].exact_matches, 1);
        assert_eq!(result.foreign_words.by_script["Syrc"].exact_matches, 1);
        assert_eq!(result.foreign_words.by_script["Hebr"].accuracy, Some(0.0));
        assert_eq!(
            diagnostics("אָב,", "אָב").foreign_words.overall.accuracy,
            Some(0.0)
        );
        assert_eq!(
            diagnostics("English only", "English only")
                .foreign_words
                .overall
                .accuracy,
            None
        );
    }

    #[test]
    fn insertions_and_deletions_conserve_global_character_counts() {
        for (reference, hypothesis) in [
            ("אב", "א"),
            ("א", "אב"),
            ("", "ܐ"),
            ("a.\u{301}", "a"),
            ("\u{301}", ""),
        ] {
            let result = diagnostics(reference, hypothesis);
            let counts = result.characters_by_script.values();
            assert_eq!(
                counts
                    .clone()
                    .map(|c| c.reference_characters)
                    .sum::<usize>(),
                reference.chars().count()
            );
            assert_eq!(
                counts
                    .clone()
                    .map(|c| c.hypothesis_characters)
                    .sum::<usize>(),
                hypothesis.chars().count()
            );
            assert_eq!(
                counts
                    .clone()
                    .map(|c| c.matches + c.substitutions + c.deletions)
                    .sum::<usize>(),
                reference.chars().count()
            );
            assert_eq!(
                counts
                    .clone()
                    .map(|c| c.matches + c.substitutions + c.insertions)
                    .sum::<usize>(),
                hypothesis.chars().count()
            );
        }
        assert_eq!(
            diagnostics("\u{301}", "").characters_by_script["Zinh"].deletions,
            1
        );
    }

    #[test]
    fn false_foreign_introduction_and_missing_foreign_tokens_affect_precision_recall() {
        let extra = diagnostics("word", "word α").foreign_words;
        assert_eq!(extra.overall.accuracy, None);
        assert_eq!(extra.overall.precision, Some(0.0));
        let missing = diagnostics("α word", "word").foreign_words;
        assert_eq!(missing.overall.accuracy, Some(0.0));
        assert_eq!(missing.overall.precision, None);
    }
}
