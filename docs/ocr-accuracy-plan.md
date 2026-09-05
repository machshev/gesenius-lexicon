# Multilingual OCR accuracy implementation plan

Status: planned; implementation has not started.
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
- [ ] Create a sample inventory recording edition, PDF and printed page, source
  hash, crop coordinates, actual scripts, typography, damage, and selection reason.
- [ ] Begin with roughly 150–300 verified lines across 8–12 Robinson pages,
  adjusting the sample to obtain real coverage rather than filling a quota.
  Include ordinary prose, dense comparisons, headwords, mixed-direction lines,
  Arabic/Syriac connections, pointing, Greek accents, and rare glyphs.
- [ ] Separate development, training/validation, and final test pages. Verify
  rare-script coverage in each usable partition; record coverage gaps explicitly.
- [ ] Record transcription authority and unresolved glyphs. Model-assisted
  transcription needs source checking before it becomes gold.
- [ ] Reproduce the existing benchmark using a source-built binary, then record
  source/model/configuration/code identities for the representative baseline.

### B. Improve scoring and stage comparison

Primary files: `crates/gesenius-core/src/{benchmark,metrics}.rs`, CLI benchmark
handling, and `benchmarks/gold/`.

- [ ] Add stable source-coordinate anchors or explicit alignment so comparisons
  remain meaningful when an engine splits, merges, or renumbers lines. Check
  source/page identity instead of trusting matching engine-generated line IDs.
- [ ] Retain overall CER/WER; add aligned script confusions, base-letter accuracy,
  pointing/accent accuracy, exact foreign-word accuracy, and script sample counts.
  Associate combining marks with their grapheme/base, including inherited marks.
- [ ] Define Unicode normalization and historical-glyph scoring policies. Preserve
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

Initial next action: inspect the current source and cached baseline, then visually
verify a small spread of candidate Robinson pages to establish the sample inventory
and split policy for milestone A.
