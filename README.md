# Gesenius OCR and Unicode corpus

This repository is a reproducible, local-first OCR pipeline for the public-domain Gesenius Hebrew lexicon. It preserves the Robinson 1854 and Tregelles 1857 editions separately, turns page scans into reviewable Unicode entries, and exports authoritative JSONL, a TEI Lex-0 profile, and schema-versioned SQLite.

Rust owns orchestration, source verification, ALTO parsing, corpus modelling, validation, review, metrics, and exports. Tesseract 5 runs an English-primary layout pass plus a multilingual word-language detector. Each detected Hebrew, Arabic, Syriac, or Ancient Greek word is then cropped, enlarged, read with one script-appropriate model, and joined back into its line. Kraken 7 is the trainable recognizer and remains an isolated subprocess. Scans never leave the local machine.

The implementation is usable now, but the corpus is explicitly an OCR draft: the full books have not been processed or human-verified, the Tregelles scan still needs an owner-selected registration, and the candidate pilot pages must be visually confirmed after that scan is selected.

## Quick start

Enter the pinned CPU environment:

```console
nix develop path:.
cargo run -- --help
```

The shell contains Rust, Clippy, rustfmt, Poppler, ImageMagick, SQLite, `xmllint`, Jing, Tesseract with English/Hebrew/Arabic/Syriac/Ancient Greek/Latin data, Noto fonts, and a CPU-only Kraken 7.0.2 environment built from `ocr/uv.lock`.

The Robinson 1854 catalogue record points at the Library of Congress scan mirrored by Internet Archive. Its exact SHA-256 is checked in:

```console
cargo run -- source fetch --edition robinson-1854
cargo run -- source verify --edition robinson-1854
```

To use a local copy instead:

```console
cargo run -- source import \
  --edition robinson-1854 \
  --path /path/to/hebrewenglishlex00gese.pdf
```

Run a PDF page or range (PDF pages are one-based):

```console
cargo run -- run --edition robinson-1854 --pages 17-20,45
cargo run -- validate
```

The run command writes phase and per-page progress to stderr while reserving
stdout for its final machine-readable JSON result. Resumed stages are still
reported as the pipeline checks and reuses their receipts.

The initial configuration runs Tesseract only. Its English pass first supplies page layout, then each multi-line layout block is cropped and re-read as a single block so language-model context stays within one column. The English-primary and multilingual ALTO outputs remain separate and are aligned by geometry. The multilingual output labels words by strong Unicode script, printed labels such as `Heb.` provide deterministic hints for otherwise garbled tokens, isolated script glitches are smoothed within a foreign phrase, and each foreign word receives a single-language recognition pass before the line is conservatively fused. The untouched page hypotheses, block-refinement manifest, and per-word decision manifest remain attached or retained for audit. After pilot ground truth produces a checksummed Kraken model, set `kraken.enabled = true`, `model_path`, and `model_sha256` in `pipeline.toml`. Kraken then becomes primary while all complete hypotheses remain attached to every spatially matching span.

## Stable CLI

```text
gesenius source fetch|import|verify
gesenius run --edition EDITION --pages PAGES
gesenius train [--execute]
gesenius validate
gesenius review serve
gesenius export --format jsonl|tei|sqlite --output DIRECTORY
gesenius report
```

Global options can relocate `sources.toml`, `pipeline.toml`, the content-addressed cache, machine corpus, or review patch log. Use `cargo run -- --help` and the subcommand help for the exact flags.

## Pipeline and artifacts

Each run is content-addressed by the source hash, complete pipeline settings, model identity, and pipeline tree identity. Completed stages have receipts and are skipped only when their inputs and outputs still match:

```text
verified PDF
  -> lossless 400 DPI raster (original.png)
  -> recorded photometric/geometric transform (processed.png + transform.json)
  -> English-primary Tesseract ALTO + multilingual Tesseract ALTO
  -> isolated single-language word OCR + reconstructed ALTO
  -> conservative word-level script fusion + optional Kraken ALTO
  -> conservative entry boundaries + explicit line assignments
  -> corpus/machine/<edition>.jsonl
```

Large PDFs, rasters, OCR intermediates, and run receipts live below `.cache/gesenius/` and are ignored by Git. Original rasters and exact transform commands are retained. `tesseract-word-recognitions.json` records each crop, detected and selected language, both texts and confidences, and which text was used. ALTO preserves regions, lines, reading order, polygons, and confidence. A recognized line is always assigned to an entry, `front_matter`, or `unparsed`.

Immutable human- or frontier-transcribed gold fixtures under `benchmarks/gold/`
measure OCR without becoming corpus corrections:

