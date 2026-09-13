# Synthetic corpus scaling and fine-tuning at 274 reviewed pairs, 2026-09-13

## Outcome

Two things are settled by this run, and they point in opposite directions.

Scaling the synthetic pretraining corpus five-fold and frequency-balancing it
bought nothing. Fine-tuning on reviewed pairs bought twenty-nine points. The
recognizer's accuracy lives in the reviewed data, not in the renderer.

All figures are character accuracy from one method — `ketos test --arch vgsl -f
path --reorder --base-dir auto -u NFC --pad 16`, Kraken 7.1 — on
`development-24`, the 24 reviewed development-partition headwords. That set is
the only reviewed material no model here has fitted or selected on. Pages 250
and 775 remain untouched.

| System | Character accuracy | Errors | Exact (word) |
| --- | --- | --- | --- |
| Tesseract isolated word pass, in the pipeline | 30.49% | 114 | 8.00% |
| Synthetic only, 50k balanced, no real data | 42.07% | 95 | 8.00% |
| Fine-tune: 10k uniform pretrain + 129 real | 65.24% | 57 | 16.00% |
| Fine-tune: 10k uniform pretrain + 274 real | 69.51% | 50 | 20.00% |
| **Fine-tune: 50k balanced pretrain + 274 real** | **71.34%** | **47** | 16.00% |

## What each change was worth

Running two pretraining bases through the same fine-tune isolates the
contributions:

- **Fine-tuning at all: +29.3 points.** The same pretrained model goes from
  42.07% to 71.34% once it sees 274 reviewed pairs.
- **Doubling the real fitting set, 129 to 274, pretrain held fixed: +4.3
  points**, 65.24% to 69.51%.
- **Five times the synthetic data plus frequency balancing, real set held
  fixed: +1.8 points**, 69.51% to 71.34%.

That last number is three characters out of 164, and the two bases swap places
depending on which metric is read: the 10k uniform base is *better* on exact
matches, 20.00% against 16.00%, and on the frozen 30, 76.72% against 75.13%.
The corpus work is not distinguishable from noise.

This refutes a specific hypothesis. The 2026-09-12 report observed that the
value of pretraining is largely invisible in the synthetic-only number and
warned against selecting a pretraining budget on it. That was a reasonable
argument and it is why both bases were fine-tuned here rather than only the new
one. It does not hold: the balanced corpus was worth nothing before fine-tuning
and nothing after it.

## The synthetic-only model does not beat the pipeline baseline

Measured on more than thirty crops, the central claim of the previous report
does not survive.

| Evaluation set | Pairs | Characters | Tesseract | Synthetic only, 50k balanced |
| --- | --- | --- | --- | --- |
| frozen-30 | 30 | 189 | 44.44% | 62.96% |
| validation-46 | 46 | 304 | 44.08% | 56.91% |
| development-24 | 24 | 164 | 30.49% | 42.07% |
| train-part-274 | 274 | 1889 | 44.04% | 41.87% |
| **heldout-298** | 298 | 2053 | **42.96%** | **41.89%** |

On 298 held-out pairs the synthetic-only recognizer scores 41.89% against
Tesseract's 42.96%. The apparent advantage runs from +18.5 points on 30 crops
to -1.1 points on 298. A 189-character evaluation set was hiding a twenty-one
point gradient.

Note also that Tesseract is nearly flat across frozen-30, validation-46 and
train-part-274 at 44.4%, 44.1% and 44.0%. The frozen 30 was representative for
the baseline while being badly unrepresentative for the recognizer, so its
stability was not evidence that it was a sound evaluation set.

## The kaf hypothesis is refuted

The previous report found 8 errors on 9 kaf occurrences, attributed them to the
corpus's natural frequency skew, and predicted that frequency-balanced sampling
would collapse them. Balancing was implemented and works: `--balance 0.75`
moves the bet/kaf character ratio from 2.29 to 1.86, and raises word-initial
kaf-with-dagesh samples from 1,539 to 1,817 at equal corpus size. Counted the
same way, the previous run's 10,000-sample slice held 295 such samples against
1,817 here, a 6.2-fold increase in exposure, of which balancing contributes
about a fifth and corpus size the rest. (The previous report's count of 28 used
a narrower configuration than the one measured here; the comparison above is
one consistent definition throughout.)

Kaf read as bet, on 298 held-out pairs containing 25 kaf occurrences:

