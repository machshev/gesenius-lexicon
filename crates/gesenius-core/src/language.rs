//! Edition-backed semantic language identification.

use crate::model::{LanguageEvidence, LanguageRun};
use std::collections::BTreeSet;
use unicode_script::{Script, UnicodeScript};

/// One language explicitly attested in the Robinson edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageProfile {
    /// BCP 47 language tag.
    pub tag: &'static str,
    /// Human-readable name used in documentation and review UI.
    pub name: &'static str,
    /// Scripts in which the edition prints native text or transliteration.
    pub scripts: &'static [&'static str],
    /// Normalized printed labels used by the edition.
    pub labels: &'static [&'static str],
}

/// Languages explicitly named or printed in the Robinson 1854 edition.
///
/// This catalogue records semantic languages, not OCR model names. Several
/// languages share a script and therefore share an OCR model; for example,
/// Biblical Aramaic is tagged `arc` while its square Hebrew type is read by
/// Tesseract's `heb` model.
pub const ROBINSON_1854_LANGUAGES: &[LanguageProfile] = &[
    profile("en", "English", &["Latn"], &["eng", "english"]),
    profile(
        "he",
        "Hebrew",
        &["Hebr", "Latn"],
        &["heb", "hebr", "hebrew"],
    ),
    profile(
        "arc",
        "Biblical Aramaic (Chaldee)",
        &["Hebr", "Armi", "Syrc", "Latn"],
        &["chald", "chaldee", "aram", "aramaic", "aramean"],
    ),
    profile("ar", "Arabic", &["Arab", "Latn"], &["arab", "arabic"]),
    profile("syr", "Syriac", &["Syrc", "Latn"], &["syr", "syriac"]),
    profile(
        "gez",
        "Ge'ez (Ethiopic)",
        &["Ethi", "Latn"],
        &["ethiop", "ethiopic", "aethiop", "aethiopic"],
    ),
    profile(
        "sam",
        "Samaritan Aramaic",
        &["Samr", "Hebr", "Latn"],
        &["samar", "samaritan"],
    ),
    profile(
        "phn",
        "Phoenician",
        &["Phnx", "Hebr", "Latn"],
        &["phoen", "phoenic", "phoenician", "phen", "phenician"],
    ),
    profile("grc", "Ancient Greek", &["Grek", "Latn"], &["gr", "greek"]),
    profile("la", "Latin", &["Latn"], &["lat", "latin"]),
    profile("fa", "Persian", &["Arab", "Latn"], &["pers", "persian"]),
    profile(
        "sa",
        "Sanskrit",
        &["Deva", "Latn"],
        &["sanscr", "sanscrit", "sanskrit"],
    ),
    profile(
        "ae",
        "Avestan (Zend)",
        &["Avst", "Latn"],
        &["zend", "avestan"],
    ),
    profile("got", "Gothic", &["Goth", "Latn"], &["goth", "gothic"]),
    profile("de", "German", &["Latn"], &["germ", "german"]),
    profile("fr", "French", &["Latn"], &["fr", "french"]),
    profile("es", "Spanish", &["Latn"], &["span", "spanish"]),
    profile("cop", "Coptic", &["Copt", "Latn"], &["copt", "coptic"]),
    profile("hy", "Armenian", &["Armn", "Latn"], &["arm", "armenian"]),
    profile(
        "egy",
        "Ancient Egyptian",
        &["Egyp", "Latn"],
        &["egypt", "egyptian"],
    ),
    profile(
        "akk",
        "Akkadian (Assyrian)",
        &["Xsux", "Latn"],
        &["assyr", "assyrian"],
    ),
    // Explicitly attested long-tail comparanda. Empty label sets keep these
    // valid for review without enabling collision-prone automatic tagging.
    profile(
        "pal",
        "Pahlavi (Middle Persian)",
        &["Phli", "Prti", "Arab", "Latn"],
        &[],
    ),
    profile("ang", "Old English (Anglo-Saxon)", &["Latn"], &[]),
    profile("da", "Danish", &["Latn"], &[]),
    profile("nl", "Dutch", &["Latn"], &[]),
    profile("it", "Italian", &["Latn"], &[]),
    profile("pt", "Portuguese", &["Latn"], &[]),
    profile("pl", "Polish", &["Latn"], &[]),
    profile("ru", "Russian", &["Cyrl"], &[]),
    profile("sv", "Swedish", &["Latn"], &[]),
    profile("no", "Norwegian/Norse", &["Latn"], &[]),
    profile("ga", "Irish", &["Latn"], &[]),
    profile("sla", "Slavic (undifferentiated)", &["Cyrl", "Latn"], &[]),
    profile("ota", "Ottoman Turkish", &["Arab", "Latn"], &[]),
    profile("tt", "Tatar", &["Arab", "Cyrl", "Latn"], &[]),
    profile("ms", "Malay", &["Arab", "Latn"], &[]),
    profile("hi", "Hindi/Hindustani", &["Deva", "Arab", "Latn"], &[]),
    profile("zh", "Chinese", &["Hani"], &[]),
    profile("xsa", "Sabaean/Himyaritic", &["Sarb", "Latn"], &[]),
    profile("mt", "Maltese", &["Latn"], &[]),
    profile("cop-x-sahidic", "Sahidic Coptic", &["Copt", "Latn"], &[]),
    profile("he-x-rabbinic", "Rabbinic Hebrew", &["Hebr"], &[]),
    profile("arc-x-talmudic", "Talmudic Aramaic", &["Hebr", "Syrc"], &[]),
];

