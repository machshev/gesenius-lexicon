//! Unicode normalization, script classification, and mixed-direction checks.

use crate::language::identify_languages;
use crate::model::{Direction, TextSpan, UnicodeWarning};
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};

/// NFC-normalizes text without applying compatibility normalization.
#[must_use]
pub fn normalize_nfc(text: &str) -> String {
    text.nfc().collect()
}

/// Returns whether a scalar is a bidi formatting control that must not be stored.
#[must_use]
pub const fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// Removes OCR-inserted bidi formatting controls from otherwise logical text.
///
/// The raw engine hypothesis and ALTO remain unchanged; canonical diplomatic
/// text uses span direction metadata instead.
#[must_use]
pub fn without_bidi_controls(text: &str) -> String {
    text.chars()
        .filter(|character| !is_bidi_control(*character))
        .collect()
}

/// Classifies a span into an ISO 15924 script code.
///
/// Common punctuation and inherited combining marks do not make a span mixed.
#[must_use]
pub fn classify_script(text: &str) -> String {
    let mut scripts = text
        .chars()
        .map(|character| character.script())
        .filter(|script| !matches!(script, Script::Common | Script::Inherited | Script::Unknown));
    let Some(first) = scripts.next() else {
        return "Zyyy".to_owned();
    };
    if scripts.any(|script| script != first) {
        "Zyyy".to_owned()
    } else {
        script_code(first).to_owned()
    }
}

/// Infers semantic direction from strong script characters.
#[must_use]
pub fn infer_direction(text: &str) -> Direction {
    let mut has_ltr = false;
    let mut has_rtl = false;
    for script in text.chars().map(|character| character.script()) {
        match script {
            Script::Common | Script::Inherited | Script::Unknown => {}
            Script::Hebrew
            | Script::Arabic
            | Script::Syriac
            | Script::Samaritan
            | Script::Phoenician
            | Script::Imperial_Aramaic
            | Script::Avestan => has_rtl = true,
            _ => has_ltr = true,
        }
    }
    match (has_ltr, has_rtl) {
        (true, true) => Direction::Mixed,
        (false, true) => Direction::Rtl,
        _ => Direction::Ltr,
    }
}

/// Returns warnings for controls, replacement characters, and orphan marks.
#[must_use]
pub fn unicode_warnings(text: &str) -> Vec<UnicodeWarning> {
    let mut warnings = Vec::new();
    let mut has_base_in_word = false;
    for (character_offset, character) in text.chars().enumerate() {
        let warning = if is_bidi_control(character) {
            Some((
                "embedded_bidi_control",
                "Store logical order and span direction instead of embedded bidi controls",
            ))
        } else if character == '\u{fffd}' {
            Some(("replacement_character", "Unresolved replacement character"))
        } else if ('\u{e000}'..='\u{f8ff}').contains(&character)
            || ('\u{f0000}'..='\u{ffffd}').contains(&character)
            || ('\u{100000}'..='\u{10fffd}').contains(&character)
        {
            Some((
                "private_use_character",
                "Private-use character requires review",
            ))
        } else if canonical_combining_class(character) != 0 && !has_base_in_word {
            Some((
                "orphan_combining_mark",
                "Combining mark has no preceding base",
            ))
        } else {
            None
        };

        if let Some((code, message)) = warning {
            warnings.push(UnicodeWarning {
                code_point: format!("U+{:04X}", u32::from(character)),
                character_offset,
                code: code.to_owned(),
                message: message.to_owned(),
            });
        }

        if character.is_whitespace() {
            has_base_in_word = false;
        } else if canonical_combining_class(character) == 0 {
            has_base_in_word = true;
        }
    }
    warnings
}

