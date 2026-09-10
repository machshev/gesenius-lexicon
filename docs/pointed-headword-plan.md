# Accurate pointed Hebrew headwords

Status: first-milestone tooling implemented, 2026-09-10. No training or reviewed
corpus changes were performed. See [implementation and remaining gates](pointed-headword-workflow.md). Complements `ocr-accuracy-plan.md`, with Robinson
1854 headwords as the first bounded target; assess Tregelles separately.

## Recommendation and observed failure

Fine-tune a dedicated Kraken headword recognizer on manually source-reviewed
word crops. First establish trustworthy labels and measurement, then compare
the trained recognizer against the existing isolated Tesseract baseline.
Training is justified as an experiment, not a guarantee of perfect pointing.

The supplied local URL was unreachable from this session. Saved index evidence
does reproduce the reported problem:

- Entry `robinson-1854:p1:e0002`, PDF page 17 / printed page 1, has `אב`.
- Its cached run is
  `d345bc9e6ec9ef1fcd6e37843839872dde1a401d8fca3a7b07163c45de34aeb5`.
  Under `.cache/gesenius/runs/<run>/robinson-1854/page-0017/`,
  `tesseract-word-recognitions.json` identifies `line_81-fused-word-0001`.
- Visually inspected `processed.png`, `words/word-00012.png`, and
  `words/word-00012-padding-0.png`: the qametz beneath aleph is visible in
  both actual OCR crops. The source reading is `אָב` (U+05D0 U+05B8 U+05D1).
- The isolated recognizer selects `אב` plus a direction mark with confidence
  0.92256594. The qametz is already absent before headword extraction.
  This example is a recognition failure, not evidence of a parser stripping
  U+05B8 or a crop clipping it. Other examples may have different causes.
- `alto.rs` preserves combining marks when extracting headwords and pads
  headword training geometry. Existing tests include pointed headwords.

Important integration constraints found in current source:

- Fast index mode explicitly skips Kraken (`README.md`, `pipeline.rs`).
- Full mode's configured Kraken contribution is restricted to weak Roman words
  on Roman lines (`pipeline.toml`). Merely replacing its model cannot fix this
  Hebrew headword path.
- `training.rs` already exports reviewed headword crops, manifests, and an
  alphabet audit, and invokes Kraken with NFC, reordering, and codec union.
  It currently mixes headword and line samples in its training manifests.
- Training assigns pages by hash; the benchmark inventory has an explicit
  frozen split manifest. These policies must be reconciled before fitting.
- Transcription review decisions do not automatically become corpus corrections
  or gold. Preparation consumes reviewed corpus spans, so the review-to-training
  handoff must be explicit and tested.

## 1. Establish a reproducible headword benchmark

- [x] Preserve the page-17 example as a development regression with image/source
  identity, rectangle, raw candidates, expected scalars, and final output.
- [ ] Audit 100–200 headwords initially across multiple pages and alphabet
  sections. Inventory every true headword on selected pages, including missed
  detections and false entry starts, rather than sampling only detected words.
- [ ] Label failure stage: detection, crop damage, recognition, candidate
  selection, extraction, or rendering. Keep original and processed crops.
- [ ] Extend the frozen inventory with explicit training pages. Preserve existing
  validation/test restrictions and exclude printed pages 1–10 from final test.
  Make training and evaluation consume one authoritative page split; reject
  overlapping pages and duplicated crops. Do not silently rehash reserved pages.
- [ ] Freeze source identities, normalization policy, labels, code/model/config
  hashes, and baseline outputs before experiments.

Report exact NFC pointed-headword accuracy as the primary transcription metric.
Also report consonant accuracy, base-aligned mark precision/recall (including
extra marks), qametz-specific omissions/substitutions, counts by mark, detection
precision/recall, and end-to-end correctly detected AND exactly transcribed
headwords divided by all true headwords. Missing entries count as failures.
Show counts and page-level uncertainty; aggregate CER alone is insufficient.

Deliverable: versioned benchmark manifest and baseline report. This separates
recognizer performance on correct crops from complete parsing performance.

## 2. Make human review produce reliable training data

- [ ] Reuse the existing transcription review UI for a headword queue: enlarged
  crop plus full-line context, editable logical-order Hebrew, pointing keyboard,
  crop adjustment, and an explicit uncertain/unreadable outcome.
- [ ] Display expected Unicode scalars on demand to distinguish visually similar
  points and reveal stray bidi controls. Save diplomatic text and NFC separately;
  do not reverse stored strings to achieve RTL display.
- [ ] Require a human to inspect every training label against its crop. Independently
  check validation/test labels and ambiguous marks. Do not invent reviewer
  approvals or accept model-generated suggestions as reviewed ground truth.
- [ ] Transcribe printed vowels, dagesh/mappiq, shin/sin dots and other present
  marks; exclude adjacent asterisks, grammar labels and homograph numbers using
  a documented boundary policy. Do not infer vowels from a dictionary.
