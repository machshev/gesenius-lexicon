# Expanded Kraken pointed-headword experiment, 2026-09-11

## Outcome

Adding all newly reviewed fitting crops did not improve held-out accuracy for
the from-scratch Kraken VGSL recognizer. The independently tested checkpoint
reached 14.29% character accuracy and 0% exact-word accuracy on the unchanged
30-headword validation set. The preceding 47-sample run reached 19.05% and 0%
respectively, so this checkpoint must not enter OCR routing.

The local, ignored experiment bundle is
`artifacts/kraken-headword-vgsl-2026-09-11-expanded`. Its selected weights are
`checkpoints/best_0.1429.safetensors`, SHA-256
`384908cfb38ebbfba1f87b448f0f993dfb9e527fec243d659122ec75ca3cba00`.
Model weights remain an external artifact and are not committed.

## Reviewed-data audit

The full-root export succeeded after the expanded queue was reviewed. It
contains 121 fitting headwords, up from 47, and the same 30 validation
headwords from printed pages 175, 475, and 925. The fitting text contains 776
Unicode scalars and the validation text contains 189. Every code point present
in validation now occurs in fitting data; the previous validation-only letters
`ק`, `ט`, and `ץ` are covered.

The validation source spans, diplomatic text, and crop hashes are byte-for-byte
equivalent to the prior experiment after removing output-path metadata. The
new `ground-truth.jsonl` SHA-256 is
`498851513f0eed9dbde52556f72191852b90a063f999c825030553add9a2c54e`;
the expanded split SHA-256 is
`c504d8edc9a9050c2e7c13d79a27f26e97c316fbb16ef335a5ebb79b5bce0dab`.

## Training and evaluation

The run used the same default 4.0-million-parameter VGSL architecture and the
repository command, without a base model:

```console
nix develop path:. --command cargo run --quiet -- train-prepared \
  --prepared artifacts/kraken-headword-vgsl-2026-09-11-expanded/prepared \
  --output-model artifacts/kraken-headword-vgsl-2026-09-11-expanded/checkpoints
```

Kraken stopped after epoch 20 when validation accuracy had not improved for ten
evaluations. Its best internal score was 14.29% at epoch 10. An independent
test of the selected weights used the explicit validation manifest:

```console
nix develop path:. --command ketos test \
  -m artifacts/kraken-headword-vgsl-2026-09-11-expanded/checkpoints/best_0.1429.safetensors \
  -f path \
  -e artifacts/kraken-headword-vgsl-2026-09-11-expanded/prepared/validation-paths.txt
```

The independent result was 162 errors among 189 reference characters: 53
deletions, 107 substitutions, and two insertions. This is 14.29% character
accuracy and 0% exact-word accuracy. Relative to the preceding run, deletion
errors fell from 84 to 53, but substitution errors rose from 65 to 107 and
total errors rose from 153 to 162. Increasing the fitting set alone therefore
did not improve this architecture/configuration.

The laptop has an Intel Meteor Lake NPU, not a TPU. Kraken's current training
path does not target that NPU, so this experiment ran on CPU. Intel integrated
GPU training through PyTorch XPU would require a separate environment and
Kraken compatibility experiment; it is not evidence about model accuracy.

## Next gate

Keep the expanded checkpoint as negative experimental evidence. The next
accuracy run should change one controlled variable instead of repeating the
same from-scratch configuration: prefer a printed-Hebrew-compatible base model,
or run a reproducible learning-rate/seed comparison if no suitable base model
is available. Continue selecting only on the frozen validation set, and require
an independently measured improvement in both character and exact-word
accuracy before integration.