const fn profile(
    tag: &'static str,
    name: &'static str,
    scripts: &'static [&'static str],
    labels: &'static [&'static str],
) -> LanguageProfile {
    LanguageProfile {
        tag,
        name,
        scripts,
        labels,
    }
}

/// Identifies semantic languages and exact character ranges in text.
///
/// `default_language` is used only for ambiguous scripts such as Latin and
/// square Hebrew; unique native scripts always take precedence. Printed labels
/// can refine a compatible neighbouring native-script form. Latin-script
/// comparative forms are relabelled only after an abbreviated printed label,
/// avoiding prose such as "the French h".
#[must_use]
pub fn identify_languages(
    text: &str,
    default_language: &str,
) -> (Option<String>, Vec<LanguageRun>) {
    let mut tokens = tokens(text, default_language);
    for index in 0..tokens.len() {
        let Some(profile) = profile_for_label(&tokens[index].text) else {
            continue;
        };
        if let Some(previous) = index.checked_sub(1).and_then(|index| tokens.get_mut(index)) {
            if previous.script != "Latn" && profile.scripts.contains(&previous.script.as_str()) {
                previous.language = profile.tag;
                previous.evidence = LanguageEvidence::PrintedLabel;
            }
        }
        let abbreviated = tokens[index].text.contains('.');
        if let Some(next) = tokens.get_mut(index + 1) {
            let native_script = next.script != "Latn" && next.script != "Zyyy";
            let labelled_latin = abbreviated
                && latin_comparison_language(profile.tag)
                && latin_citation_candidate(&next.text);
            if profile.scripts.contains(&next.script.as_str()) && (native_script || labelled_latin)
            {
                next.language = profile.tag;
                next.evidence = LanguageEvidence::PrintedLabel;
            }
        }
    }

    let mut runs: Vec<LanguageRun> = Vec::new();
    for token in tokens.into_iter().filter(|token| token.has_letters) {
        if let Some(previous) = runs.last_mut().filter(|previous| {
            previous.language == token.language
                && previous.script == token.script
                && previous.evidence == token.evidence
        }) {
            previous.end = token.end;
        } else {
            runs.push(LanguageRun {
                start: token.start,
                end: token.end,
                language: token.language.to_owned(),
                script: token.script,
                evidence: token.evidence,
            });
        }
    }

    let languages = runs
        .iter()
        .map(|run| run.language.as_str())
        .collect::<BTreeSet<_>>();
    let language = match languages.len() {
        0 => Some("zxx".to_owned()),
        1 => languages.first().map(|language| (*language).to_owned()),
        _ => Some("mul".to_owned()),
    };
    (language, runs)
}

/// Finds an explicit printed language label.
#[must_use]
pub fn profile_for_label(text: &str) -> Option<&'static LanguageProfile> {
    let label = normalized_label(text);
    if ambiguous_lowercase_abbreviation(&label)
        && text.contains('.')
        && !text
            .chars()
            .find(|character| character.is_alphabetic())
            .is_some_and(char::is_uppercase)
    {
        return None;
    }
    ROBINSON_1854_LANGUAGES
        .iter()
        .find(|profile| profile.labels.contains(&label.as_str()))
}

/// Looks up a semantic language by BCP 47 tag.
#[must_use]
pub fn profile_for_tag(tag: &str) -> Option<&'static LanguageProfile> {
    ROBINSON_1854_LANGUAGES
        .iter()
        .find(|profile| profile.tag == tag)
}

#[derive(Debug)]
struct Token {
    start: usize,
    end: usize,
    text: String,
    language: &'static str,
    script: String,
    evidence: LanguageEvidence,
    has_letters: bool,
}

fn tokens(text: &str, default_language: &str) -> Vec<Token> {
    let characters = text.chars().collect::<Vec<_>>();
    let mut result = Vec::new();
    let mut start = None;
    for (index, character) in characters
        .iter()
        .copied()
        .chain(std::iter::once(' '))
        .enumerate()
    {
        if !character.is_whitespace() {
            start.get_or_insert(index);
            continue;
        }
        let Some(token_start) = start.take() else {
            continue;
        };
        let token_text = characters[token_start..index].iter().collect::<String>();
        let script = token_script(&token_text);
        let has_letters = token_text.chars().any(char::is_alphabetic);
        let (language, evidence) = language_for_script(&script, default_language);
        result.push(Token {
            start: token_start,
            end: index,
            text: token_text,
            language,
            script,
            evidence,
            has_letters,
        });
    }
    result
}

