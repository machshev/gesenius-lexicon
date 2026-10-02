# Prompt experiments, 2026-10-02

Two alternative pass 2 prompts were run against the ten gold pages and audited line by line against the scans.
Prompt v3 is the "v-next" experiment; prompt v4 is "no added or dropped marks, no script change, single-line
running head, keep word order". Both are opt-in (`tool/frontier-verify.py --prompt-version`); the default is v2.

CER and WER against gold (v2 / v3 / v4), measured before the gold corrections of this date:

| measure | v2 | v3 | v4 |
|---|---|---|---|
| overall CER / WER | 0.0010 / 0.0038 | 0.0073 / 0.0122 | 0.0012 / 0.0065 |
| Hebrew CER | 0.0083 | 0.0462 | 0.0071 |
| Arabic CER | 0 | 0.0442 | 0.0177 |
| Syriac CER | 0 | 0.0328 | 0.0164 |

Scan audit of the lines where the prompts disagree:

- v3 vs v2 (90 lines): old (v2) right 46, new (v3) right 23, both wrong 11, ambiguous 10.
- v4 vs v2 (78 lines): v2 right 23, v4 right 41, both wrong 7, ambiguous 7. Most of v4's lead is running heads. On
  body Hebrew, v4 fixes about 4 and breaks about 4 per 100 lines, so it is a wash there, and v4 adds unprinted
  Arabic marks.

Decision: the full book uses prompt v2, the line-pairing fix of d821b72, and a deterministic running-head step
(`unpoint_running_head` in `tool/frontier-to-alto.py`: Hebrew without vowel points, dagesh, meteg or accents,
keeping the shin and sin dots). That recovers v4's only clear win without v4's regressions. v3 and v4 stay
opt-in. Full audit tables are in the appendices below.

## Appendix A: v3 vs v2 audit

# Prompt v3 vs v2 audit (pass 2, 90 differing lines, 10 pages; 266/791 not opened)

Each verdict was judged from the 2054x3488 raster, with crops zoomed 5-14x. Gold was not used to decide. Per-row detail is in audit.jsonl.

## Verdicts
| | old-right | new-right | both-wrong | ambiguous |
|---|---|---|---|---|
| All 90 | 46 | 23 | 11 | 10 |
| Hebrew lines (72) | 37 | 19 | 10 | 6 |

## v3 regressions (46 old-right rows)
- Pointing: 21. These are wrong vowels, plus dropped marks: Syriac vowels, Arabic tanwin, a meteg, holem-vav. Row 30 is the reverse: v3 added pointing to unpointed אילן נעבד.
- Inserted text: 13. Twelve are header items repeated as extra lines on pdf 116, 491, 641, 716 and 1091, and one is a duplicate "comp." line on pdf 66. v2 has the same habit on pdf 66, 341, 716 and 941, so this is probably a pass-2 merge artifact more than the prompt. The extra v3 header items also drop shin dots.
- Consonant or script: 6. Examples: צָפַן for גָּנַן, בְּצַלְמִים, an Arabic word turned into Syriac, an inserted Syriac taw, "i. c.".
- Reordering: 3, all in the pdf 66 c1-r02 cluster. Spacing 2, punctuation 1.

## pdf 66 drop and reorder
- No line was really dropped. v3 moved the "comp. אֶלְיָקִים…" line above ע״ו and also emitted a second, reversed copy of it, so rows 19-22 are a reorder plus a duplicate. The ע״ו line straddles the boundary between bands b00 and b01, so a merge bug in pass 2 is the likely cause.
- The other "reordering", row 11 (אֵל עֶלְיוֹן …), is correct in v3. The phrases are printed left to right, and v3 keeps the printed period. v2 reversed the list and changed the period to a comma.

## Does v3 fix printed-vs-expected?
Yes, in 11 lines:
- 8 body lines where v2 wrote the dictionary form: rows 24, 25, 43, 71, 74, 77, 81, 89.
- 3 running heads where v2 added pointing: rows 26, 67, 82.

v3 also has 12 more new-right rows, for example a dagesh v2 inserted (row 42), a vav v2 inserted (row 64), Syriac ܫܠܶܡ (row 87) and the duplicate headers on pdf 66. But v3 also writes expected forms itself (row 80), and in rows 31, 66 and 75 both versions missed the printed form.

