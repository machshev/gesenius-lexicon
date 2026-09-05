# Multilingual OCR accuracy implementation plan

Status: in progress; initial sample audit, cached baseline, and scoring foundations verified.
Created: 2026-09-05.

## Objective

Improve source-faithful recognition across the Robinson 1854 lexicon, especially
pointed Hebrew, Arabic, Syriac, polytonic Greek, transliteration, and rare printed
glyphs. Preserve original OCR evidence and distinguish recognized text from
editorial reconstruction. Keep Tregelles separate until its source is registered
and its samples are verified.

This file is the working plan for subsequent sessions. Complete bounded
milestones, record evidence below, and commit verified work as it progresses.
Session groupings are suggested boundaries, not deadlines.

## Starting evidence and limits

The 2026-09-05 review inspected `pipeline.rs`, `alto.rs`, `metrics.rs`,
`benchmark.rs`, `training.rs`, the configuration, and existing documentation.
The worktree was clean when this plan was created; HEAD was `9294c57`.

The existing release binary evaluated the single checked-in gold fixture,
`benchmarks/gold/robinson-1854-p001-e0001.json`, against:

```text
.cache/gesenius/runs/360d1913632707d21584c6affccc1934848a4ded6cc583535be2ea5b1aecdc15/robinson-1854/page-0017/tesseract-fused.alto.xml
```

Result: CER 0.1470775, WER 0.2216312, 3,148 reference characters, 564 reference
words, and no missing gold line IDs. The current first entry contained
`אְכֶף` where the gold has `אֶלֶף.`. These are cached-output observations, not
a fresh execution of current source or a whole-book accuracy estimate. Reproduce
with a source-built binary before using this as an experimental baseline.

The existing per-script metric independently filters reference and hypothesis
into five scripts. Its results are diagnostic only: spatial correspondence is
lost, inherited marks are omitted from script totals, and rare scripts are not
covered. The gold fixture also maps historical Aleph variants to Phoenician
Aleph; make that transcription policy explicit in future scoring.

## Working rules

- Start each session by reading this file, `AGENTS.md`, and current Git status.
  Preserve unrelated work and inspect the latest implementation before resuming.
- Keep semantic language, visible script, and OCR model routing separate.
- Retain source crops, raw hypotheses, transformations, model hashes, selected
  candidates, and reasons for changes. Dictionary or model-assisted suggestions
  must not masquerade as direct image recognition.
- Inspect system load before expensive work. Run one generator or Cargo rebuild
  at a time. Use the pinned `nix develop path:.` environment where needed.
- Freeze test pages before tuning; never use them to build dictionaries, choose
  thresholds, train recognizers, or repeatedly select configurations.
- A passing parser test or `validate` is not evidence of transcription accuracy.
  Corpus changes require benchmark comparison, source inspection, corpus diff,
  and validation before their own commit.
- Do not process the whole book until a bounded representative evaluation
  supports the selected changes.

## Session group 1: establish representative measurement

### A. Verify the sample and baseline

- [ ] Visually audit candidate pages in `pilot.toml`; replace unsupported category
  or script claims with source-verified selections. Do not infer printed script
  from semantic language, such as assuming Aramaic requires `Armi`.
- [x] Create a sample inventory recording edition, PDF and printed page, source
  hash, crop coordinates, actual scripts, typography, damage, and selection reason.
- [ ] Begin with roughly 150–300 verified lines across 8–12 Robinson pages,
  adjusting the sample to obtain real coverage rather than filling a quota.
  Include ordinary prose, dense comparisons, headwords, mixed-direction lines,
  Arabic/Syriac connections, pointing, Greek accents, and rare glyphs.
- [x] Separate development, training/validation, and final test pages. Verify
  rare-script coverage in each usable partition; record coverage gaps explicitly.
- [ ] Record transcription authority and unresolved glyphs. Model-assisted
  transcription needs source checking before it becomes gold.
- [ ] Reproduce the existing benchmark using a source-built binary, then record
  source/model/configuration/code identities for the representative baseline.

### B. Improve scoring and stage comparison

Primary files: `crates/gesenius-core/src/{benchmark,metrics}.rs`, CLI benchmark
handling, and `benchmarks/gold/`.

- [x] Add stable source-coordinate anchors or explicit alignment so comparisons
  remain meaningful when an engine splits, merges, or renumbers lines. Check
  source/page identity instead of trusting matching engine-generated line IDs.
- [x] Retain overall CER/WER; add aligned script confusions, base-letter accuracy,
  pointing/accent accuracy, exact foreign-word accuracy, and script sample counts.
  Associate combining marks with their grapheme/base, including inherited marks.
