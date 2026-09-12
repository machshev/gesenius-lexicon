# Gesenius OCR and Unicode corpus

This repository is a reproducible, local-first OCR pipeline for the public-domain Gesenius Hebrew lexicon. It preserves the Robinson 1854 and Tregelles 1857 editions separately, turns page scans into reviewable Unicode entries, and exports authoritative JSONL, a TEI Lex-0 profile, and schema-versioned SQLite.

Rust owns orchestration, source verification, ALTO parsing, corpus modelling, validation, review, metrics, and exports. Tesseract 5 runs an English-primary layout pass plus a multilingual word-script detector. Each word that looks foreign is then cropped, enlarged, read with every one of the Hebrew, Arabic, Syriac, and Ancient Greek models, arbitrated to a single script, and joined back into its line. Semantic BCP 47 language identification remains separate from OCR routing and records exact language runs with evidence. Kraken 7 is the trainable recognizer and remains an isolated subprocess. Scans never leave the local machine.

The implementation is usable now, but the corpus is explicitly an OCR draft: the full books have not been processed or human-verified, the Tregelles scan still needs an owner-selected registration, and the candidate pilot pages must be visually confirmed after that scan is selected.

## Quick start

Enter the pinned CPU environment:

```console
nix develop path:.
cargo run -- --help
```

The shell contains Rust, Clippy, rustfmt, Poppler, ImageMagick, SQLite, `xmllint`, Jing, Tesseract with English/Hebrew/Arabic/Syriac/Ancient Greek/Latin data, Noto fonts, and a CPU-only Kraken 7.1 environment built from `ocr/uv.lock`.

Check the OCR environment and install the configured Kraken model with:

```console
cargo run -- setup
```

Use `cargo run -- setup --check-only` in CI or before a run when downloads are
not allowed. Missing system tools are installed by entering the pinned shell
with `nix develop path:.`; the setup command downloads only the ignored model
weight and verifies its configured SHA-256.

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

Use an open-ended range such as `17-` to process page 17 through the
registered end of the PDF.

The run command writes phase and per-page progress to stderr while reserving
stdout for its final machine-readable JSON result. Resumed stages are still
reported as the pipeline checks and reuses their receipts.

The English Tesseract pass first supplies page layout, then each multi-line layout block is cropped and re-read as a single block so language-model context stays within one column. The English-primary and multilingual ALTO outputs remain separate and are aligned by geometry. The multilingual output routes words by strong Unicode script, and words the English pass could only read as implausible Latin are routed to Hebrew, the script the edition sets its lemmas in. Each routed word is read at every configured crop padding and segmentation mode with every single-script model, and the script is then arbitrated from those readings: a printed label such as `Heb.` or `Chald.` decides outright, as does a confident reading in a distinctive script, and otherwise the best agreement-weighted confidence wins with Hebrew preferred on ties. A script neither a label nor recognized code points announced is never introduced on confidence alone. Every trial and crop is recorded, so rejected script and crop interpretations are as auditable as the chosen one. The untouched page hypotheses, block-refinement manifest, and per-word decision manifest remain attached or retained for audit. The configured PP-OCRv6 model supplies an independent full-page hypothesis. In line-refinement mode, only its high-confidence, geometrically comparable Roman readings can replace low-confidence Roman words, and mixed-script lines remain under the script-specific Tesseract path.

## Stable CLI

