# Language coverage and identification

The corpus keeps three separate facts that must not be conflated:

- `language` and `language_runs[].language` are semantic BCP 47 tags;
- `script` and `language_runs[].script` are ISO 15924 writing systems;
- Tesseract settings such as `heb` and `ara` are OCR model identifiers.

For example, Biblical Aramaic is `arc` even when printed in square Hebrew
(`Hebr`) and recognized with Tesseract's `heb` model. Persian is `fa` when an
adjacent `Pers.` label identifies Arabic-script text; the visible script alone
would only justify an Arabic-script OCR route, not the Arabic language tag.
Likewise, if a printed `Arab.` label precedes characters that OCR encoded as
Syriac, the semantic tag is `ar`, the script remains `Syrc`, and the evidence is
`printed_label`. This records the disagreement instead of silently rewriting it.

## Robinson 1854 inventory

The edition's preface explicitly describes Hebrew and its Semitic comparanda,
then names Sanskrit, Zend (Avestan), Persian, Greek, Latin, Gothic, German, and
English. A full text-layer audit of the local pinned source also finds repeated
labels or discussions for the following languages. The automatic catalogue is
intentionally conservative: a language is identified only from distinctive
Unicode script or a sufficiently local printed label.

| BCP 47 | Edition name | Printed scripts | Current identification |
|---|---|---|---|
| `en` | English | `Latn` | edition default |
| `he` | Hebrew | `Hebr`, transliterated `Latn` | script; `Heb.`/`Hebr.` label |
| `arc` | Biblical Aramaic/Chaldee | `Hebr`, `Armi`, `Syrc`, transliterated `Latn` | `Chald.`/Aramaic label; never inferred from Hebrew script alone |
| `ar` | Arabic | `Arab`, transliterated `Latn` | script; `Arab.` label |
| `syr` | Syriac | `Syrc`, transliterated `Latn` | script; `Syr.` label |
| `gez` | Ge'ez/Ethiopic | `Ethi`, transliterated `Latn` | script; Ethiopic label |
| `sam` | Samaritan Aramaic | `Samr`, sometimes comparative `Hebr`/`Latn` | script; unambiguous Samaritan label |
| `phn` | Phoenician | `Phnx`, comparative `Hebr`/`Latn` | script; Phoenician label |
| `grc` | Ancient Greek | `Grek`, transliterated `Latn` | script; `Gr.` label |
| `la` | Latin | `Latn` | abbreviated `Lat.` label only |
| `fa` | Persian | `Arab`, transliterated `Latn` | `Pers.` label; Arabic script alone remains `ar` |
| `sa` | Sanskrit/Sanscrit | `Deva`, transliterated `Latn` | script; abbreviated `Sanscr.` label |
| `ae` | Avestan/Zend | `Avst`, transliterated `Latn` | script; abbreviated `Zend.` label |
| `got` | Gothic | `Goth`, transliterated `Latn` | script; abbreviated `Goth.` label |
| `de` | German | `Latn` | abbreviated `Germ.` label only |
| `fr` | French | `Latn` | abbreviated `Fr.` label only |
| `es` | Spanish | `Latn` | abbreviated `Span.` label only |
| `cop` | Coptic, including Sahidic references | `Copt`, transliterated `Latn` | script; abbreviated `Copt.` label |
| `hy` | Armenian | `Armn`, transliterated `Latn` | script; abbreviated `Arm.` label |
| `egy` | Ancient Egyptian | `Egyp`, transliterated `Latn` | script or explicit Egyptian label |
| `akk` | Akkadian/Assyrian | `Xsux`, transliterated `Latn` | script or explicit Assyrian label |

The machine-readable catalogue also accounts for Pahlavi/Middle Persian, Anglo-Saxon, Danish,
Dutch, Italian, Portuguese, Polish, Russian, Swedish/Norse, Celtic/Irish,
Slavic, Turkish/Tatar, Malay, Hindustani, Chinese, Himyaritic/Sabaean, Maltese,
Rabbinic Hebrew, and Talmudic Aramaic. These are accounted for in the review
inventory with BCP 47 tags but remain manual-only until fixtures establish collision-free labels
and exact historical BCP 47 choices. This avoids errors such as reading biblical
`Dan.` as Danish or assigning every occurrence of the prose word “French” to a
French citation.

| Manual BCP 47 tags | Edition terminology |
|---|---|
| `pal` | Pahlavi/Middle Persian |
| `ang` | Anglo-Saxon |
| `da`, `nl`, `it`, `pt`, `pl`, `ru`, `sv`, `no` | named European comparanda |
| `ga`, `sla` | Irish/Celtic and undifferentiated Slavic |
| `ota`, `tt` | historical Turkish and Tatar |
| `ms`, `hi`, `zh` | Malay, Hindustani, and Chinese |
| `xsa`, `mt` | Himyaritic/Sabaean and Maltese |
| `cop-x-sahidic` | Sahidic Coptic |
| `he-x-rabbinic`, `arc-x-talmudic` | Rabbinic Hebrew and historically unspecified Talmudic Aramaic |

## OCR support

The pinned initial OCR environment provides `eng`, `heb`, `ara`, `syr`, `grc`,
and `lat`. These models cover the dominant scripts and remain deliberately
separate from semantic language metadata. Languages without an exact model are
still identified and retained; they are not silently relabelled as the fallback
model's language. Adding a model requires a gold fixture showing an improvement
for that language/script before it joins the default pass.

## Evidence and validation

Each `language_run` uses Unicode-scalar offsets into normalized text and records
one of:

- `unicode_script` for a distinctive native script;
- `printed_label` for a local edition label that resolves a shared or
  OCR-confused script;
- `edition_default` for otherwise ambiguous dominant English prose or the
  Hebrew-language structural role of a lexicon headword.

A line containing more than one language is summarized as `mul`; text with no
linguistic content is `zxx`. Validation checks run bounds, concrete catalogue
tags, script agreement, and the line-level summary. Legacy spans without
language metadata receive a warning so existing OCR drafts remain readable but
cannot be mistaken for fully classified output.