- [x] Define Unicode normalization and historical-glyph scoring policies. Preserve
  diplomatic text; report any equivalence-aware score separately from exact score.
- [ ] Score segmentation and entry boundaries separately from transcription;
  include missing and extra text in designated fully transcribed regions.
- [ ] Compare page OCR, block refinement, isolated recognition, lexical changes,
  and final fusion on the same regions. Report gains and regressions by script.
- [ ] Add an oracle-candidate diagnostic: how often is the correct transcription
  already among retained alternatives but rejected by selection?

Acceptance: a reproducible baseline report, verified sample inventory, frozen
split manifest, and meaningful regression tests for wrong-script substitutions,
missing marks, rare scripts, and split/merged alignment. Missing coverage is
visible in reports. Record numeric acceptance tolerances before tuning.

## Session group 2: make selection faithful and auditable

### C. Separate lexical suggestions from recognition

Primary file: `crates/gesenius-core/src/pipeline.rs`, especially `lexical_prior`
and `select_word_candidate`; configuration: `pipeline.toml`.

- [ ] Record raw candidate text separately from any lexicon-proposed text and
  retain the rule, dictionary identity, and confidence provenance.
- [ ] Remove automatic post-recognition pointing/consonant replacement from the
  canonical recognition path, or require independently recorded image evidence
  before accepting it. Keep unsupported reconstruction as a review suggestion.
- [ ] Compare lexicon off, ranking-only, and current behavior on development data.
  Audit the six configured roots and document their source and evaluation overlap.
- [ ] Ensure confidence from an original reading is never presented as measured
  confidence for a substituted spelling.

Acceptance: ambiguous same-skeleton vocalizations and one-consonant neighbours
remain distinguishable; evidence is recoverable; held-out evaluation occurs only
after the selection policy is fixed.

### D. Replace arbitrary score advantages with measured selection

- [ ] Evaluate the existing confidence × square-root-length score and lexical
  bonus, including cases where spurious combining marks increase the score.
- [ ] Estimate correctness by model, script, crop mode, and confidence bucket on
  development/validation data. Use a simple documented selection rule first;
  introduce a learned ranker only if sample size and measured gains justify it.
- [ ] Use disagreement and selection margin for review prioritization; do not
  treat repeated runs of the same model as independent corroboration.
- [ ] Preserve an unresolved state when the evidence cannot choose reliably.

Acceptance: report selection accuracy, oracle gap, per-script regression, and
review volume at fixed quality targets. Include all candidate scores and reasons
in the decision manifest without discarding raw alternatives.

## Session group 3: improve crops and routing

### E. Compare segmentation and crop hypotheses

Primary files: `pipeline.rs` crop generation and `alto.rs` geometric fusion.

- [ ] Separate source-context padding from synthetic white border settings.
  Compare tight crops with a clean border against existing 0/8-pixel source padding.
- [ ] Correct the PSM 10 comment: it is single-character mode, not sparse text.
  Benchmark existing modes against suitable alternatives, including PSM 11 where
  appropriate, before changing defaults.
- [ ] Add bounded script-run/line alternatives for split or merged words,
  clipped points, and connected Arabic/Syriac text. Keep alignment to page geometry.
- [ ] Test image views selectively: original grayscale versus contrast processing,
  measured deskew, and justified threshold/scale variants. Inspect native scan
  detail before assuming higher raster DPI or enlargement adds information.
- [ ] Report accuracy and runtime per variant; prune variants that add cost without
  demonstrated benefit. Avoid a full Cartesian product across all pages.

Acceptance: source-checked examples demonstrate that added marks belong to the
target and connected words remain complete; aggregate gains survive evaluation
outside the examples used to design the change.

### F. Allow recovery from an incorrect initial script

Primary file: `alto.rs`, especially `select_script_trial`, fallback classification,
and printed-label routing.

- [ ] Measure routing recall using true script labels, including foreign words
  initially read as plausible Latin or as the wrong foreign script.
- [ ] Prototype an independent visual script signal on difficult crops, comparing
  it with a bounded exploration of alternative script recognizers. Choose the
  approach from measured accuracy and cost; a classifier is not a prerequisite.
- [ ] Retain multiple plausible routes where necessary. Replace unconditional
  trust in labels with evidence-aware handling of misread labels and scope errors.
- [ ] Allow strong visual evidence to propose a previously unannounced script,
  while auditing false script introduction and preserving uncertainty.
- [ ] Add source fixtures for unlabelled Syriac, Hebrew/Aramaic shared script,
  Persian/Arabic shared script, and ordinary Roman text that resembles OCR rubbish.