```text
gesenius source fetch|import|verify
gesenius run --edition EDITION --pages PAGES
gesenius train --splits PATH [--headwords-only] [--execute]
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

The command reports exact CER/WER, a separate NFC-equivalence score, aligned
script substitutions and sample counts, exact foreign-containing token
accuracy/precision, missing gold lines, and alignment/provenance status. Base and
pointing diagnostics remain separate from exact text scores; absent foreign-token
support reports null accuracy rather than a perfect result. Existing fixtures use legacy line IDs
and are explicitly unverified without a source identity. To check an asserted
identity against the gold fixture, add `--hypothesis-identity identity.json` with:

```json
{"edition":"robinson-1854","source_page":17,"source_sha256":"466b061e770f212cb7d888d8dadc2a54575fb115bf6de9cdb24b0c280461ccaa"}
```

Resolve that identity from the hypothesis's source receipts; copying the gold
identity alone does not establish provenance. Coordinate-aligned fixtures also
require source image dimensions, rectangular anchors, and matching asserted
`coordinate_frame` values identifying the image and transforms. Their reported
text policy joins line boundaries with spaces. Coordinate results also report
one-to-one line correspondences, split/merge candidates and missing lines
separately from transcription error. OCR outside the gold anchors remains
unassessed; legacy fixtures have no measured segmentation result. See the
[metric policy](docs/ocr-metric-policy.md),
[sample inventory](benchmarks/sample-inventory/robinson-1854-audit.md), and
[cached baseline](benchmarks/baselines/ocr-baseline-page17-2026-09-05.md) for
scoring limits and the current measurement work.

Compare retained OCR stages on one gold sample with:

```console
cargo run -- benchmark-stages --manifest experiment/stages.json > experiment/comparison.json
```

The manifest names the gold fixture, baseline stage, and each stage's ALTO and
source-identity file. Paths resolve relative to the manifest. The report preserves
full scores, support and segmentation evidence, adds signed changes against the
baseline and preceding stage, and fingerprints every input and the CLI executable.
See the [stage comparison contract](docs/ocr-metric-policy.md#stage-comparison-reports)
for the manifest format and interpretation. This command evaluates existing ALTO;
it does not run OCR or infer source identity from filenames.

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
- BCP 47 language summaries and exact language runs, ISO 15924 scripts, evidence, and direction metadata;
- source page, region, line, polygon, transform, and page image for every span;
- untouched engine/model hypotheses, confidence, Unicode warnings, and review state.

Compatibility normalization is never applied. Canonical text is stored in Unicode logical order without embedded bidi controls; OCR-inserted controls remain auditable in the untouched ALTO and engine hypothesis.

The schema files are:

- `schema/corpus.schema.json` — JSON entry interchange contract;
- `schema/tei-lex0.rng` — pinned project TEI Lex-0 profile;
- `schema/sqlite-v2.sql` — application-independent SQLite schema.
- `docs/languages.md` — edition language inventory, BCP 47 policy, and OCR coverage.

See [docs/data-model.md](docs/data-model.md) for review and export details.

## Review and training

### Entry index

Build a lightweight index of the currently processed entries, applying saved
corpus corrections first:

```console
cargo run -- index --edition robinson-1854 --output artifacts/index.json
```

To generate index candidates directly from scans with headword-focused OCR:

```console
cargo run -- index --edition robinson-1854 --pages 17-57 --output artifacts/index-fast.json
```

The independent primary and multilingual full-page OCR passes run concurrently,
allowing both index generation and the full pipeline to use multiple CPU cores.
Page parsing remains ordered so entries that continue across page boundaries are
reconstructed deterministically.

This mode keeps rasterization, preprocessing, English layout OCR, and an
English/Hebrew page pass. It separately bounds candidate headword words and
re-reads only those crops with the higher-accuracy isolated Hebrew pass.
Definition words are not refined; block re-reading, PDF-text correction, and
Kraken are skipped. A new entry requires both page-relative headword indentation
and a crop recognized as a Hebrew headword; incidental OCR block boundaries and
Latin-shaped structural guesses do not split an entry. Headword accuracy still
requires source review, especially where the page pass misses a candidate
entirely.
Draft records are saved after every completed page under
`.cache/gesenius/index-candidates/`, separately from the full machine corpus and
its review patches. Per-page parses and the completed-page ledger are written
atomically before a page is reported complete. Repeating the same page range
resumes cached stages. The completed-page ledger never widens the requested
range: `17-20` processes exactly those four PDF pages, while the explicitly
open-ended `17-` processes page 17 through the registered final page. The output
includes all accumulated candidate records for that edition.

The versioned JSON contains headwords (diplomatic and NFC), IDs and aliases,
homographs, source polygons and page images, corpus revisions, and provenance.
Entries without a detected headword remain present for review. Cross-page entries
retain all their source regions. `pages_with_entry_regions` reports represented
pages, not proof that every entry on those pages was detected.

This is a candidate index, not a verified full-document index.
Boundary review is always `unreviewed`: existing transcription approval does not
establish correct entry divisions. A dedicated split/merge and headword review
workflow is still needed to verify the generated candidates.
Definition correction and Bible vocabulary/frequency matching can follow later.

Start the loopback-only review service:

```console
cargo run -- review serve --bind 127.0.0.1:8787
```

When index candidates exist in `.cache/gesenius/index-candidates`, the review
server includes them automatically. Machine-corpus entries take precedence
when the same stable entry ID exists in both sources.

The UI shows the source page and ALTO overlays, complete competing hypotheses, Unicode code points, script warnings, confidence, and editable structured JSON. Saves require the revision the reviewer observed; stale edits receive HTTP 409 rather than silently overwriting newer work.

The review UI defaults to **Pages** and the first available edition. Entry and
page data load only for the selected edition. The page catalogue is compressed
into contiguous PDF-page ranges with the constant printed-page offset; the page
selector expands these ranges locally, without a redundant sidebar. HTMX
requests overlays and the right-hand entry key only for the selected page. Full
page images retain their aspect ratio and scroll vertically. Materialized
editions are cached in memory and refreshed automatically when their corpus
JSONL or the review patches change.

Choose **Transcription review** in the header (or open `/transcriptions`) to
review the source-anchored benchmark drafts. Select a line, enter your reviewer
name, and compare the prefilled draft with its crop. **Approve & next** saves an
unchanged reading and advances; after editing, **Save corrections & next** records
your corrected text. **Needs fixes / uncertain** saves an unresolved review with
a required note and keeps the current line open. Source-check notes are visible
immediately. Zoom controls and an original-crop link help inspect fine marks.

Transcriptions are edited as ordered **language runs**. Each run has its own
language and left-to-right/right-to-left direction; the combined preview isolates
RTL text within the surrounding English. Existing plain text is split into
script-based editing runs without changing any characters. Foreign-script runs
start as **Unspecified** until you choose their semantic language (for example,
Hebrew versus Aramaic). For `p0050-right-004`, the Hebrew form is edited separately
from `Plur.` and the following numbered English gloss.

Choose **New run language**, select text or place the cursor within a run, then
click **Create run at cursor / selection**. **Join next run** combines adjacent
pieces using the first run's language and direction, allowing entire RTL phrases
or sentences. The keyboard types into the last focused run or review notes.
Saved `runs` retain language, direction and text; their exact concatenation must
match the plain transcription. Direction is stored as metadata, not invisible
bidi controls. Existing reviews remain readable and are not rewritten.

The transcription form places its offline **Unicode keyboard** to the right of
the crop and transcription on desktop. The keyboard stays alongside the editor
and scrolls independently; narrow screens stack the panels. It includes Hebrew /
Aramaic, Arabic / Persian, Syriac, polytonic Greek, Ethiopic, Phoenician and Latin
transliteration palettes. Choose **Language / script**, click a letter, then its
pointing keys. Points attach to the preceding letter (or a selected letter);
the dotted-circle preview is not inserted. Search by Unicode name, glyph or code
point, or use **Insert code point** for another character. Backspace removes one
mark or character; **Undo keyboard edit** restores the last palette edit. Click
the transcription or review-notes field to choose where keys type. Switching
palettes does not alter language metadata or normalize existing text.

Transcription reviews append to `corpus/review/transcription-reviews.jsonl`
(beside the configured `--patches` file). New decisions record
`review_method: draft_assisted` and the displayed draft, so they are not presented
as blind transcriptions. Existing independent readings and journal records remain
intact; new reviews have no invented independent reading. Subsequent decisions
preserve history and require the same reviewer name.
Changed source/draft metadata or crop hashes cannot silently reuse an old review.
These decisions do not automatically promote text into gold or alter corpus
entries. Reloading the page restores saved progress. Drafts default to
`benchmarks/transcription-drafts`; override with `review serve
--transcription-drafts PATH`. Only samples labelled training, development or validation
are offered. This is a local review workflow, not authenticated reviewer identity.

During a multipage `run`, each completed page is atomically published to the machine corpus. Use **Reload** in an already-running review UI to browse and review that page while OCR continues on later pages.

Reviews append complete replacements to `corpus/review/patches.jsonl`. Machine JSONL is not rewritten, and generated TEI/SQLite files are never edited. Materialization replays the strictly monotonic patch chain.

`pilot.toml` fixes 24 printed-page labels per edition and records why each page was selected. Once corrected or verified pilot spans exist:

```console
cargo run -- train --pilot pilot.toml --splits PATH --output training
cargo run -- train --pilot pilot.toml --splits PATH --output training \
  --execute --output-model training/checkpoints \
  --base-model models/base.mlmodel --seed 42 --deterministic