fn token_script(text: &str) -> String {
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
        first.short_name().to_owned()
    }
}

fn language_for_script(script: &str, default_language: &str) -> (&'static str, LanguageEvidence) {
    let script_language = match script {
        "Hebr" => "he",
        "Arab" => "ar",
        "Syrc" => "syr",
        "Ethi" => "gez",
        "Samr" => "sam",
        "Phnx" => "phn",
        "Grek" => "grc",
        "Deva" => "sa",
        "Avst" => "ae",
        "Goth" => "got",
        "Copt" => "cop",
        "Armn" => "hy",
        "Egyp" => "egy",
        "Xsux" => "akk",
        "Latn" | "Zyyy" => "en",
        _ => "und",
    };
    if let Some(default) =
        profile_for_tag(default_language).filter(|profile| profile.scripts.contains(&script))
    {
        let evidence = if default.tag == script_language && !matches!(script, "Latn" | "Zyyy") {
            LanguageEvidence::UnicodeScript
        } else {
            LanguageEvidence::EditionDefault
        };
        return (default.tag, evidence);
    }
    let evidence = if matches!(script, "Latn" | "Zyyy") {
        LanguageEvidence::EditionDefault
    } else {
        LanguageEvidence::UnicodeScript
    };
    (script_language, evidence)
}

fn latin_comparison_language(tag: &str) -> bool {
    matches!(
        tag,
        "la" | "fa" | "sa" | "ae" | "got" | "de" | "fr" | "es" | "cop" | "hy"
    )
}

fn latin_citation_candidate(text: &str) -> bool {
    let word = normalized_label(text);
    word.chars().count() >= 2
        && !matches!(
            word.as_str(),
            "a" | "an"
                | "and"
                | "comp"
                | "f"
                | "from"
                | "id"
                | "in"
                | "is"
                | "m"
                | "n"
                | "name"
                | "of"
                | "or"
                | "pers"
                | "person"
                | "pr"
                | "root"
                | "see"
                | "the"
                | "to"
                | "used"
                | "word"
        )
}

fn ambiguous_lowercase_abbreviation(label: &str) -> bool {
    matches!(label, "arm" | "fr" | "gr" | "pers")
}

fn normalized_label(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{identify_languages, profile_for_label, ROBINSON_1854_LANGUAGES};
    use crate::model::LanguageEvidence;

    #[test]
    fn catalogue_uses_unique_bcp47_tags_and_recognizes_historical_labels() {
        let tags = ROBINSON_1854_LANGUAGES
            .iter()
            .map(|profile| profile.tag)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(tags.len(), ROBINSON_1854_LANGUAGES.len());
        assert_eq!(
            profile_for_label("Chald.").map(|profile| profile.tag),
            Some("arc")
        );
        assert_eq!(
            profile_for_label("Sanscr.").map(|profile| profile.tag),
            Some("sa")
        );
        assert_eq!(
            profile_for_label("Zend.").map(|profile| profile.tag),
            Some("ae")
        );
    }

    #[test]
    fn identifies_mixed_native_scripts_without_conflating_language_and_script() {
        let (language, runs) = identify_languages("Compare Arab. أب and Syr. ܐܒ.", "en");
        assert_eq!(language.as_deref(), Some("mul"));
        assert_eq!(
            runs.iter()
                .map(|run| (run.language.as_str(), run.script.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("en", "Latn"),
                ("ar", "Arab"),
                ("en", "Latn"),
                ("syr", "Syrc")
            ]
        );
    }

    #[test]
    fn printed_labels_distinguish_languages_that_share_scripts() {
        let (_, aramaic) = identify_languages("אָב Chald. m.", "en");
        assert_eq!(aramaic[0].language, "arc");
        assert_eq!(aramaic[0].evidence, LanguageEvidence::PrintedLabel);

        let (_, persian) = identify_languages("Pers. بند", "en");
        assert_eq!(persian[1].language, "fa");
        assert_eq!(persian[1].script, "Arab");
    }

    #[test]
    fn abbreviated_labels_identify_latin_script_comparative_forms_conservatively() {
        let (_, runs) = identify_languages("Sanscr. bandha, Germ. Band", "en");
        assert_eq!(
            runs.iter()
                .map(|run| run.language.as_str())
                .collect::<Vec<_>>(),
            vec!["en", "sa", "en", "de"]
        );

        let (_, prose) = identify_languages("the French h in habit", "en");
        assert!(prose.iter().all(|run| run.language == "en"));

        let (_, grammar) = identify_languages("1 pers. suffix; Pers. pr. n.", "en");
        assert!(grammar.iter().all(|run| run.language == "en"));
    }

    #[test]
    fn non_linguistic_text_is_explicitly_tagged() {
        let (language, runs) = identify_languages("15, 8. 9.", "en");
        assert_eq!(language.as_deref(), Some("zxx"));
        assert!(runs.is_empty());
    }
}
