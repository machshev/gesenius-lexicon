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
| 116 | 101 | 0 | 3872 | 724 | 0.0005 | 0.0014 | 0.0005 |
| 341 | 97 | 0 | 3726 | 722 | 0.0003 | 0.0028 | 0.0003 |
| 716 | 90 | 0 | 3305 | 658 | 0.0006 | 0.0030 | 0.0006 |
| 191 | 91 | 0 | 3473 | 632 | 0.0032 | 0.0222 | 0.0032 |
| 491 | 98 | 0 | 3905 | 747 | 0.0005 | 0.0040 | 0.0005 |
| 641 | 97 | 0 | 3585 | 672 | 0.0008 | 0.0045 | 0.0008 |
| 941 | 90 | 0 | 3242 | 628 | 0.0006 | 0.0016 | 0.0006 |
| 1091 | 93 | 0 | 3668 | 753 | 0.0008 | 0.0027 | 0.0008 |
| **total** | 929 | 0 | 35528 | 6805 | 0.0008 | 0.0046 | |

Per script, totals over all pages (aligned character counts; CER = (sub+del+ins)/ref chars; Latn includes English and transliteration; Zyyy is digits/punctuation/space; token accuracy is exact foreign-containing-token accuracy, not defined for Latn):

| script | ref chars | sub | del | ins | CER | ref tokens | token acc |
|---|---|---|---|---|---|---|---|
| Latn | 19689 | 1 | 0 | 0 | 0.0001 |  | n/a |
| Hebr | 3508 | 11 | 4 | 3 | 0.0051 | 560 | 0.9786 |
| Grek | 286 | 0 | 0 | 0 | 0.0000 | 52 | 1.0000 |
| Arab | 113 | 0 | 0 | 0 | 0.0000 | 24 | 1.0000 |
| Syrc | 61 | 0 | 0 | 0 | 0.0000 | 11 | 1.0000 |
| Zyyy | 11871 | 1 | 0 | 9 | 0.0008 |  | n/a |

Per page per script CER (aligned; ref chars in brackets):

| pdf | Latn | Hebr | Grek | Arab | Syrc |
|---|---|---|---|---|---|
| 17 | 0.0000 [1825] | 0.0030 [337] | 0.0000 [32] | - | 0.0000 [24] |
| 66 | 0.0000 [2188] | 0.0050 [402] | 0.0000 [29] | 0.0000 [19] | 0.0000 [3] |
| 116 | 0.0000 [2501] | 0.0087 [230] | 0.0000 [18] | 0.0000 [5] | - |
| 341 | 0.0000 [2072] | 0.0000 [354] | 0.0000 [16] | 0.0000 [15] | - |
| 716 | 0.0000 [1613] | 0.0051 [394] | 0.0000 [9] | 0.0000 [20] | 0.0000 [8] |
| 191 | 0.0000 [1895] | 0.0096 [415] | 0.0000 [46] | 0.0000 [19] | 0.0000 [7] |
| 491 | 0.0005 [2156] | 0.0000 [391] | 0.0000 [73] | 0.0000 [3] | - |
| 641 | 0.0000 [1823] | 0.0050 [402] | 0.0000 [44] | 0.0000 [3] | - |
| 941 | 0.0000 [1765] | 0.0075 [267] | - | 0.0000 [18] | 0.0000 [19] |
| 1091 | 0.0000 [1851] | 0.0095 [316] | 0.0000 [19] | 0.0000 [11] | - |

## Notes

- Latin, Greek, Arabic and Syriac are essentially error-free on these kept lines (one Latin substitution: pdf 491
  `guilar`, see the change log); nearly all error is in pointed Hebrew (11 substitutions, 4 deletions, 3 insertions
  over 3508 characters, token accuracy 0.979).
- Highest Hebrew CER: pdf 116 (0.0087), 191 (0.0096) and 1091 (0.0095), all on small Hebrew samples.
- The highest page WER is pdf 191 (0.0222): the gold now keeps `2,25.` style references tight, while pass 2 writes
  `2, 25.`.

## Change log

- 2026-10-02, gold correction: `pdf0716-c2-019` now reads `the root שָׁמַם.` (patah; the scan shows patah, the
  earlier gold had tsere). Pass 2 has tsere there, so pdf 716 moves from CER 0.0012 to 0.0015 and WER 0.0030 to
  0.0046, Hebrew CER on 716 from 0.0102 to 0.0127; overall CER 0.0009 to 0.0010, WER 0.0037 to 0.0038. No other
  page changed. The pass 2 records were not touched: the alignment fix to `tool/frontier-verify.py` (running heads
  and reordered word lists no longer emitted twice) was not applied to the stored records, because rewriting
  them would invalidate the gold-review sidecars and needs paid adjudication for several pages.
- 2026-10-02, running-head step and four gold corrections. `tool/frontier-to-alto.py` now unpoints the header
  region (`unpoint_running_head`: drops U+0591 to U+05C7 except the shin and sin dots and the maqaf), and the ALTO
  records were rebuilt from the stored pass 2 records with no model call; 7 of the 24 non-held-out records changed
  (116, 566, 641, 716, 866, 1016, 1091). Gold: `pdf0716-c1-029` carries the meteg (`נָֽשֶׁךְ`); `pdf0191-c2-023` has
  `Jeb’a` (U+2019); `pdf0191-c2-029` has `2,25.`, `40,12.`, `41,15.`, `2,8.` tight (the policy has gold follow the
  print); `pdf0491-c2-033` keeps the print defect `guilar` (print-defect convention). Effect: overall CER 0.0010 to
  0.0008, WER 0.0038 to 0.0046; Hebrew CER 0.0083 to 0.0051; Hebrew token accuracy 0.970 to 0.979. WER rises
  because pass 2 spells the reference pairs with a space (pdf 191 WER 0.0079 to 0.0222) and `guitar` against the
  print `guilar` (pdf 491); these are the corrected gold exposing pass 2 errors, not regressions.
