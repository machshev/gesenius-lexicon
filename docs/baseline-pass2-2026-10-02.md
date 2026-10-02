# Pass 2 baseline against whole-page gold, 2026-10-02

Scores of the existing pass 2 frontier transcriptions (`corpus/frontier/robinson-1854/alto/pdf-*.json`,
`pass: 2`) against the reviewed whole-page gold fixtures (`benchmarks/gold/robinson-1854-pdfNNNN-full.json`)
for the ten development pages 17, 66, 116, 341, 716 (phase A) and 191, 491, 641, 941, 1091 (phase B).
No transcription was re-run. Held-out pages 266 and 791 are not scored (the script refuses them).

**Caveat: this is an upper bound, not an accuracy estimate.** The gold is partly derived from pass 2
(lines where independent readers agreed with pass 2 under the 2-of-3 rule), and contested or unreviewed
lines are excluded from gold, so the remaining lines are the easier, agreed ones. Excluded lines stay
excluded: gold line counts below are the kept lines only. Latin CER of exactly 0 reflects this selection.
Metric conventions follow `docs/ocr-metric-policy.md` (exact Unicode scalars, no normalization; the NFC
column is the canonical-equivalence diagnostic). Per-script CER here is the aligned count
(substitutions + deletions + insertions charged per the policy) over reference characters of that script.
`missing` is gold lines with no overlapping hypothesis line (0 everywhere; alignment is by source coordinates,
identity verified for every page). pdf 66 reproduces the Rust test (CER 0.0005, WER 0.0027, 95 lines).

Command (requires `cargo build`; runs `gesenius benchmark` per page, offline, free):

```
python3 tool/score-pass2-baseline.py            # all ten pages
python3 tool/score-pass2-baseline.py 66 191     # a subset
```

## Results

Per page (exact Unicode CER/WER, aligned by source coordinates):

| pdf | gold lines | missing | ref chars | ref words | CER | WER | NFC CER |
|---|---|---|---|---|---|---|---|
| 17 | 77 | 0 | 2957 | 535 | 0.0003 | 0.0019 | 0.0003 |
| 66 | 95 | 0 | 3795 | 734 | 0.0005 | 0.0027 | 0.0005 |
| 116 | 101 | 0 | 3872 | 724 | 0.0015 | 0.0041 | 0.0015 |
| 341 | 97 | 0 | 3726 | 722 | 0.0003 | 0.0028 | 0.0003 |
| 716 | 90 | 0 | 3304 | 658 | 0.0012 | 0.0030 | 0.0012 |
| 191 | 91 | 0 | 3477 | 636 | 0.0017 | 0.0079 | 0.0017 |
| 491 | 98 | 0 | 3905 | 747 | 0.0003 | 0.0027 | 0.0003 |
| 641 | 97 | 0 | 3585 | 672 | 0.0008 | 0.0045 | 0.0008 |
| 941 | 90 | 0 | 3242 | 628 | 0.0006 | 0.0016 | 0.0006 |
| 1091 | 93 | 0 | 3668 | 753 | 0.0019 | 0.0053 | 0.0019 |
| **total** | 929 | 0 | 35531 | 6809 | 0.0009 | 0.0037 | |

Per script, totals over all pages (aligned character counts; CER = (sub+del+ins)/ref chars; Latn includes English and transliteration; Zyyy is digits/punctuation/space; token accuracy is exact foreign-containing-token accuracy, not defined for Latn):

| script | ref chars | sub | del | ins | CER | ref tokens | token acc |
|---|---|---|---|---|---|---|---|
| Latn | 19689 | 0 | 0 | 0 | 0.0000 |  | n/a |
| Hebr | 3507 | 10 | 3 | 15 | 0.0080 | 560 | 0.9714 |
| Grek | 286 | 0 | 0 | 0 | 0.0000 | 52 | 1.0000 |
| Arab | 113 | 0 | 0 | 0 | 0.0000 | 24 | 1.0000 |
| Syrc | 61 | 0 | 0 | 0 | 0.0000 | 11 | 1.0000 |
| Zyyy | 11875 | 0 | 0 | 5 | 0.0004 |  | n/a |

Per page per script CER (aligned; ref chars in brackets):

| pdf | Latn | Hebr | Grek | Arab | Syrc |
|---|---|---|---|---|---|
| 17 | 0.0000 [1825] | 0.0030 [337] | 0.0000 [32] | - | 0.0000 [24] |
| 66 | 0.0000 [2188] | 0.0050 [402] | 0.0000 [29] | 0.0000 [19] | 0.0000 [3] |
| 116 | 0.0000 [2501] | 0.0261 [230] | 0.0000 [18] | 0.0000 [5] | - |
| 341 | 0.0000 [2072] | 0.0000 [354] | 0.0000 [16] | 0.0000 [15] | - |
| 716 | 0.0000 [1613] | 0.0102 [393] | 0.0000 [9] | 0.0000 [20] | 0.0000 [8] |
| 191 | 0.0000 [1895] | 0.0096 [415] | 0.0000 [46] | 0.0000 [19] | 0.0000 [7] |
| 491 | 0.0000 [2156] | 0.0000 [391] | 0.0000 [73] | 0.0000 [3] | - |
| 641 | 0.0000 [1823] | 0.0050 [402] | 0.0000 [44] | 0.0000 [3] | - |
| 941 | 0.0000 [1765] | 0.0075 [267] | - | 0.0000 [18] | 0.0000 [19] |
| 1091 | 0.0000 [1851] | 0.0222 [316] | 0.0000 [19] | 0.0000 [11] | - |

## Notes

- Latin, Greek, Arabic and Syriac are error-free on these kept lines; all error is in pointed Hebrew
  (10 substitutions, 3 deletions, 15 insertions over 3507 characters, token accuracy 0.971).
- Highest Hebrew CER: pdf 1091 (0.0222) and pdf 116 (0.0261), both on small Hebrew samples.
- Zyyy (digits, punctuation, spaces) has 5 insertions in total.
