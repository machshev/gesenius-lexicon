# Batch 001 spot-check (pdf 19-70)

Date: 2026-10-04. Full notes, per-line verdicts (JSONL) and crops are in the session scratch dir
(`batch-001-check/`). pdf 266 and 791 were not viewed.

## Transcription (40 lines, 36 pages)

The sample file has 30 lines (14 he, 8 ar, 8 syr), and 29 of them are `adjudicated`, so it is
biased toward hard lines. I used all 14 Hebrew, 4 Arabic and 4 Syriac lines, and added 11 Hebrew,
3 Greek and 4 Latin lines at random from pass 2. Each crop was read at 4x. Codex read the 6 suspect
lines independently (crop only), and 2-of-3 was applied.

| script | lines | confirmed errors |
|---|---|---|
| Hebrew | 25 | 2 (8%) |
| Arabic | 4 | 0 |
| Syriac | 4 | 0 (1 unresolved) |
| Greek | 3 | 0 |
| Latin | 4 | 0 |

- pdf 55: the print has plene `חַלּוֹנוֹת`; pass 2 wrote the Masoretic `חַלֹּנוֹת` (dictionary-form spelling).
- pdf 35: the print has `הַגְּיאָוֹת` with sheva; pass 2 wrote the Masoretic tsere `הַגֵּ` (dictionary-form pointing).
- Not confirmed (codex sided with pass 2): a holam on `אֹהֶל` (pdf 34) and a mark over `فلان`
  (pdf 64). Unresolved: a zqapha in `ܝܗܘܕܳܝܐ` (pdf 61). Needs a human look: `אֲיֵמָה` against
  `אֲיֵבָה` (pdf 56).

All errors are dictionary-form substitutions; there are no consonant errors, dropped text or
inserted text. Hebrew line error rate 8%, against the reference of about 9% (4-14%). CER is about
0.002 (3 edits in 1,563 characters), against 0.0051. Both are in range.

## Segmentation

Entries over 4,000 characters:
- Genuine: p24:e0007 אוֹפִיר, p43:e0001 I. אַיִן, p44:e0007 אִישׁ, p49:e0007 I. אֵל, and
  p50:e0002 III. אֵל (scans of pdf 67-68 show only this entry). p250:e0001 is a held-out page; from
  its text it is a page-top continuation of הָיָה, which is expected.
- Failure, merge: p47:e0009 is `אָכִישׁ` (3 lines) plus the missed headword `* אָכַל` (scan pdf 63).
- Failure, dropped lines: p2:e0001 is `אָב` with its first 6 lines (pdf 17 c1-r02-l12..17) present
  in ALTO but missing from the corpus. The pdf 18 continuation is left without a headword.

Headless (23; only p2:e0001 falls in the batch range): the letter introduction p1:e0001 and the
letter headings p105:e0004 and p169:e0003 are expected (da5733e). The page-top continuations
p169:e0001 and p250:e0001 are expected. p2:e0001 is a failure. p200:e0003 is a one-character `]`
from tesseract on a legacy page.

Further failures found:
- pdf 33: column detection failed. There are 4 full-width chunks, 59 unverified
  `draft_only`/`reread_only` lines, duplicated and interleaved lines, and the entries p16:e0016 to
  p17:e0005 are scrambled. Run-batch reported 0 failures and validate passed.
- Missed headwords are systematic: about 22 in about 12 entries, or about 5% of the 403 new entries.
  Examples: p18:e0007 holds 5 entries, p34:e0007 absorbs 5, p38:e0009 holds 4, and p42:e0012
  absorbs `* אִין`. Likely causes: flat line x coordinates (60 of 418 regions, no indent signal) and
  a `*` hanging in the margin.
- The content line pdf 59 c0-r00-l01 `3. Emim, ...` is dropped from the corpus.

## Verdict

Go for transcription, after one fix. First add a layout guard to `tool/run-batch.py`: fail or rerun
a page when a column chunk is wider than about 1,200 px or when it has more than a few unverified
lines. Then rerun pdf 33. Fix the importer in parallel and re-import from ALTO, which needs no new
transcription: headword detection without relying on indentation, the dropped first column line,
and the `אָב` start after the letter introduction. Also add an ALTO-to-corpus line-parity check and
a headword-pattern scan to the batch report.
