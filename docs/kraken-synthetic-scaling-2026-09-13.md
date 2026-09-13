# Synthetic corpus scaling and fine-tuning at 274 reviewed pairs, 2026-09-13

## Measurement method, and why it is stated first

Every figure here is character accuracy and exact-match against the
logical-order NFC reference, with recognition through Kraken's `rpred` on each
crop. This is the path a pipeline actually uses, and it is the same comparison
the Tesseract baseline has always been scored with.

That matters because `ketos test` reports systematically worse numbers for the
same weights:

| Model, on development-24 | `ketos test` | `rpred`, logical NFC |
| --- | --- | --- |
| Synthetic only, 50k balanced | 42.07% | 55.49% |
| Fine-tune, 10k pretrain + 129 real | 65.24% | 84.15% |
| Fine-tune, 50k balanced + 274 real | 71.34% | 89.63% |

The gap is not the comparison order: reordering both strings to display order
gives an identical error count. It is the inference path — `ketos test`'s
dataset preprocessing differs from `rpred`'s crop extraction and input
transforms, and produces worse predictions from identical weights.

The consequence is that mixing the two invalidates comparisons. Scoring Kraken
with `ketos test` while scoring Tesseract in logical-order NFC understates
Kraken by fourteen to eighteen points. An earlier draft of this report did
exactly that and drew a conclusion that the consistent method reverses.

## Outcome

| System, on development-24 | Character accuracy | Errors | Exact pointed |
| --- | --- | --- | --- |
| Tesseract isolated word pass, in the pipeline | 30.49% | 114 | 1 / 24 |
| Synthetic only, 50k balanced, no real data | 55.49% | 73 | 3 / 24 |
| Fine-tune: 10k uniform pretrain + 129 real | 84.15% | 26 | 11 / 24 |
| Fine-tune: 10k uniform pretrain + 274 real | 89.02% | 18 | 13 / 24 |
| **Fine-tune: 50k balanced pretrain + 274 real** | **89.63%** | **17** | **15 / 24** |

`development-24` is the 24 reviewed development-partition headwords: the only
reviewed material no model here has fitted or selected on. Pages 250 and 775
remain untouched.

The selected model reads fifteen of twenty-four development headwords exactly
right, every letter and every point, against one for the pass currently in the
pipeline.

## What each change was worth

Running two pretraining bases through the same fine-tune isolates the
contributions:

- **Fine-tuning at all: +34.1 points.** The same pretrained model goes from
  55.49% to 89.63% once it sees 274 reviewed pairs.
- **Doubling the real fitting set, 129 to 274, pretrain held fixed: +4.9
  points**, 84.15% to 89.02%.
- **Five times the synthetic data plus frequency balancing, real set held
  fixed: +0.6 points**, 89.02% to 89.63%.

That last number is one character out of 164. The corpus work bought nothing.

This refutes a specific hypothesis. The 2026-09-12 report observed that the
value of pretraining is largely invisible in the synthetic-only number and
warned against selecting a pretraining budget on it. That argument is why both
bases were fine-tuned here rather than only the new one. It does not hold: the
balanced corpus was worth nothing before fine-tuning and nothing after it.

## Synthetic pretraining alone does beat the pipeline baseline

On `train-part-274` — 274 pairs and 1,889 characters, held out entirely for a
model that fitted no real data:

| System | Character accuracy | Exact pointed |
| --- | --- | --- |
| Tesseract | 44.04% | 1 / 274 |
| Synthetic only, 50k balanced | 47.86% | 30 / 274 |

The character-accuracy margin is narrow, 3.8 points, but the exact-match margin
is not: thirty headwords read perfectly against one. Character accuracy
understates the difference between a recognizer that is usually nearly right
and one that is occasionally exactly right, which is the distinction the
acceptance target is written in.

## The kaf hypothesis is refuted

The previous report found 8 errors on 9 kaf occurrences, attributed them to the
corpus's natural frequency skew, and predicted that frequency-balanced sampling
would collapse them. Balancing was implemented and works: `--balance 0.75`
moves the bet/kaf character ratio from 2.29 to 1.86 and raises word-initial
kaf-with-dagesh samples from 1,539 to 1,817 at equal corpus size. Counted the
same way, the previous run's 10,000-sample slice held 295 such samples against
1,817 here, a 6.2-fold increase in exposure. (The previous report's count of 28
used a narrower configuration; the comparison here is one consistent definition
throughout.)