Acceptance: improved routing recall without exceeding the predefined false-route
tolerance. Semantic tags remain distinct from script/model choices; rare or
unsupported scripts are explicitly reviewable.

## Session group 4: adapt recognition to the edition

### G. Train and compare model arrangements

Primary file: `crates/gesenius-core/src/training.rs`; configuration and model cards
under `pipeline.toml` and `models/`.

- [ ] Audit crop/text correspondence and logical reading order before training,
  including mixed RTL/LTR lines, points, punctuation, and merged review spans.
- [ ] Verify actual training/validation/test script coverage. Preserve page-level
  isolation and exclude final benchmark pages from model and dictionary fitting.
- [ ] Compare an edition-adapted mixed-line recognizer with specialist script-run
  recognizers. Check base-model alphabet, typography suitability, and training
  interface compatibility before launching an expensive job.
- [ ] Include the hard material currently excluded by Roman-only Kraken fusion;
  change fusion eligibility only after the trained model demonstrates competence.
- [ ] Record learning curves and prioritize additional reviewed examples where
  errors persist. Do not infer adequacy from a fixed line count.
- [ ] Publish checksummed model identity, data/split identity, commands, runtime,
  metrics, limitations, and a model card for any adopted checkpoint.

Acceptance: a frozen model/configuration improves representative accuracy over
the strongest measured baseline, meets predefined script-specific tolerances,
and has an affordable measured runtime. If it fails, retain the baseline and
record the result rather than adopting the model by default.

## Integration and release checkpoint

- [ ] Run focused meaningful tests during each implementation milestone, then
  repository-required formatting, warning-denied Clippy, workspace tests, and
  relevant fixture checks before declaring integration complete.
- [ ] Evaluate the selected configuration once on the reserved test sample;
  report counts, per-script results, failure examples, and limitations. A failed
  test used for subsequent tuning becomes development evidence; reserve new test
  material for the next independent evaluation.
- [ ] Regenerate a bounded representative corpus range, review source images and
  corpus diffs, check entry boundaries and language metadata, and run `validate`.
- [ ] Commit implementation and reviewed generated corpus changes as separate
  milestones where practical. Record the completed run ID and commit hashes.
- [ ] Update README and language coverage documentation to describe actual
  behavior and measured support. Decide whether broader processing is justified.

## Reference documentation

