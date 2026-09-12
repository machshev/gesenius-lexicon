# Controlled Kraken headword experiments, 2026-09-12

This note records negative and positive results from the expanded reviewed set.
It does not authorize a model for OCR routing.

## Matched from-scratch comparison

Both 4.0-million-parameter VGSL runs used the same 30 reviewed validation
headwords, seed 42, deterministic PyTorch operations, default architecture, and
validation-based stopping. Independent `ketos test` evaluation produced:

| Reviewed fitting headwords | Selected checkpoint | Character accuracy | Errors (I/D/S) | Exact words |
| ---: | --- | ---: | --- | ---: |
| 47 | `best_0.1534.safetensors` | 15.34% | 1 / 70 / 89 | 0 / 30 |
| 121 | `best_0.1481.safetensors` | 14.81% | 0 / 72 / 89 | 0 / 30 |

The larger fitting set made one additional character error. The earlier 19.05%
run was not seed-controlled and is therefore not evidence that either current
dataset regressed. The matched result does show that adding these reviewed
headwords does not rescue a small recognizer trained from scratch.

## PP-OCRv6 small baseline

The official PP-OCRv6 multilingual small weights, evaluated without fitting on
the same 30 headwords, scored 51.32% character accuracy: 92 errors (0 insertion,
81 deletion, 11 substitution) and 0 exact words. The upstream model record says
the recognizer covers Hebrew and reports 6.50% Hebrew in-domain CER, but its
training domain and line geometry differ from these lexicon headword crops. See
the [official model record](https://zenodo.org/records/21788405).

The downloaded weights were verified before use:

- file: `ppocrv6-small.safetensors`
- size: 13,065,380 bytes
- MD5: `07279cd68cd4402807542799f12d0f4a`
- SHA-256: `3108259779f5abfed3908a1c9eafba64d17e4a2a169368c539ecfeff04c018f3`

Model binaries and experiment artifacts remain ignored rather than being
committed to Git.

## Fine-tuning boundary on this laptop

Two bounded CPU fine-tuning attempts were killed by the operating system before
an epoch completed and produced no checkpoint. Codec-union fitting instantiated
14.1 million trainable parameters. A second one-epoch attempt used a new codec
and NFD normalization but still instantiated 12.7 million parameters because
Kraken's auxiliary NRTR decoder remained active. Batch size was already one.

The laptop exposes an Intel Meteor Lake NPU through `intel_vpu`; it is not a TPU.
The pinned PyTorch build is CPU-only and Kraken's training device interface does
not expose this NPU. It therefore cannot remove the memory boundary for this
run.

## Next experiment

Keep the reviewed headword validation set as the target benchmark, but enlarge
fitting coverage with reviewed Hebrew word crops from anywhere in the same
Robinson typeface. Record those as `hebrew-word`, separate from `headword`, and
exclude detected headword lines from the general word queue. This supplies more
letter and point examples without pretending body words are headwords or
leaking validation pages into fitting.

After the new queue is reviewed, export both word classes into the page-level
training/validation split. Continue to report target accuracy against the fixed
headword validation subset. Fine-tuning the PP-OCRv6 baseline will require a
machine with more memory/a supported accelerator, or an upstream-supported way
to disable or freeze the auxiliary decoder.