- [ ] Provide a deliberate promotion/export path from accepted review decisions
  to training samples, preserving reviewer, revision, source/crop hashes and
  split. Reject stale crops, unresolved labels and mismatched image/text pairs.
- [ ] Add a headword-only preparation option and report alphabet coverage for
  TRAINING separately from validation/test. Audit marks actually used by the
  edition; the broad U+0591–U+05C7 audit range is not a requirement to manufacture
  examples for every code point.

Start with roughly 500–1,000 reviewed training headwords across many pages,
plus separate validation material; use a learning curve to decide whether more
are needed. This is a planning budget, not a sufficiency claim. Include short
lemmas, visually similar vowel pairs, multiple points per letter, clean/damaged
print, and genuinely unpointed headwords to measure hallucinated vowels.
Enlarge the final test to several hundred headwords on reserved pages if needed
for a useful estimate; avoid repeatedly consulting it while tuning.

Deliverable: human-reviewed, auditable image/text pairs and isolated manifests.

## 3. Run a bounded recognition experiment

- [ ] Benchmark the present Tesseract word pass on the frozen development crops.
  Test a small number of source-padding/grayscale alternatives to identify cheap
  improvements. The page-17 crop already contains the mark, so padding alone is
  not an established fix.
- [ ] Inspect candidate Kraken base models for printed Hebrew suitability,
  licence, codec, input resolution and pinned-version compatibility. Compare a
  suitable Hebrew model, if available, with the configured general model; do
  not assume multilingual support implies competence at pointing.
- [ ] Smoke-test training on a small development subset before a full run:
  verify `ketos` flags against pinned Kraken 7.1, logical RTL/combining-mark
  round-trip, crop resizing, codec expansion, checkpoint export and inference.
- [ ] Train the headword specialist using explicit train/validation manifests.
  Record seed, commands, environment, base/model/data hashes, runtime and learning
  curves. Compare increasing training-set sizes and select checkpoints using
  validation exact-word and mark metrics, not training loss alone.
- [ ] Prioritize additional human reviews by observed error class and model
  disagreement, while retaining a representative validation distribution.

Kraken documents fine-tuning from existing weights, explicit validation manifests,
and codec expansion via `--resize union`. Its current online documentation can
move beyond the repository's pinned release; validate actual command support
before execution: [recognition training documentation](https://kraken.re/main/user_guide/training_recognition.html).

Deliverable: reproducible baseline-versus-specialist report, including failures.

## 4. Integrate the winning recognizer into headword parsing

- [ ] Add a separately configured Hebrew headword recognizer to both fast index
  and full processing, using the same crop geometry and Unicode policy as training.
  Keep its model identity in cache keys so a new checkpoint triggers recognition.
- [ ] Preserve raw Tesseract/Kraken word hypotheses, crop identity, selected text,
  model hash and selection reason at word level. Existing inherited line
  hypotheses are insufficient to audit a specialist headword decision.
- [ ] Select candidates using measured validation performance; never reward
  extra combining marks merely for making a string longer. Keep dictionary
  vocalizations as review suggestions unless independently supported by pixels.
- [ ] Route uncertain or conflicting readings to review. Calibrate any automatic
  acceptance threshold against exact pointed-word correctness; the observed
  92.3% confidence on `אב` is not a reliable pointing guarantee.
- [ ] Ensure human corrections survive reruns. Verify full flow from crop through
  structured headword, index, review display and exports, retaining U+05B8 and
  other marks. Regression checks should include inserted, deleted and substituted
  points, legitimately unpointed lemmas, crop-boundary points and missed entries.

Deliverable: bounded regenerated pages, source comparison and regression report.

## 5. Acceptance and rollout

Proposed targets to freeze before tuning: at least 98% exact pointed-headword
accuracy and 99% headword detection recall on representative reserved pages;
report false detections, mark errors, counts and uncertainty alongside them.
These are engineering targets, not measured results or promised outcomes.
Require an improvement over the strongest baseline without concealing consonant
or rare-mark regressions. Sparse categories remain explicitly unverified.

For automatically accepted output, target at least 99.5% exact precision on an
adequately sized independent evaluation and report its confidence interval and
coverage. If the available evidence cannot support that claim, retain review
status. Perfect final transcription requires human review of unresolved cases;
do not automatically promote an entire machine corpus to verified.

- [ ] Fix model, preprocessing, detection and selection policy before opening
  final-test results. If results inform further tuning, reserve a fresh test.
- [ ] Review bounded corpus diffs, entry boundaries and source crops; run relevant
  tests, repository checks and corpus validation before broader regeneration.
- [ ] Publish the model card, checksum, data/split identity and metrics. Keep a
  reproducible baseline for rollback. Commit implementation and reviewed corpus
  changes as separate milestones; model weights remain external artifacts.

First implementation milestone tooling is implemented: page-17 regression evidence,
a headword benchmark evaluator and review-to-training export with a unified split
policy. The representative inventory and human-reviewed batch are still pending. Then collect the first
reviewed batch and train the specialist. A full-book OCR rerun is unnecessary
until this bounded experiment demonstrates improvement.
