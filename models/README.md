# OCR models

Do not commit model weights here. `*.mlmodel`, `*.onnx`, and `*.safetensors` are
ignored.

The default pipeline uses the Apache-2.0 PP-OCRv6 medium model published as
`10.5281/zenodo.21788410`. Run `cargo run -- setup` from the repository root
to check the OCR tools, download the model, and verify that its SHA-256 matches
`pipeline.toml`. The equivalent manual flow is `kraken get
10.5281/zenodo.21788410`, followed by copying `medium.safetensors` from the
printed model directory to `models/ppocrv6-medium.safetensors`.

For a release, publish each model separately with:

- a SHA-256 checksum;
- the completed `model-card.template.toml`;
- base model identity and licence;
- training corpus/source rights;
- exact `uv.lock`, pipeline commit, command, and page split;
- overall and per-script CER/WER;
- CPU/GPU and known-limitations notes.