```bash
cargo run -- benchmark \
  --gold benchmarks/gold/robinson-1854-p001-e0001.json \
  --alto .cache/gesenius/runs/<run>/robinson-1854/page-0017/tesseract-fused.alto.xml
```

The command reports overall CER/WER, per-script CER, and missing gold line IDs.

Entries continuing onto a consecutive page retain the stable ID of the page on which they began. Headers and footers are kept out of continuations. Printed hyphenation remains diplomatic until a reviewer makes an explicit structural correction.

## Corpus contract

JSONL is authoritative and stores one entry per line. IDs are source-derived:

```text
<edition>:p<printed-page>:e<ordinal>
```

An entry contains:

- edition/source provenance and aliases;
- a headword, homograph, grammar labels, senses, citations, cross-references, and etymology where confidently parsed;
- ordered blocks and `unclassified` fallback content;
- diplomatic and NFC-normalized text;
- BCP 47 language, ISO 15924 script, and direction metadata;
- source page, region, line, polygon, transform, and page image for every span;
- untouched engine/model hypotheses, confidence, Unicode warnings, and review state.

Compatibility normalization is never applied. Canonical text is stored in Unicode logical order without embedded bidi controls; OCR-inserted controls remain auditable in the untouched ALTO and engine hypothesis.

The schema files are:

- `schema/corpus.schema.json` — JSON entry interchange contract;
- `schema/tei-lex0.rng` — pinned project TEI Lex-0 profile;
- `schema/sqlite-v1.sql` — application-independent SQLite schema.

See [docs/data-model.md](docs/data-model.md) for review and export details.

## Review and training

Start the loopback-only review service:

```console
cargo run -- review serve --bind 127.0.0.1:8787
```

The UI shows the source page and ALTO overlays, complete competing hypotheses, Unicode code points, script warnings, confidence, and editable structured JSON. Saves require the revision the reviewer observed; stale edits receive HTTP 409 rather than silently overwriting newer work.

Reviews append complete replacements to `corpus/review/patches.jsonl`. Machine JSONL is not rewritten, and generated TEI/SQLite files are never edited. Materialization replays the strictly monotonic patch chain.

`pilot.toml` fixes 24 printed-page labels per edition and records why each page was selected. Once corrected or verified pilot spans exist:

```console
cargo run -- train --pilot pilot.toml --output training
cargo run -- train --pilot pilot.toml --output training \
  --execute --output-model training/checkpoints \
  --base-model models/base.mlmodel
```

Splits are deterministic by edition and source page, so lines from one page cannot leak across train, validation, and test. Ground truth is emitted as line crops plus `.gt.txt`, and baseline CER/WER is reported overall and per script. Training explicitly uses NFC logical-order text and a CPU-capable Kraken invocation.

Model binaries are ignored. Publish them as separately checksummed release artifacts with a completed [model card template](models/model-card.template.toml).

## Exports and reports

Review patches are applied before validation and export:

```console
SOURCE_DATE_EPOCH=0 cargo run -- export --format jsonl --output artifacts/jsonl
SOURCE_DATE_EPOCH=0 cargo run -- export --format tei --output artifacts/tei
SOURCE_DATE_EPOCH=0 cargo run -- export --format sqlite --output artifacts/sqlite
cargo run -- report --output reports
```

Every output directory contains `manifest.json` with corpus/schema version, source and model hashes, pipeline identity, generation time, metrics, and draft status. Tesseract provenance is a composite SHA-256 of the exact requested `.traineddata` files; Kraken provenance is the registered model-file SHA-256. TEI metadata repeats the provenance and is validated with `xmllint`. SQLite records the same metadata internally, enables foreign keys, indexes normalized headwords, and provides English full-text search.

Use `SOURCE_DATE_EPOCH` for release builds. With identical materialized entries and manifest, all three primary artifacts rebuild byte-for-byte.

The comparison report covers currently processed page/entry completeness, confidence and script distribution, missing headwords, and edition-only or textually differing headwords. It never selects a canonical edition automatically.

## Verification

```console
nix flake check path:.
nix develop path:. --command bash -euo pipefail -c \
  'cargo fmt --all --check &&
   cargo clippy --workspace --all-targets -- -D warnings &&
   cargo test --workspace'
nix develop path:. --command ./tool/test-fixture-pipeline.sh
```

Fixtures cover both editions, columns, headers, damaged contrast, mixed Hebrew/Arabic/Syriac/Greek/Latin, engine disagreement, cross-page entries, and diplomatic hyphenation. Full-book OCR and model training remain manual release jobs because they are resumable but computationally expensive.

Release gates and the remaining owner decisions are documented in [docs/release.md](docs/release.md).