| Corpus | Kaf read as bet |
| --- | --- |
| 10k uniform | 2 |
| 50k balanced | 3 |

Kaf/bet was never a significant error mode. Every kaf in the 30-crop validation
set happened to be word-initial kaf with dagesh, and that accident produced a
confident causal story about frequency priors which a larger sample dissolves.
A six-fold increase in exposure fixed a defect that was not there.

## Error structure

For the selected model on `development-24`, 47 errors:

- **40 of 47 errors involve a vowel point**; only 7 are letter-only.
- The largest single confusions are qamats inserted (5), segol dropped (4),
  hiriq dropped (4), qamats dropped (4).
- Dropped and inserted points outnumber point *substitutions*, so the failure
  is as much detecting that a mark is present as identifying which mark it is.

This is the residual the acceptance target has to clear, and it is a
small-marks-in-a-degraded-scan problem, not a letterform problem. More
synthetic letterforms will not address it.

## Method

Corpus: `artifacts/synthetic-hebrew-v2-balanced`, 50,000 training and 2,000
validation renderings, seed 42, `--balance 0.75`, from
`tool/render-synthetic-hebrew.py`. Alpha 0.75 rather than 1.0 was chosen by
simulation: it yields more word-initial kaf-with-dagesh exposure (2,714 against
2,645 clusters) *and* retains more vocabulary (8,445 against 7,711 distinct
words), because alpha 1.0 over-concentrates on words carrying ultra-rare
clusters that are not kaf. Zero duplicate images, disjoint train/validation
vocabularies, all 41 scalars covered.

Real data: 344 reviewed headwords, all outstanding review completed
2026-09-12. 274 training-partition, 46 validation-partition, 24 development.
Exported by `gesenius export-headword-training` from a bounded review root and
by `tool/export-development-headwords.py` for the partition that command
deliberately skips. The frozen 30 is a byte-identical subset of the validation
partition throughout, so continuity with earlier experiments holds.

Pretraining selected checkpoints on the 46 real validation pairs rather than
the synthetic score, which is the flaw the previous report identified in its own
method. Real transfer rose 55.6% to 68.8% by epoch 4 and then oscillated for
ten epochs without improving; early stopping ended the run at epoch 15 after
about 13 hours. The 30-epoch cap was never binding, unlike the previous run.

Fine-tuning used `--resize union` so the codec admits the space scalar, which
the single-word synthetic corpus never renders and which occurs 6 times across
5 multi-word headwords.

Commands, digests, results and the corrected prior baselines are in
`artifacts/synthetic-pretrain-2026-09-12-50k-balanced-seed42/`.

## Never report Kraken's internal validation score

Kraken's own score overstated independent test in every case measured:

| Model | Internal score | `ketos test`, same pairs |
| --- | --- | --- |
| Synthetic only, 50k balanced | 68.75% | 56.91% |
| Fine-tune from 50k balanced | 89.47% | 72.04% |
| Fine-tune from 10k uniform | 87.50% | 69.74% |

This is the same substitution that inflated the 2026-09-12 report, where the
121-pair row recorded 14.81% — that run's `best_0.1481` checkpoint score —
against 12.70% on independent test. The internal metric excludes characters
outside the trained alphabet and runs on the compiled dataset's preprocessing.
It is a selection signal, not a result.

## Next

- **Review is the highest-yield activity available.** Doubling the reviewed
  fitting set returned +4.3 points, a real effect; five times the synthetic
  data returned +1.8, within noise on this evaluation set. The next experiment
  worth running is more reviewed pairs, not a better renderer.
- **Target the pointing, not the letterforms.** 85% of residual errors involve
  vowel points, and dropped/inserted marks outnumber substitutions. Crop
  geometry, resolution below the baseline, and binarization are the places to
  look.
- **The held-out set is now thin.** Fitting on the 274 training-partition pairs
  leaves only 24 clean pairs and 164 characters, where two points is three
  characters. Reserve additional reviewed pages for evaluation before the next
  fine-tune, or the next comparison will be unmeasurable.
- **Acceptance is far off.** 98% exact pointed implies about 99.7% character
  accuracy. This run reaches 16-20% exact on clean data. That is a recognizer
  worth routing into review, not an accepted one.
- Pages 250 and 775 remain untouched and are still the only source of a clean
  final estimate.
