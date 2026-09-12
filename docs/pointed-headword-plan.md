# Accurate pointed Hebrew headwords

Status: measurement and review-to-training tooling are implemented. Two
from-scratch Kraken VGSL experiments failed: 47 fitting pairs reached 19.05%
independently measured character accuracy and 0% exact-word accuracy, and
repeating with 121 fitting pairs reached only 14.29% and 0% on the unchanged
validation set. Neither run was seeded, so that difference is not interpretable.

On 2026-09-12 a review of the plan changed its diagnosis. The bottleneck is not
believed to be the number of reviewed headwords. It is the total quantity of
character-level supervision, which manual transcription cannot reach at any
plausible budget, and a labelling defect that corrupted roughly a quarter of the
points in every reviewed set including validation. The defect is now corrected
and the next experiment is synthetic pretraining rather than further collection.
See [implementation and remaining gates](pointed-headword-workflow.md).
Complements `ocr-accuracy-plan.md`, with Robinson 1854 headwords as the first
bounded target; assess Tregelles separately.

## Recommendation

Pretrain a Hebrew recognizer on synthetic renderings of pointed Hebrew in
period-appropriate typefaces, then fine-tune it on the human-reviewed crops.
Establish trustworthy labels and measurement first, then compare the result
against the existing isolated Tesseract baseline. Training remains an
experiment, not a guarantee of correct pointing.

### Why the previous direction stalled

The right unit for a CTC recognizer is characters, not words. The 121 reviewed
fitting headwords carry 776 Unicode scalars, a mean of 6.47 scalars each. The
earlier planning budget of 500–1,000 reviewed headwords is therefore about
3,200–6,500 characters. Kraken fine-tuning conventionally expects roughly
1,000 lines, on the order of 40,000–60,000 characters; training from scratch
expects considerably more. The budget was short of a fine-tuning corpus by
about an order of magnitude and of a from-scratch corpus by roughly two.

Measured review throughput makes the manual route worse than it appears. The
journal records 382 headword review events covering 176 unique headwords in
about 147 minutes of active time, counting gaps under five minutes. That is
roughly 50 seconds per unique headword including revisions. 1,000 headwords is
therefore 8–14 hours of focused work, and the ~7,700 needed to reach 50,000
characters is over 100 hours. Both observed experiments are consistent with a
corpus far below the useful threshold rather than with a curve that more
transcription would climb.

### The baseline to beat

Measured on the 172 resolved reviewed headwords against the Tesseract
suggestions stored in their drafts:

| Metric | Value |
| --- | --- |
| Exact NFC pointed match | 2 / 172 (1.2%) |
| Consonant skeleton exact | 68 / 172 (40%) |
| Consonant character accuracy | 64.2% (35.8% CER) |

Two consequences. The bar is about 1.2% exact, not zero, so both trained
checkpoints at 0% exact-word accuracy are worse than the recognizer already in
the pipeline. And the consonant skeleton is much weaker than the page-17
example alone suggests, which limits how far dictionary-assisted labelling
(section 6) can be pushed.

The 98% exact-headword acceptance target in section 7 implies about 99.7%
character accuracy at 6.47 scalars per headword. The distance from 14.29% should
be read in those terms.

### Observed failure and integration constraints

The originally reported problem reproduces from saved index evidence:

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
- Training assigns pages by hash; the benchmark inventory has an explicit
  frozen split manifest. These policies are now reconciled through one
  authoritative split file.
- Transcription review decisions do not automatically become corpus corrections
  or gold. Preparation consumes reviewed corpus spans, so the review-to-training
  handoff must stay explicit and tested.

## 1. Correct the pointing defect before measuring anything

- [x] Audit the mark scalars in every resolved review. Across 170 resolved
  headwords, U+05C7 QAMATS QATAN appeared 72 times against 27 uses of U+05B8
  QAMATS, U+05C5 HEBREW MARK LOWER DOT 32 times against 8 uses of U+05B4 HIRIQ,
  and U+05BA HOLAM HASER FOR VAV 25 times against 9 uses of U+05B9 HOLAM. That
  is roughly 129 of 480 marks, about 27%, on wrong scalars.