Kaf read as bet, across the 298 held-out pairs containing 25 kaf occurrences:
**4 for the synthetic-only model, 0 for the fine-tuned model.** Kaf/bet was
never a significant error mode. Every kaf in the 30-crop validation set happened
to be word-initial kaf with dagesh, and that accident produced a confident
causal story about frequency priors which a larger sample dissolves.

## Error structure

The fine-tuned model makes 17 errors on `development-24`: 9 involve a vowel
point, 8 are letter errors. The synthetic-only model, measured over all 298
held-out pairs, makes 615 point-involving and 443 letter errors, 58% points.

So pointing is the larger share but not overwhelmingly so, and fine-tuning
improves both. With only 17 residual errors on the clean set, the split is not
a reliable basis for prioritising work; a larger clean set is needed before
targeting one failure mode over the other.

## The fine-tuned models memorise their fitting set

The selected model scores **100% character accuracy and 274 of 274 exact on
`train-part-274`**, its own fitting data. A 4.0-million-parameter network
trivially memorises 274 samples.

Two consequences. `train-part-274` is not evidence about any fine-tuned model.
And the clean evidence for the selected model is `development-24` alone: 24
pairs, 164 characters, where a single character is 0.6 points and two points is
three characters. Differences of a few points between fine-tuned models here
are not measurable, which is why this report does not claim the 50k base beats
the 10k base.

## Method

Corpus: `artifacts/synthetic-hebrew-v2-balanced`, 50,000 training and 2,000
validation renderings, seed 42, `--balance 0.75`, from
`tool/render-synthetic-hebrew.py`. Alpha 0.75 rather than 1.0 was chosen by
simulation: it yields more word-initial kaf-with-dagesh exposure (2,714 against
2,645 clusters) *and* retains more vocabulary (8,445 against 7,711 distinct
words), because alpha 1.0 over-concentrates on words carrying ultra-rare
clusters that are not kaf. Zero duplicate images, disjoint train/validation
vocabularies, all 41 scalars covered.

Real data: 344 reviewed headwords, all outstanding review completed 2026-09-12.
274 training-partition, 46 validation-partition, 24 development. Exported by
`gesenius export-headword-training` from a bounded review root, and by
`tool/export-development-headwords.py` for the partition that command
deliberately skips. The frozen 30 remains a byte-identical subset of the
validation partition, so continuity with earlier experiments holds.

Pretraining selected checkpoints on the 46 real validation pairs rather than the
synthetic score, which is the flaw the previous report identified in its own
method. Real transfer rose to its peak by epoch 4 and then oscillated for ten
epochs without improving; early stopping ended the run at epoch 15 after about
13 hours. The 30-epoch cap was never binding.

Fine-tuning used `--resize union` so the codec admits the space scalar, which
the single-word synthetic corpus never renders and which occurs 6 times across
5 multi-word headwords.

Commands, digests and results are in
`artifacts/synthetic-pretrain-2026-09-12-50k-balanced-seed42/`.

## Never report Kraken's internal validation score

Kraken's own score overstated independent measurement in every case:

| Model | Internal score | Measured |
| --- | --- | --- |
| Synthetic only, 50k balanced | 68.75% | 67.76% on validation-46 |
| Fine-tune from 50k balanced | 89.47% | 88.82% on validation-46 |
| Fine-tune from 10k uniform | 87.50% | 87.17% on validation-46 |

Under the consistent method the gap is small, about one point, rather than the
twelve to seventeen points `ketos test` suggested. The internal score is still
a selection signal computed on the selection set, and still excludes characters
outside the trained alphabet, so it should not be reported as a result — but the
alarming discrepancy reported earlier was an artifact of the measurement method,
not of the score.

## Next

- **Review is the highest-yield activity available.** Doubling the reviewed
  fitting set returned +4.9 points; five times the synthetic data returned +0.6.
  The next experiment worth running is more reviewed pairs, not a better
  renderer.
- **Reserve pages for evaluation before the next fine-tune.** Fitting on the
  274 training-partition pairs leaves 24 clean pairs, and the model memorises
  its fitting set completely. Without reserved pages the next comparison cannot
  be measured.
- **Use the recognizer to seed review.** At 89.63% character accuracy and 15 of
  24 exact, its output is a far better review starting point than the Tesseract
  suggestion at 30.49% and 1 of 24.
- **Acceptance is closer but not reached.** 98% exact pointed implies about
  99.7% character accuracy. This run reaches 62.5% exact on clean data, from a
  baseline of 4%.
- Pages 250 and 775 remain untouched and are still the only source of a clean
  final estimate.
