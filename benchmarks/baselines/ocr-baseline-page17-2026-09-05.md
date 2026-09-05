# Robinson 1854 page-17 cached OCR baseline

Date: 2026-09-05  
Scope: PDF page 17 (printed page 1), gold entry `robinson-1854:p1:e0001`  
Run: `360d1913632707d21584c6affccc1934848a4ded6cc583535be2ea5b1aecdc15`

This report evaluates already materialized ALTO outputs. It does not rerun OCR. The evaluator was the clean source-built `target/debug/gesenius` from commit `9eb6ad381742a4e464fcedb72217e1f168e4622d` (SHA-256 `9bfeef72c5988961b4cc09fe29902fde95a4a13aa4a6ca01703bac762e04ba99`). Gold is `benchmarks/gold/robinson-1854-p001-e0001.json` (SHA-256 `7600120a3fe44ca807ca1bd91c6e7c9ef6483e7b9698f115241f6a458451c8ea`), with 3,148 reference characters and 564 words.

| Cached stage | CER | WER | Missing gold lines |
| --- | ---: | ---: | ---: |
| `pdf-text-layer` | 0.9749047 | 1.0000000 | 79 |
| `tesseract-primary` | 0.1756671 | 0.2500000 | 0 |
| `tesseract-multilingual` | 0.1724905 | 0.2907801 | 0 |
| `tesseract-block-refined` | 0.1677255 | 0.2251773 | 0 |
| `tesseract-word-recognized` | 0.1311944 | 0.2180851 | 0 |
| `tesseract-fused` | 0.1470775 | 0.2216312 | 0 |
| `kraken` | 0.9749047 | 1.0000000 | 79 |
| `kraken-refined` | 0.1299238 | 0.2109929 | 0 |

The previously documented fused baseline is reproduced exactly: CER `0.14707750952986023`, WER `0.22163120567375885`, no missing lines. The strongest cached stage among line-complete outputs on these metrics is `kraken-refined`; this is a stage comparison, not evidence that Kraken replaced the canonical fusion path or that the result generalizes beyond this page. The `pdf-text-layer` and raw `kraken` rows have 79 missing gold line IDs, so their high error scores are line-ID alignment failures and are not comparable OCR accuracy measurements. The script CER values remain diagnostic because the baseline evaluator at `9eb6ad3` loses spatial correspondence and omits inherited marks from script totals.

The cached page records source SHA-256 `466b061e770f212cb7d888d8dadc2a54575fb115bf6de9cdb24b0c280461ccaa`, scan ID `loc-gdcmassbookdig.hebrewenglishlex00gese`, recorded Tesseract model identity strings `8755b804...e1565` (English) and `d80008bc...25bf` (multilingual), and a recorded Kraken model-file SHA-256 `15313b51...81ac9`. The current engine computes Tesseract identities by hashing sorted language names and the SHA-256 digests of their traineddata files; these cached values were not independently recomputed against the original model bytes here. Current configuration hashes are pipeline.toml `ad64c396...ed79d`, sources.toml `3148a87e...883b`, flake.lock `da6dd33d...3255`, and Cargo.lock `b16f20ac...599b8`.

The cached receipts retain per-stage commands, input hashes, output paths, and completion times under `.cache/gesenius/runs/<run>/robinson-1854/page-0017/`. They do not identify the source commit that produced the run, a standalone complete pipeline/tree manifest, the exact Nix closure, or the Tesseract traineddata bytes. These are explicit provenance gaps for future runs to close.

Rebuild the evaluator at the recorded source commit, then reproduce each row with:

```bash
git checkout 9eb6ad381742a4e464fcedb72217e1f168e4622d
nix develop path:. --command cargo build --locked
target/debug/gesenius benchmark \
  --gold benchmarks/gold/robinson-1854-p001-e0001.json \
  --alto .cache/gesenius/runs/360d1913632707d21584c6affccc1934848a4ded6cc583535be2ea5b1aecdc15/robinson-1854/page-0017/<stage>.alto.xml
```