- [x] Identify the mechanism. The `Vowel points` palette in
  `crates/gesenius-core/src/review/transcription-keyboard.js` offered U+05BA,
  U+05C4, U+05C5 and U+05C7 in the same row as the points the edition prints.
  At review size they render identically to holam, the shin dot, hiriq and
  qamats. The over-used scalars are exactly the lookalikes adjacent to the
  correct keys.
- [x] Remove those four keys from the palette and assert their absence in
  `tool/test-transcription-keyboard.cjs`. The remaining sixteen vowel points are
  all marks the edition actually prints.
- [x] Re-encode the affected reviews with
  `python3 tool/apply-pointing-normalization.py`. 135 points across 102 reviews
  were corrected; the audit is in `corpus/review/pointing-normalization.json`.
  Each correction appends a revision carrying the original reviewer, state and
  timestamp, and prior revisions stay in the journal.

The substitution is a re-encoding, not a re-reading. The edition prints no glyph
distinguishing qamats qatan from qamats or holam haser for vav from holam, and
U+05C5 is Masoretic punctuation rather than a vowel, so a diplomatic
transcription of these pages cannot license any of them under the project's own
no-inference policy. The reviewer's judgement of which mark is present stands.

After correction the mark distribution matches ordinary Biblical Hebrew — qamats
101, patah 70, dagesh 64, sheva 50, hiriq 43 — where before it did not. The
export is unchanged at 121 fitting and 30 validation pairs with 776 and 189
scalars, and `alphabet-audit.json` no longer reports code points the edition
never prints.

- [ ] Re-check the 30 validation labels against their crops. The frozen
  validation set was affected, so the 19.05% and 14.29% figures were measured
  partly against incorrect references and are not a clean baseline for the next
  comparison. Treat both as superseded rather than as a learning curve.

## 2. Synthetic pretraining

This is the next experiment. It changes one variable, needs no new
transcription, and is evaluated on the existing frozen validation set. The
renderer is implemented; the training run is not yet done.

- [x] Render pointed Hebrew word images from open text sources. Labels are exact
  by construction. `tool/fetch-hebrew-wordlist.py` builds the word list from the
  Strong's Hebrew lexicon headwords (8,985 distinct pointed lemmas over 41
  scalars, covering every scalar in the reviewed fitting and validation sets),
  with the Westminster Leningrad Codex available for breadth. Cantillation,
  meteg and rafe are stripped, since the reviewed headwords contain none.
- [x] Use a font *mixture*, not a single face. A single-font synthetic model
  overfits to that face and transfers poorly. `tool/render-synthetic-hebrew.py`
  weights the seven fonts below and verifies each covers the word list's charset
  before rendering, because Pango substitutes silently for a missing glyph.
- [x] Match scale to the real crops: headword ink height including points above
  and below is median 53 px, p10-p90 42-68 px, on a 2061x3488 page raster. The
  renderer samples a triangular height over that range per image.
- [x] Randomize degradation per sample: rotation, blur, ink spread, contrast,
  paper tone, sensor noise, JPEG artefacts and margins, each recorded per sample
  in the manifest. The degradation model matters more to transfer than the font
  choice.
- [x] Produce the first corpus. `artifacts/synthetic-hebrew-v1` holds 50,000
  training and 2,000 validation samples at seed 42, rendered in 316 seconds and
  occupying 548 MB. Its training split carries 348,709 scalars, a mean of 6.97
  each, against 776 in the reviewed fitting set: about 450 times the supervision.
- [ ] Pretrain on that corpus, then fine-tune on the reviewed crops. Seed and
  use `--deterministic` throughout.
- [ ] Report the synthetic-only and fine-tuned scores separately on the frozen
  validation set. Expect a large gain over 14.29% but not acceptance-level
  accuracy from synthetic data alone; the domain gap to a real 1850s foundry
  face is genuine.

Synthetic output is pretraining material, never gold. It must not enter the
benchmark, the review queue or the corpus, and its own validation split exists
only for early stopping: the reportable number is `ketos test` against the real
frozen validation manifest. Commands and enforced invariants are in
[the workflow](pointed-headword-workflow.md).

### Font selection

Candidates were rendered through HarfBuzz (`pango-view`, so mark positioning is
real) using the actual reviewed headword texts, normalized to matched letter
height, and scored by best-shift IoU against the binarized real crops:

