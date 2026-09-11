# Kraken pointed-headword experiment, 2026-09-11

## Outcome

Kraken 7.1 successfully trained a new VGSL recognizer and emitted a loadable
checkpoint. This resolves the mechanics and resource failure seen when the
34.7-million-parameter PP-OCRv6 medium model was fine-tuned, but it is not an
accuracy success: the selected checkpoint reached 2.33% character accuracy and
0% exact-word accuracy on six validation headwords.

The local, ignored experiment bundle is
`artifacts/kraken-headword-vgsl-2026-09-11`. Its selected weights are
`model/best_0.0233.safetensors`, SHA-256
`af9299c10060edf9d750ae32b2933bab6d42c1dfb764fa9cd96394bccd7951f7`.
Model weights remain an external artifact and are not committed.

## Reviewed-data audit

The current journal does not contain 100 distinct, trainable, current headword
labels. It contains 81 latest headword decisions: 80 resolved and one marked
not a headword. Comparing those decisions with the current crop manifests gives:

| Partition | Current resolved | Stale resolved | Missing | Current not-headword |
| --- | ---: | ---: | ---: | ---: |
| Training | 38 | 9 | 0 | 1 |
| Development | 11 | 8 | 5 | 0 |
| Validation | 14 | 1 | 15 | 0 |

Development pages are deliberately unavailable for fitting. The exporter also
rejects a whole sample manifest when one of its included lines is stale or
unresolved. The clean bounded export therefore contains 34 fitting headwords
from printed pages 11, 25, 75, 125, 200, 300, 400, and 850, plus six validation
headwords from printed page 175. Its `ground-truth.jsonl` SHA-256 is
`7793003cc6953c4153cc251549a96c73a603ab9e5cea6168b17ab2e639ff0f19`;
the frozen split SHA-256 is
`35e2216836534e5ac8865aa3a29c84fb561f5403e536197f02507a3a066cc0d1`.

Pages 550 and 650 have a mixture of current and stale fitting reviews. Page 475
has a mixture of current and stale validation reviews, and page 925 remains
unreviewed. Those labels must be reviewed against their current crops before a
larger safe export. Stale decisions were not promoted or silently reused.

## Training and evaluation

The host had 30 GiB RAM, 14 GiB reported available, and exhausted swap before
the run. A one-epoch smoke test used Kraken's default 4.0-million-parameter VGSL
architecture from scratch and completed all 34 fitting samples, producing a
checkpoint. The validation-based run used the repository's existing command:

```console
nix develop path:. --command cargo run --quiet -- train-prepared \
  --prepared artifacts/kraken-headword-vgsl-2026-09-11/prepared \
  --output-model artifacts/kraken-headword-vgsl-2026-09-11/checkpoints
```

Kraken stopped after epoch 11 when validation accuracy had not improved for ten
evaluations. The best checkpoint was epoch 1 at 2.33% character accuracy. An
independent `ketos test` invocation over the explicit validation manifest
confirmed 42 errors among 43 reference characters, comprising 40 deletions and
two substitutions, with 0% exact-word accuracy. The model emitted almost no
text and missed all validation occurrences of dagesh/mappiq, hiriq, patah,
sheva, qamats, qamats qatan, tsere, and segol.

Kraken also reported 21 training-only characters absent from validation. With
only six validation words, this result is a mechanics check and a strong
insufficiency signal, not a useful estimate of generalization. Do not integrate
this checkpoint into OCR routing.

## Next gate

Re-review the stale current-crop items, review the queued validation page, and
collect substantially more fitting headwords across the alphabet. Then rerun a
learning curve with a printed-Hebrew-compatible base model or the resource-safe
VGSL model, selecting by held-out exact-word and combining-mark metrics rather
than training loss.
