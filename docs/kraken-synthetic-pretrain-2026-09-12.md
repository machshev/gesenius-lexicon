# Synthetic pretraining for pointed headwords, 2026-09-12

DRAFT: the final fine-tuning numbers from the 30-epoch run are still pending.
Everything else below is measured and final.

## Outcome

Synthetic pretraining works. A recognizer pretrained only on rendered pointed
Hebrew, with no real training data at all, reads the Robinson 1854 headword
crops better than the Tesseract pass currently in the pipeline. Fine-tuning that
model on the reviewed real pairs roughly halves the baseline's error count and
produces the first checkpoint in this project to read any headword exactly
right, pointing included.

All figures are character accuracy on the frozen 30-crop, 189-character
validation set unless stated otherwise.

| System | Character accuracy | Errors | Exact pointed |
| --- | --- | --- | --- |
| Tesseract isolated word pass, in the pipeline | 44.44% | 105 | 0 / 30 |
| Kraken VGSL, 47 real fitting pairs, seed 42 | 12.17% | 166 | 0 / 30 |
| Kraken VGSL, 121 real fitting pairs, seed 42 | 14.81% | 161 | 0 / 30 |
| Synthetic only, no real data, epoch 15 | 68.78% | 59 | 6 / 30 |
| Synthetic epoch 8 + 2 epochs on 129 real pairs | 75.13% | 47 | 5 / 30 |

The decomposition matters more than any single number. The same 129 real pairs
that reach 14.81% on their own reach 75.13% when they fine-tune a synthetically
pretrained model. The reviewed data was never useless; there was never enough of
it to learn letterforms from scratch. Synthetic rendering supplies the
letterforms, the reviewed pairs supply the domain correction, and neither alone
is sufficient.

## Method

Corpus: `artifacts/synthetic-hebrew-v1`, 50,000 training and 2,000 validation
renderings at seed 42, from `tool/render-synthetic-hebrew.py`. Word list is
8,985 Strong's Hebrew lemmas from `tool/fetch-hebrew-wordlist.py`. This run used
the first 10,000 training samples; CPU-only Torch at about 25 samples/s made the
full corpus a 17-hour proposition.

Real data: `real-export` inside the experiment directory, exported from the
reviewed queue and pinned so concurrent review work could not shift it. 129
fitting pairs, 832 scalars; 30 validation pairs, 189 scalars. The validation
split is byte-identical to the one used by the 2026-09-11 and 2026-09-12
experiments, so the comparison above is like for like.

Commands, input digests, the transfer curve and the baseline report are in
`artifacts/synthetic-pretrain-2026-09-12-10k-seed42/`. Weights remain external.

## The pinned Kraken version is load-bearing

The first attempt failed with `format_type path not in [xml, page, alto,
binary]`. Kraken 7.0.2 advertises `--format-type path` and then raises from a
recognition data module that never handles the case; 7.1 handles it. Because
`execute_kraken_training` resolves `ketos` from `PATH`, a stale environment
silently substitutes the broken version and the repository's own documented
`train-prepared` command fails with a trace that names neither the version nor
the fix. `training.rs` now checks the version before spending a run.

## Transfer curve

Each pretraining checkpoint was tested against the real validation set while
training continued. Synthetic validation accuracy is in-domain and saturates
early; real transfer is the number that matters.

| Epoch | Synthetic validation | Real crops | Real errors |
| --- | --- | --- | --- |
| 1 | 46.85% | 35.45% | 122 |
| 2 | 80.89% | 44.97% | 104 |
| 3 | 89.29% | 57.67% | 80 |
| 5 | 96.05% | 64.55% | 67 |
| 8 | 98.12% | 66.67% | 63 |
| 12 | 98.72% | 69.31% | 58 |
| 15 | 99.12% | 68.78% | 59 |

Real accuracy passed the Tesseract baseline at epoch 2 and was still improving
at epoch 15 while synthetic validation sat above 99%.

Two cautions this curve taught. Single-epoch movements of four points or less
are noise on 189 characters: a dip at epochs 6 and 7 looked like saturation and
was not, and a decision to stop training early was nearly taken on it. And
Kraken selects its best checkpoint on the *synthetic* score, which keeps
improving after real transfer has peaked, so the checkpoint it labels best need
not be the best starting point for real pages.

## Pretraining budget is not cheap to skip

Fine-tuning two epochs on the 129 real pairs from different pretraining depths:

| Pretraining epochs | Synthetic only, real crops | After fine-tune | Errors |
| --- | --- | --- | --- |
| 5 | 64.55% | 67.20% | 62 |
| 8 | 66.67% | 74.60% | 48 |

Three extra pretraining epochs were worth two points before fine-tuning and
seven and a half points after it. The value of pretraining is largely invisible
in the synthetic-only number. Do not select a pretraining budget on it.

Note also that fine-tuning from epoch 8 gave 75.13% and 74.60% on two runs that
differed only in thread count, both seeded and `--deterministic`. Treat
differences below about half a point as noise.

## Error structure

Confusions for the 75.13% model, 47 errors:

- 8 errors are kaf read as bet, out of 9 kaf occurrences.
- About 14 errors total are letter substitutions, the rest scattered
  (ayin/nun both directions, qof as het or he, final mem as tet).
- About 25 errors are point errors: dagesh inserted or dropped, and
  hiriq, qamats, tsere, segol and sheva confused with one another.
- A few letter deletions (gimel, vav, yod).

The point confusions are the expected residual: small marks below the baseline
in a degraded scan. The kaf failure is not residual, and it is a defect in the
synthetic corpus rather than a limit of the method.

### Why kaf fails

Rendering is not at fault: every font in the mixture draws the kaf/bet
distinction correctly, kaf with a rounded corner and no foot, bet with a
protruding foot.

The synthetic corpus inherits the natural frequency skew of Hebrew, because the
renderer samples words uniformly from the lemma list. In the 10,000 training
samples bet is 5.14% of characters and kaf 2.14%, a 2.4 to 1 prior favouring
bet on an ambiguous shape. More sharply: every kaf in the validation set is
word-initial kaf with dagesh, and the training set contains 28 examples of that
exact configuration.

A fine visual discrimination, under degradation, on a configuration seen 28
times, against a competitor seen far more often, is what produces 8 failures
out of 9.

This is a hypothesis supported by the frequency data, not a demonstrated cause.
The test is to render a frequency-balanced corpus and re-measure; the prediction
is that kaf errors collapse and several points of character accuracy follow.

## Next

- Add frequency-balanced word sampling to the renderer so rare letters and rare
  letter/point configurations receive supervision out of proportion to their
  natural frequency. Re-render and re-measure against this run as the control.
- Report base-aligned mark precision and recall through the repository's own
  `benchmark-headwords` evaluator rather than Kraken's aggregate.
- The 98% exact-pointed acceptance target implies about 99.7% character
  accuracy. This experiment moves exact-pointed from 0 to roughly 1 in 5. That
  is a usable-with-review recognizer at best, not an accepted one.
- Every number here is validation performance, selected on the same 30 crops it
  is scored against. The reserved final-test pages, 250 and 775, remain
  untouched and are the only source of a clean estimate.
- The §1 validation re-check is still open. These references had their encoding
  corrected by the pointing normalization but their readings have not been
  re-verified against the crops.