- [Tesseract image quality, borders, and segmentation modes](https://tesseract-ocr.github.io/tessdoc/ImproveQuality.html)
- [Kraken recognition training](https://kraken.re/main/user_guide/training_recognition.html)

Check documentation against pinned tool versions before implementing commands.

## Session handoff log

Append an entry at each session boundary and update task checkboxes above.
Record partial work honestly; do not mark a corpus milestone complete based only
on code checks.

```text
Date / session:
Milestone and completed checkboxes:
Commits:
Source, configuration, model, and run identities:
Commands and verification results:
Accuracy comparison and inspected source examples:
Decisions and rejected alternatives:
Unfinished processes or uncommitted changes:
Blockers or missing sample coverage:
Exact next action:
```

### 2026-09-05: first measurement milestone

- Coordinator delegated source auditing to Sol, scoring implementation to Terra,
  and cached baseline provenance to Luna. Builds and verification are serialized.
- Audited ten source pages, corrected pilot script assertions, and froze page
  partitions in `benchmarks/sample-inventory/robinson-1854-splits.toml`.
  Inventory and reproducible raster identities are in the adjacent audit document.
  Commits: `2ebf89a`, `8a7e2cf`.
- No new verified gold lines yet. Broad candidate regions are not line anchors.
  Validation lacks Ethiopic; historical Aleph glyphs have no held-out sample.
  Unaudited pilot pages remain explicitly unverified. Milestone A stays open.
- Clean source build at `9eb6ad3` reproduced the existing cached fused result:
  CER 0.14707750952986023, WER 0.22163120567375885. This evaluates old OCR
  artifacts, not a fresh OCR run or a representative baseline. Report commit:
  `a8614fa`, under `benchmarks/baselines/`. Cached stages
  with incompatible line IDs cannot be compared as transcription accuracy.
- Fixed the fixture runner's obsolete SQLite schema path in `de6d32a`; loading
  the current v2 schema succeeded in the pinned Nix environment.

Next sampling action: transcribe and independently source-check exact lines from
the frozen development and validation pages, add coordinate anchors and resolve
uncertain marks. Record numeric acceptance tolerances before recognition tuning.

Scoring implementation verified and committed as `c8906ee`:

- Added optional same-page source rectangles, matching asserted source/coordinate
  frame identities, image-dimension checks, and explicit line-boundary text policy.
  Split/merged hypotheses are scored once in declared ALTO reading order. Legacy
  line-ID fixtures remain explicitly identified; old exact CER/WER are unchanged.
- Added dynamic reference-script counts (including Phnx), alphabetic base and
  base-aligned mark diagnostics with linear working memory. Filtered per-script
  CER is still diagnostic; aligned script confusions, exact foreign-word scores,
  separate segmentation/entry metrics, and oracle-candidate scoring remain open.
- Checks: `nix develop path:. --command cargo fmt --all --check`;
  `nix develop path:. --command bash -euo pipefail -c
  './tool/test-fixture-pipeline.sh && cargo build --locked'`.
  Passed 72 core unit tests, 23 fixture tests with external TEI validation,
  warning-denied workspace/all-target Clippy, XML schema and SQLite v2 checks.
  CLI smoke reproduced legacy exact CER/WER, accepted matching asserted identity,
  and rejected an incorrect PDF page without emitting metrics.
- Initial integration exposed compile errors and review found line-boundary,
  coordinate-frame, mark-association and memory-cost issues; these were corrected
  before the final passing checks. The scoring agent reached its usage limit
  after handoff, so the coordinator completed integration and final regressions.
- No OCR generator or corpus mutation ran. Candidate-source inventory is not gold;
  no recognition-accuracy improvement or release readiness is claimed. The next
  milestone must source-check line transcription and add real anchored fixtures.

Handoff: no unfinished processes or generated corpus changes. All implementation
and audit files are committed; this plan update records the remaining work.

### 2026-09-05: first anchored transcription draft

- Continued the sampling gate with twelve consecutive right-column lines from
  printed page 50 / PDF 66, a frozen development page. Added benchmark-shaped
  coordinate drafts, twelve exact source crops and a per-line review manifest
  under `benchmarks/transcription-drafts/robinson-1854-p050/`.
- Recomputed the source PDF and existing 180 dpi page hashes; both match the
  inventory. Direct Poppler 25.10.0 rendering at 360 dpi produced a 1855 × 3139
  raster, SHA-256 `14c4a651784c5ef7ac01fe7ab5b3f14cbfa21aedfa71564c9eb4f78c299f057e`.
  Exact commands, frame identity, crop bounds and crop hashes are retained.
- Inspected full-page context, native higher-resolution region, enlarged view,
  and the assembled individual crops. Drafted directly from the scan without OCR
  or dictionary proposals. Coverage: 12 Latn lines, 5 also Hebr, 1 also Grek.
  Recorded six lines needing particular point/accent/spelling checks, including
  apparent defective Hebrew plurals; all twelve still require second review.
- These are explicitly drafts outside the gold directory. No independent second
  reviewer was available in this session. Accepted new gold count remains zero;
  no measurement checkbox is marked complete. No final-test OCR was inspected.
- Verification: source/raster hashes matched; JSON, frozen development membership,
  twelve unique ordered in-bounds non-overlapping anchors, matching crop dimensions
  and hashes, and absence of Unicode directional controls checked. `git diff
  --check` passed. This is a data/documentation change; no Cargo rebuild, generator,
  corpus mutation or recognition-accuracy measurement ran.
- Exact next action: obtain independent source review of these twelve lines and
  resolve the recorded uncertainties before promotion; prepare the development
  page 700 rare-script region and validation page 175 using the same evidence
  format. The representative sample target, scoring policy and numeric acceptance
  tolerances remain open. No unfinished processes.

### 2026-09-05: transcription review form

- Added **Transcription review** to `cargo run -- review serve`, available at
  `/transcriptions`. The form loads the twelve page-50 drafts, shows source crops
  with zoom, and saves an independent reading before exposing draft text or
  uncertainty notes. Reviewers then record a resolved/unresolved decision.
- Added a separate append-only `transcription-reviews.jsonl` beside the configured
  corpus patch file. Records preserve source identity, reviewer, timestamp,
  immutable first reading, final text, notes and optimistic revision. Crop hashes
  are verified; changed source/draft metadata invalidates previous review state.
  Only development/validation-labelled samples are offered. Gold promotion
  remains a separate audited action; no real review or corpus mutation occurred.
- Verification: pinned Nix formatting, 75 core unit tests, 23 fixture tests with
  external TEI validation, warning-denied workspace/all-target Clippy, XML schema
  and SQLite checks, and locked build passed. New regressions exercise source-first
  gating, immutable readings, stale saves, persistence, source changes, crop
  tampering, reviewer consistency, unresolved notes and held-out sample exclusion.
- Local HTTP checks passed for hidden drafts, crop bytes, save/reveal, stale 409,
  resolution and journal history. Firefox control-level checks passed for loading
  the crop, entering an independent reading, revealing comparison, resolving and
  restoring progress after reload. Inspected the rendered form. Initial browser
  harness attempts needed an existing profile directory and explicit waits for
  asynchronous data/image loading; final checks passed. JavaScript syntax passed
  Node's parser. Test journals/profiles were isolated in temporary directories;
  test servers and browsers stopped.
- Next action: use the form for actual independent review of page 50; resolve
  flagged readings, then promote accepted lines with their review evidence. The
  broader sample, scoring policy and acceptance-tolerance work remains open.

### 2026-09-05: Unicode transcription keyboard

- Added an offline on-screen keyboard to the transcription form, with seven
  language/script palettes and 807 named Unicode keys: Hebrew/Aramaic square
  script, Arabic/Persian, Syriac, polytonic Greek, Ethiopic, Phoenician, and Latin
  transliteration. Hebrew cantillation and script punctuation are selectable;
  vowel points remain visible beside the letter palette.
- Keys insert at the active transcription/notes caret. Combining marks attach to
  the preceding or selected letter without inserting the dotted-circle preview.
  Added name/glyph/code-point search, scalar code-point entry, one-code-point
  backspace and keyboard undo. Surrogate pairs remain intact; keyboard switching
  does not change semantic language or normalize diplomatic text.
- Verification: five JavaScript regression groups passed, covering mark placement,
  literal multi-script sequences, selections, astral glyphs, invalid code points,
  and palette contents. Pinned Nix formatting, 75 core tests, 23 fixture tests,
  external TEI validation, strict Clippy, XML/SQLite checks and locked build passed.
  Firefox real-click tests passed for Hebrew points, Greek accents, Arabic/Syriac
  vowels, Phoenician deletion/undo, notes targeting, search, code-point insertion,
  mixed-script save/reveal/resolution and reload persistence. Inspected the
  rendered keyboard. Browser tests used temporary review data; no real draft was
  reviewed and no corpus/gold file changed. Test processes stopped.
- Next action remains independent source review through the form, followed by
  audited gold promotion and the broader representative measurement work.

### 2026-09-05: aligned scoring and Unicode policy

- Continued scoring independently while the user completes page-50 source review.
  At session start, the first line was resolved at revision 2 and matched the
  current draft/source digest. The live `transcription-reviews.jsonl` is user work;
  it remains untouched and excluded from these implementation commits.
- Completed the script/foreign-token diagnostic and normalization-policy items
  in B. Added full-stream minimum-edit script confusion counts and exact
  foreign-containing whitespace-token accuracy/precision, retaining punctuation,
  pointing, introduced scripts and explicit zero-support counts. Existing
  base-letter/mark CER remain diagnostics with their documented limits; they are
  not calibrated probabilities of correct recognition.
- Added separate NFC CER/WER with normalized denominators; diplomatic CER/WER
  remain unchanged. Documented historical-glyph exclusions and the legacy Aleph
  mapping limitation. Defined initial numeric no-regression tolerances (0 increase)
  before tuning; missing sample coverage still prevents adoption claims.
- Implemented deterministic Hirschberg alignment with linear working storage;
  tested minimum cost and index conservation exhaustively on short strings,
  asymmetric inputs, wrong-script swaps, absent/introduced scripts, rare scripts,
  moved/missing marks, punctuation, canonical equivalence, compatibility glyphs,
  and old-report deserialization. Checks passed: 84 core tests, 23 fixture tests
  with external TEI validation, warning-denied workspace/all-target Clippy,
  formatting, XML schema/SQLite checks, and locked build in pinned Nix.
- Source-built CLI re-evaluation of cached page-17 fused ALTO reproduced exact
  CER 0.14707750952986023 and WER 0.22163120567375885 with no missing line IDs.
  The new strict foreign-token diagnostic finds 7 exact matches / 93 reference
  tokens; this is an old, legacy-line-ID, unverified-source benchmark comparison,
  not representative accuracy or a new OCR run. A separate provenance report
  records the diagnostic snapshot after the implementation commit.
- No OCR generator, corpus change, gold promotion or final-test evaluation ran.
  Next independent scoring tasks: segmentation/entry-boundary measures, comparable
  stage reports, and oracle-candidate scoring. Representative sampling and baseline
  work remain open while source reviews continue.
