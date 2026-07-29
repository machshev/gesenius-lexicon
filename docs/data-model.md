# Corpus and review model

## Authority

The base machine corpus is one JSON object per line in `corpus/machine/<edition>.jsonl`. Human changes are append-only `ReviewPatch` objects in `corpus/review/patches.jsonl`. A patch names the entry, observed base revision, new revision, reviewer, timestamp, note, and complete replacement entry.

The materialized corpus is:

```text
sorted base JSONL + strictly ordered valid patches
```

Patch replay fails on a missing entry, skipped/repeated revision, mismatched entry ID, or changed source identity. The review service takes an exclusive OS lock while checking and appending a revision.

Generated TEI and SQLite are disposable views. They must never become correction sources.

## Text

`diplomatic` records the corpus transcription; `normalized` is exactly its NFC form. Compatibility normalization is prohibited because it can erase meaningful historical glyph distinctions. The untouched OCR text remains in `hypotheses`, even when an engine inserted bidi controls that are excluded from canonical text.

Every span has a BCP 47 language when known, an ISO 15924 script, and `ltr`, `rtl`, or `mixed` direction. Text is in logical order. Direction is presentation metadata, not an embedded formatting character.

`machine`, `corrected`, and `verified` apply both to entries and spans. A draft may publish machine text, but provenance, confidence, warnings, and review state remain visible.

## Structure and fallback

The parser opens a new entry when a non-margin line begins with Hebrew. It carries the last entry to an immediately consecutive page. When no continuation exists, leading non-margin content opens a headless fallback entry so section introductions and page continuations are retained; only margin lines remain `unparsed`. Content first enters ordered `unclassified` blocks; later parsers or reviewers can identify forms, grammar, definitions, etymology, citations, cross-references, and senses without losing order.

Stable IDs derive from edition, the printed page on which the entry begins, and its one-based ordinal. Merges and splits retain displaced IDs as aliases.

## Geometry

Every span cites one or more source coordinates:

- one-based source PDF page and printed-page label;
- ALTO region and line IDs;
- polygon in processed image coordinates;
- content-addressed page image;
- transform identity.

The run directory keeps `original.png`, `processed.png`, the exact ImageMagick arguments, input/output dimensions, and ordered transform operations. Raw English-primary Tesseract, multilingual Tesseract, reconstructed word-level Tesseract, and Kraken ALTO files remain separate. `tesseract-word-recognitions.json` records the detected and selected language, isolated crop, candidate text and confidence, and selection result for each foreign word. ALTO word boxes and confidences are retained in the intermediate model so embedded foreign-script runs can be aligned geometrically without treating a mixed line as one recognition decision.

## SQLite

Schema version 1 separates editions, entries, aliases, senses, blocks, spans, OCR hypotheses, source coordinates, Unicode warnings, citations, and cross-references. `PRAGMA foreign_key_check` and `integrity_check` are release gates. `entry_fts` is an FTS5 projection; JSONL remains authoritative.