```

Reviewed headwords are emitted as dedicated word-crop training samples, in
addition to reviewed entry lines. Reviewed body-text Hebrew words can use the
same compact crop path while retaining `sample_kind: hebrew-word`; the review UI
keeps them separate from target headwords. This lets corrections to fine vowel
points feed the recognizer without pairing a short Hebrew transcription with a
whole mixed-language source line. All samples stay in the same explicit
page-level partition. Add `--headwords-only` to the corpus-training path to
exclude entry lines.

Supply an authoritative printed-page split manifest with `--splits PATH`; unlisted reviewed pages are rejected, and development/final-test pages are excluded from preparation. The hash-based split fallback has been removed. Ground truth is emitted as line crops plus `.gt.txt`, and baseline CER/WER is reported overall and per script. Training explicitly uses NFC logical-order text and a CPU-capable Kraken invocation. Use an explicit `--seed` with `--deterministic` for comparable training runs.

The execution step writes `training-paths.txt` and `validation-paths.txt`; the
execution path passes these manifests to Kraken 7.1 with
`--training-data`/`--evaluation-data` and expands a loaded codec with
`--resize union`. The Hebrew mark range U+0591–U+05C7 is learnable only for
code points represented by real reviewed image examples, so inspect the
emitted `alphabet-audit.json` before launching a run; corpus preparation lists
training-only observed frequencies and missing Hebrew marks/letters. Missing
code points are not a requirement to manufacture labels.

For the pointed-headword benchmark and deliberate human review export, see the
[headword workflow](docs/pointed-headword-workflow.md). `export-headword-training`
creates auditable headword-only pairs; `train-prepared` validates and trains an
existing export. Training and model rollout still require reviewed data and
the bounded recognition experiment.

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