/// Refreshes derived Unicode fields after a review edit.
pub fn refresh_span(span: &mut TextSpan) {
    let reviewed_language = span.language.clone();
    let reviewed_runs = span.language_runs.clone();
    let preserve_reviewed_languages = reviewed_runs
        .iter()
        .any(|run| run.evidence == crate::model::LanguageEvidence::Reviewer);
    span.normalized = normalize_nfc(&span.diplomatic);
    span.script = classify_script(&span.normalized);
    span.direction = infer_direction(&span.normalized);
    let previous_language = span.language.clone();
    let previous_runs = span.language_runs.clone();
    let default_language = span
        .language
        .as_deref()
        .filter(|language| !matches!(*language, "mul" | "zxx" | "und"))
        .unwrap_or("en");
    (span.language, span.language_runs) = identify_languages(&span.normalized, default_language);
    if preserve_reviewed_languages {
        span.language = reviewed_language;
        span.language_runs = reviewed_runs;
    }
    if span.language == previous_language {
        for run in &mut span.language_runs {
            if previous_runs.iter().any(|previous| {
                previous.language == run.language
                    && previous.script == run.script
                    && previous.evidence == crate::model::LanguageEvidence::PrintedLabel
            }) {
                run.evidence = crate::model::LanguageEvidence::PrintedLabel;
            }
        }
    }
    span.warnings = unicode_warnings(&span.diplomatic);
}

/// Computes a character-count weighted confidence.
#[must_use]
pub fn aggregate_confidence<'a>(spans: impl IntoIterator<Item = &'a TextSpan>) -> f32 {
    let (weighted, count) = spans
        .into_iter()
        .fold((0.0_f32, 0_usize), |(sum, count), span| {
            let length = span.normalized.chars().count().max(1);
            (sum + span.confidence * length as f32, count + length)
        });
    if count == 0 {
        0.0
    } else {
        weighted / count as f32
    }
}

fn script_code(script: Script) -> &'static str {
    match script {
        Script::Common | Script::Inherited | Script::Unknown => "Zyyy",
        _ => script.short_name(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_script, infer_direction, is_bidi_control, normalize_nfc, unicode_warnings,
        without_bidi_controls,
    };
    use crate::model::Direction;

    #[test]
    fn normalization_is_canonical_not_compatibility() {
        assert_eq!(normalize_nfc("\u{05e9}\u{05b8}"), "\u{05e9}\u{05b8}");
        assert_eq!(normalize_nfc("\u{2126}"), "\u{03a9}");
        // NFC does not turn a compatibility ligature into separate letters.
        assert_eq!(normalize_nfc("\u{fb03}"), "\u{fb03}");
    }

    #[test]
    fn combining_marks_remain_in_their_script_context() {
        assert_eq!(classify_script("שָׁלוֹם"), "Hebr");
        assert_eq!(infer_direction("שָׁלוֹם"), Direction::Rtl);
    }

    #[test]
    fn mixed_logical_order_is_explicit() {
        assert_eq!(infer_direction("אב Gen 1:1"), Direction::Mixed);
        assert_eq!(classify_script("אב Gen"), "Zyyy");
    }

    #[test]
    fn less_common_edition_scripts_keep_their_iso_15924_identity() {
        assert_eq!(classify_script("𐤀"), "Phnx");
        assert_eq!(classify_script("ࠀ"), "Samr");
        assert_eq!(classify_script("अ"), "Deva");
        assert_eq!(classify_script("𐬀"), "Avst");
        assert_eq!(classify_script("Ⲁ"), "Copt");
        assert_eq!(classify_script("𐌰"), "Goth");
    }

    #[test]
    fn bidi_controls_are_reported() {
        assert!(is_bidi_control('\u{200f}'));
        assert_eq!(
            unicode_warnings("a\u{200f}b")[0].code,
            "embedded_bidi_control"
        );
        assert_eq!(without_bidi_controls("אב\u{200f} Gen"), "אב Gen");
    }

    #[test]
    fn orphan_combining_marks_are_reported() {
        assert_eq!(
            unicode_warnings("\u{05b0}א")[0].code,
            "orphan_combining_mark"
        );
    }
}
