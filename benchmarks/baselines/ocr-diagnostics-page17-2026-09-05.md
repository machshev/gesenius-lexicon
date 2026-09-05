# Cached page-17 scoring diagnostics

The scoring implementation at `41191bc` was checked against the same cached fused
ALTO and legacy page-1 gold used in the earlier baseline. Recomputed SHA-256 hashes
match both earlier input identities. [The JSON report](ocr-diagnostics-page17-2026-09-05.json)
records the exact command, implementation/binary/input hashes and full results.
No OCR stage ran, and this does not measure a new recognizer or whole-book accuracy.

| Measure | Result |
|---|---:|
| Exact CER | 0.14707750952986023 (unchanged) |
| Exact WER | 0.22163120567375885 (unchanged) |
| NFC CER / WER | Same as exact in this sample |
| Missing gold line IDs | 0 |
| Foreign-containing reference tokens | 93 |
| Foreign-containing hypothesis tokens | 47 |
| Exact aligned foreign-token matches | 7 |
| Exact foreign-token accuracy | 7 / 93 = 7.53% |
| Exact foreign-token precision | 7 / 47 = 14.89% |

These strict tokens include attached punctuation and every point/accent; the
metric is not linguistic word segmentation. The source comparison still uses
legacy engine line IDs and reports `source_identity: unverified`. Canonical
scores remain separate, and the old historical-Aleph mapping in the fixture
prevents any claim about the distinct drawn shapes. The aligned script matrix
now exposes cross-script substitutions, including Hebrew→Latin and Syriac→Latin;
it does not turn this legacy fixture into representative gold.

The representative source-reviewed sample is still being assembled. The initial
no-regression tolerances and exact/canonical/foreign-token conventions are in
[the metric policy](../../docs/ocr-metric-policy.md). Segmentation, comparable
stage evaluation and candidate-selection oracle diagnostics remain open.