| Font | Mean IoU | Source |
| --- | --- | --- |
| Frank Ruhl Libre Bold | 0.480 | google-fonts |
| Ezra SIL | 0.466 | `artifacts/fonts/` (OFL) |
| Bona Nova Bold | 0.464 | google-fonts |
| Taamey Frank CLM Bold | 0.459 | `artifacts/fonts/` (GPL+FE) |
| Keter YG Bold | 0.443 | `nixpkgs#culmus` |
| Frank Ruehl CLM Bold | 0.425 | `nixpkgs#culmus` |
| Noto Serif Hebrew Bold | 0.392 | noto-fonts |
| Cardo | 0.374 | google-fonts |
| … | | |
| David Libre | 0.243 | google-fonts |

This is a coarse proxy over seven words; treat the ordering as indicative rather
than decisive. It agrees with the visual comparison and with type history. The
1854 Hebrew shows very heavy horizontal strokes against near-hairline verticals
in wide, squat proportions — 19th-century European square Hebrew of the
Ashkenazi printing tradition. Frank-Rühl (Rafael Frank, 1908, for the C.F. Rühl
foundry in Leipzig) was drawn as a modernization of that tradition, and two
independent digitizations of it rank at the top. Ezra SIL, modelled on the
Biblia Hebraica type, is the same lineage under OFL.

Proposed mixture: Frank Ruhl Libre and Frank Ruehl CLM as the primary pair, with
Ezra SIL, Taamey Frank CLM, Bona Nova, Keter YG and Noto Serif Hebrew as
secondaries, each with weight and width jitter. Provenance and licences are in
`artifacts/fonts/README.md`. SBL Hebrew is the closest published revival of the
Vilna type but was not obtained: its advertised URL now 404s, and its licence is
free only for individual non-profit scholarly use, which is a poor fit for a
corpus whose provenance must stay redistributable.

Note that the inline body Hebrew is the same typeface at a smaller size, so body
crops are in-domain for the same recognizer.

## 3. Close the remaining confounders

These are cheap and must precede any expensive collection push.

- [ ] Fix a seed and use `--deterministic` in every accuracy run. The 47-to-121
  comparison is currently uninterpretable.
- [ ] Resolve the Kraken geometry mismatch: the pinned base weights were trained
  with baseline geometry while path-mode crop input is treated as bounding-box
  geometry.
- [ ] Inspect candidate base models for printed Hebrew suitability, licence,
  codec, input resolution and pinned-version compatibility. Do not assume
  multilingual support implies competence at pointing.
- [ ] Benchmark the present Tesseract word pass on the frozen development crops
  and test a small number of source-padding/grayscale alternatives. The page-17
  crop already contains the mark, so padding alone is not an established fix.
- [ ] Smoke-test training: verify `ketos` flags against pinned Kraken 7.1,
  logical RTL/combining-mark round-trip, crop resizing, codec expansion,
  checkpoint export and inference.

## 4. Establish a reproducible headword benchmark

- [x] Preserve the page-17 example as a development regression with image/source
  identity, rectangle, raw candidates, expected scalars, and final output.
- [x] Extend the frozen inventory with explicit training pages. Preserve existing
  validation/test restrictions and exclude printed pages 1–10 from final test.
  Make training and evaluation consume one authoritative page split; reject
  overlapping pages and duplicated crops. Do not silently rehash reserved pages.
- [ ] Audit 100–200 headwords initially across multiple pages and alphabet
  sections. Inventory every true headword on selected pages, including missed
  detections and false entry starts, rather than sampling only detected words.
  This gates every detection and end-to-end metric and is cheap relative to the
  experiments above; currently about 5 headwords per page are detected against
  an unknown true count, so detection misses are invisible.
- [ ] Label failure stage: detection, crop damage, recognition, candidate
  selection, extraction, or rendering. Keep original and processed crops.
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

## 5. Keep human review producing reliable training data

- [x] Reuse the transcription review UI for a headword queue: crop plus full-line
  context, editable logical-order Hebrew, pointing keyboard, crop adjustment, and
  an explicit uncertain/unreadable outcome.
- [x] Restrict the pointing keyboard to marks the edition prints (section 1).
- [ ] Display expected Unicode scalars on demand to distinguish visually similar
  points and reveal stray bidi controls. The scalar display exists but did not
  prevent the U+05C7/U+05C5/U+05BA defect, so it is not sufficient on its own;
  consider surfacing the scalar inline rather than on demand.
