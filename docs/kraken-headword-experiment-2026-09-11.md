# Kraken pointed-headword experiment, 2026-09-11

## Outcome

Kraken 7.1 successfully trained a new VGSL recognizer and emitted a loadable
checkpoint. This resolves the mechanics and resource failure seen when the
34.7-million-parameter PP-OCRv6 medium model was fine-tuned, but it is not an
accuracy success: an independent test of the selected checkpoint reached 19.05%
character accuracy and 0% exact-word accuracy on 30 validation headwords.

The local, ignored experiment bundle is
`artifacts/kraken-headword-vgsl-2026-09-11-complete`. Its selected weights are
`checkpoints/best_0.2011.safetensors`, SHA-256
`c99c05064bde15547bbcf7aae489e948140cacd9c77d359ff40c44279fd77e33`.
Model weights remain an external artifact and are not committed.

## Reviewed-data audit

The review records were not lost. A local commit initially contained 18 new
journal records. The subsequent `git pull --rebase` replayed it over a remote
commit containing another 60 journal records and the associated page-700 and
page-925 crop manifests. The resulting commit, `c9fd831`, contains all 78
additions.

Comparing the complete journal with the post-rebase manifests gives 101 current
resolved headwords and one current `not_headword` decision:

| Partition | Current resolved | Current not-headword |
| --- | ---: | ---: |
| Training | 47 | 1 |
| Development | 24 | 0 |
| Validation | 30 | 0 |

There are no stale or missing headword decisions. Development pages remain
deliberately unavailable for fitting. The authoritative export therefore
contains 47 fitting headwords from all ten training pages and 30 validation
headwords from printed pages 175, 475, and 925. Its `ground-truth.jsonl`
SHA-256 is
`43d406b36521aa50d61d9957319bca5790158321a4944e28f4ad323e4a6da618`;
the frozen split SHA-256 is
`35e2216836534e5ac8865aa3a29c84fb561f5403e536197f02507a3a066cc0d1`.

## Training and evaluation

The host had 30 GiB RAM, 14 GiB reported available, and nearly exhausted swap
before the run. Kraken's default 4.0-million-parameter VGSL architecture trained
from scratch without exhausting memory. The validation-based run used the
repository's existing command:

```console
nix develop path:. --command cargo run --quiet -- train-prepared \
  --prepared artifacts/kraken-headword-vgsl-2026-09-11-complete/prepared \
  --output-model artifacts/kraken-headword-vgsl-2026-09-11-complete/checkpoints
```

Kraken stopped after epoch 33 when validation accuracy had not improved for ten
evaluations. Its internal selection score peaked at 20.11% at epoch 23, while
an independent `ketos test` invocation over the explicit validation manifest
reported 153 errors among 189 reference characters: 84 deletions, 65
substitutions, and four insertions. That is 19.05% character accuracy and 0%
exact-word accuracy.

Kraken reported 12 training-only characters absent from validation and three
validation-only letters (`ק`, `ט`, and `ץ`) absent from fitting data. The latter
cannot be learned by a model trained from scratch. This split is useful for
page-level isolation but too small and alphabetically separated for recognizer
training. Do not integrate this checkpoint into OCR routing.

After this experiment, the fitting queue was expanded with 74 source-anchored
candidates from 15 additional pages. Their unreviewed machine suggestions contain
all three missing validation letters. They are not included in the experiment
counts or checkpoint and cannot enter a new export until human source review.

## Next gate

Collect substantially more fitting headwords across the alphabet, ensuring
that every validation character occurs in fitting examples without moving the
held-out pages into training. Then rerun a learning curve with a
printed-Hebrew-compatible base model or the resource-safe VGSL model, selecting
by held-out exact-word and combining-mark metrics rather than training loss.