## Gold lines that look wrong
- pdf0716-c2-019: print is שָׁמַם, gold has שָׁמֵם.
- pdf0191-c2-023: print has a typographic apostrophe (Jeb’a), gold has ASCII.
- pdf0191-c2-029: every chapter,verse pair is printed tight, gold has spaces. This is a spacing-convention question.
- pdf0017-c1-004 and pdf0066-c1-035 are unclear (comma or period).

## Recommendation
Reword; do not adopt v3 as it is. Keep the "as printed, not the expected form" sentence and the running-head sentence. Add:
1. Do not add any point that is not printed, and do not drop any point, tanwin, seyame or meteg that is.
2. Never change a word's script or language.
3. Write the running head once, as one line, keeping shin and sin dots.

Separately, fix the pass-2 band-overlap merge, which produced the pdf 66 reorder and duplicate.

## Appendix B: v4 vs v2 audit

# Prompt v4 vs v2, visual audit of the 78 diff rows

Judged from the 400 dpi raster crops (no pdf 266/791). Readings from the v2-vs-v3 audit were reused where they were clear and still applied. Rows are in audit.jsonl.

## Verdicts
| | old-right (v2) | new-right (v4) | both-wrong | ambiguous |
|---|---|---|---|---|
| all 78 | 23 | 41 | 7 | 7 |
| rows with Hebrew | 20 | 28 | 6 | 3 |
| Arabic tokens (rows 12, 22, 34, 44, 72) | 4 | 0 | 1 | 0 |
| Syriac tokens (rows 1, 2, 13, 76) | 2 | 2 | 0 | 0 |

Of v4's 41 wins:
- 12 drop duplicate running-head items that v2 invented (rows 5-7, 25-27, 31-32, 62, 65-67).
- 4 write the running head unpointed.
- 15 are Hebrew printed-vs-expected fixes (9, 18, 19, 21, 37, 38, 51, 57, 58, 64, 70, 74, 75, 77 and the period in row 10).
- 2 are Syriac (13, 76).
- The rest are Latin and Greek conventions: ϑ, ’, small caps, and the "soutnwards" and "guilar" print defects.

## v4 regressions (23 old-right)
- Pointing (19).
  - v4 inserts marks that are not printed: holem in ע״ו and ל״ה, a meteg in נָשַׁל, a dagesh in כֵן, a furtive patah in מִשְׁטֹח, a rukkakha in Syriac, and Arabic fathas, kasra/sukun or tanwin in قرض, الخروع and جَبَّانَة.
  - v4 also changes vowels: יֶשׁ for יֵשׁ, אֲשִׁישׁ, גְּבִיעַ, מִגְבְּלוֹת, כִּנָּת, קֵינִי, the defective חַרְבֹנֵי, אֶגְפִּים, and segol and qamets swaps.
  - 9 of these are identical to v3's errors.
- Consonant (2).
  - Row 53: צָפַן is written for the printed גָּנַן, a lexical substitution.
  - Row 12: a superscript alef is written where a lam-alif ligature with a full alif is printed.
- Punctuation (1): row 52, "6. 2." becomes "6, 2.".
- Case (1): row 54, PIEL against the title-case convention.

## Arabic and Syriac
v4 is genuinely wrong on the gold-bearing Arabic and Syriac lines (pdf0941-c2-037 and pdf0017-c1-030): it added marks that are not printed, and gold is right on both. On the excluded lines, v4 is worse on Arabic (it adds vowels) and splits even on Syriac.

## Gold lines that look wrong
- pdf0716-c1-029: the meteg is printed, נָֽשֶׁךְ.
- pdf0191-c2-023: Jeb’a should have the typographic apostrophe.
- pdf0191-c2-029: the print sets the references tight, "2,25." with no space.
- pdf0491-c2-033: the print literally reads "guilar"; this applies only under the print-defect convention.

## Net
There are about 360 Hebrew-bearing lines across the 10 pages.
- Content: v4 fixes 15 and breaks about 15 (≈ ±4.2 per 100 lines each), so the net is about zero.
- Running heads and duplicates: v4 fixes 12 more lines.
- Net: about 3 fewer erroneous lines per 100 Hebrew lines, almost all from header handling.
