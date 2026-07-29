# OCR models

Do not commit model weights here. `*.mlmodel` and `*.onnx` are ignored.

For a release, publish each model separately with:

- a SHA-256 checksum;
- the completed `model-card.template.toml`;
- base model identity and licence;
- training corpus/source rights;
- exact `uv.lock`, pipeline commit, command, and page split;
- overall and per-script CER/WER;
- CPU/GPU and known-limitations notes.