- [ ] Require a human to inspect every training label against its crop. Independently
  check validation/test labels and ambiguous marks. Do not invent reviewer
  approvals or accept model-generated suggestions as reviewed ground truth.
- [ ] Transcribe printed vowels, dagesh/mappiq, shin/sin dots and other present
  marks; exclude adjacent asterisks, grammar labels and homograph numbers using
  a documented boundary policy. Do not infer vowels from a dictionary.
- [x] Provide a deliberate promotion/export path from accepted review decisions
  to training samples, preserving reviewer, revision, source/crop hashes and
  split. Reject stale crops, unresolved labels and mismatched image/text pairs.
- [ ] Report alphabet coverage for TRAINING separately from validation/test.
  Audit marks actually used by the edition; the broad U+0591–U+05C7 audit range
  is not a requirement to manufacture examples for every code point.

Human review is now a supplement to synthetic data rather than the primary
corpus. Prioritize it by observed error class and model disagreement, and keep
enough representative validation material to select checkpoints. Include short
lemmas, visually similar vowel pairs, multiple points per letter, clean/damaged
print, and genuinely unpointed headwords to measure hallucinated vowels. Enlarge
the final test to several hundred headwords on reserved pages if needed for a
useful estimate; avoid repeatedly consulting it while tuning.

Deliverable: human-reviewed, auditable image/text pairs and isolated manifests.

## 6. Scale real labels without proportional typing

Pursue these only if section 2 shows the curve is genuinely label-limited.

- [ ] Harvest inline Hebrew, not only headwords. The fused ALTO carries about 60
  Hebrew tokens per page against 5–10 detected headwords, roughly 70,000 Hebrew
  word images across the book. The `hebrew-word` queue already generates these.
- [ ] Auto-label quoted Hebrew from its printed citation. The lexicon prints the
  reference beside the quotation, so the reference can be parsed, the verse
  pulled from WLC, and the OCR skeleton aligned to the verse's words to obtain
  pointed text without typing. Measure the agreement rate on 100 human-checked
  examples first: WLC follows Leningrad while an 1854 lexicon follows van der
  Hooght, so pointing differs occasionally, and te'amim must be stripped. Decide
  from that sample whether the aligned labels are gold or merely proposals.
- [ ] Consider dictionary-proposed candidates for the headword queue, matching
  the OCR skeleton against a pointed lemma list under the constraint of
  alphabetical position, with the reviewer confirming against pixels by keypress
  and an escape to free typing. At 40% skeleton-exact and 65% within one edit,
  candidate generation is viable but not reliable enough to accept unchecked.
  If adopted, restrict suggestions to fitting data: validation and final test
  stay blind-typed with no suggestion shown, so measurement is uncontaminated.
  This narrows, and does not abandon, the no-dictionary-inference rule — a
  suggestion a human rejects against the pixels is not an inferred vowel.
- [ ] Evaluate a vision model as a label proposer by scoring exact NFC match on
  the 30 validation crops. Adopt it under the same fitting-only guardrail if it
  scores well; drop it if it does not. Settle this by measurement, not
  assumption.

## 7. Integrate the winning recognizer into headword parsing

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

## 8. Acceptance and rollout

Proposed targets to freeze before tuning: at least 98% exact pointed-headword
accuracy, equivalently about 99.7% character accuracy, and 99% headword
detection recall on representative reserved pages; report false detections, mark
errors, counts and uncertainty alongside them. These are engineering targets,
not measured results or promised outcomes. Require an improvement over the
strongest baseline, currently 1.2% exact, without concealing consonant or
rare-mark regressions. Sparse categories remain explicitly unverified.

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

## Ordering

1. Pointing correction and validation re-check (section 1). Done except the
   validation re-check, which blocks trustworthy selection.
2. Confounders: seed, determinism, geometry, base-model survey (section 3).
3. Synthetic pretraining and fine-tuning on the existing 121 pairs (section 2).
   This is the highest-information experiment available and needs no new
   transcription.
4. Complete-page inventory (section 4), which unblocks detection metrics.
5. Only if 3 shows the curve is label-limited, scale labels through section 6
   rather than through proportional manual typing.

A full-book OCR rerun remains unnecessary until a bounded experiment
demonstrates improvement.
