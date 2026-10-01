# Plan: regenerate the gold fixtures, 2026-10-01

Status: plan only. No fixture, record or tool has been changed. This is step 4
of `docs/frontier-transcription-2026-09-23.md` ("Next").

## 1. What exists

### Format and consumption

- Three fixtures in `benchmarks/gold/`, loaded by `GoldBenchmark::load`
  (`crates/gesenius-core/src/benchmark.rs`): `id`, `edition`, `source_page`
  (PDF page), `source_sha256` (of the PDF), `authority` (free text), optional
  `source_image` (`width`, `height`, `coordinate_frame`), and `lines`, each
  with `line_id`, `text` and optional `source` (`source_page` plus pixel
  `bounds`). Either every line is anchored or none is.
- Pages covered: p001-e0001 is PDF 17, the whole Aleph entry, 79 lines, with
  no anchors (legacy line-ID alignment). p050-right-top is PDF 66, 12 lines,
  anchored. p700-left-comparisons is PDF 716, 9 lines, anchored. The anchored
  ones use a 360 dpi poppler frame (1855x3139); the frontier rasters are 400
  dpi (about 2061x3488), so new gold needs its own frame.
- Consumers: `gesenius benchmark` and `benchmark-stages` (policy in
  `docs/ocr-metric-policy.md`: exact CER/WER, NFC score, base/mark
  diagnostics, per-script aligned counts, exact foreign-token accuracy,
  coordinate alignment by positive box overlap, zero-increase acceptance
  gates); `tool/score-frontier-transcription.py` (NFC, collapse spaces, drop
  the space before closing punctuation, match each gold line to its most
  similar page line); the Rust test
  `checked_in_gold_fixtures_are_structurally_valid`, which loads every
  `benchmarks/gold/*.json`; and the committed page-17 baselines in
  `benchmarks/baselines/`, which cite the p001 file by SHA-256.
- Provenance: p001 is "frontier transcription checked visually". p050 and p700
  are assistant drafts, source-checked by James. Drafts and reviews are in
  `benchmarks/transcription-drafts/`.
- Split policy (`benchmarks/sample-inventory/robinson-1854-splits.toml`, printed
  page = PDF - 16): development 1, 50, 100, 325, 700 (PDF 17, 66, 116, 341,
  716). Validation includes 175, 475, 625, 925, 1075 (PDF 191, 491, 641, 941,
  1091). Final test is 250 and 775 (PDF 266, 791), to be inspected once.

### Where the fixtures are wrong

Diffing the fixtures against the pass 2 lines (matched by similarity, NFC
form as in the scorer):

- p001: 18 of 79 lines differ; 3 are trivia (comma vs full stop after
  "Plutarch", a curly apostrophe, printed ϑ vs θ). The other 15 are in
  foreign scripts. Examples: line 30 הִקְטִיל vs הִתְקַטֵּל; line 31 Arabic
  أَقْتَلَ vs Syriac ܐܶܬܩܰܛܰܠ (pass 2 note says Serto Syriac); line 39 Syriac
  ܐܒܪ vs ܗܰܒܳܒܳܐ; line 50 Arabic vs Syriac; line 29 ה vs ח; line 65 and 66 order
  of Hebrew items swapped; line 51 בְּאֵר vs בְּאֹר.
- p050: 0 of 12 differ. The gold is identical to pass 2, which fixed the
  pass 1 אֵלִים error. This is the best case.
- p700: 1 of 9 differs, and only because pass 2 never ran on PDF 716 (below);
  the gold has Syriac points, pass 1 has none.
- Some p001 disagreements are not proof that pass 2 is right. Lines 41 and
  54 (אוֹר/עוּר, אֲרִישׁ/אֲרוּשׁ in gold; אוּד/עוּד, אָדַשׁ/דּוּשׁ in pass 2)
  were marked "agreed" by pass 1 and pass 2, so the two model passes can
  share an error. On line 66 the pass 2 adjudicator's own note says each
  reading was half right. Agreement between passes is not verification.
- The p001 fixture has no anchors, and the metric policy says its Phoenician
  Aleph mapping is a legacy interpretation.

### Pass 2 coverage problem

The handoff says pass 2 is done for all 24 pages. It is not. Nine records have
`errors` of 26 to 32 and cost 0: PDF 566, 716, 791, 866, 941, 1016, 1091,
1156, 1171. Every band reread failed (`claude exited 1` with zero tokens, most
likely a quota or usage stop), so every line is `draft_only` and
`needs_review`. These records are indistinguishable from pass 1 text. Of the
gold pages, 716, 941 and 1091 are affected. Fifteen pages are valid: 5, 9, 15,
17, 18, 31, 66, 116, 191, 266, 341, 416, 491, 641, 1176. Across them, 122
lines were adjudicated, 5 unresolved (all on PDF 641), 3 one-sided. `verify`
skips only error-free bands, so a rerun pays for failed bands only.

## 2. Plan

### Pages

Phase A, development pages, whole page each: PDF 17, 66, 116, 341, 716. About
525 lines, of which roughly 230 contain Hebrew, Arabic, Syriac or Ethiopic.
Phase B, validation pages: PDF 191, 491, 641, 941, 1091, about 530 lines. Do B
after A has shown the process works. These ten pages cover the scripts the
policy requires (page 716 has Syriac and Ethiopic; 17 has Syriac and
historical Aleph).

Not included: PDF 266 and 791 (final test; reviewing them would retire the
final test, so leave them unread by any reviewer); training pages (not
needed as gold); front matter and index pages 5, 9, 15, 18, 1156, 1171, 1176
(no partition, and the index is a separate layout problem).

Prerequisite: rerun pass 2 for PDF 716, 941, 1091 (about 90 bands, about
$8 at list price, about 10 minutes with six workers). 566, 791, 866, 1016,
1156 and 1171 are not needed for gold; rerun them only before the full book.

### Surfacing disagreements (`tool/gold-queue.py`)

Per page, build a queue from pass 1 (`corpus/frontier/robinson-1854/pdf-N.json`),
pass 2 and the line geometry that `frontier-to-alto.py` already measures. Each
item is one pass 2 band (2 to 4 lines, with its enlarged image from
`.cache/.../pass2/`) holding, per line: pass 1 text, pass 2 reread, final
pass 2 text, status, adjudicator verdict and note, and a token-level diff
classified as one of: punctuation or whitespace, vav/yod only, points only,
order only, script change, case, letter change, other. Lines get a tier:

- Tier 1: status other than `agreed`, any script change, any vav/yod-only or
  order-only difference, any Arabic, Syriac or Ethiopic text, any Syriac
  point difference.
- Tier 2: `agreed` lines with pointed Hebrew.
- Tier 3: `agreed` Latin and Greek lines only.

Because the two passes are the same model reading the same page, "agreed"
is weak evidence. So the reviewer does not review only the queue (below).

### Reviewer (Opus-class, `claude -p`, 400 DPI rasters)

Three steps per band, run per page, resumable by image digest like pass 1 and
2:

1. Blind read. The reviewer sees the band image only (2x for Latin and Greek
   bands, 3x for bands with Hebrew, Arabic or Syriac) and the pass 2 prompt
   conventions, not the earlier readings. This covers every band on the
   gold pages, so the independent evidence does not depend on pass 2's
   status flags. Different model family tier from the transcriber
   (`claude-fable-5-1`), so errors are less correlated.
2. Reconcile. Wherever the blind read differs from the pass 2 final text
   after normalisation, the reviewer is shown both, plus pass 1, and the
   band image, and must name the character-level evidence for each
   difference. Verdict per line: `pass2`, `blind`, `edited` (with text), or
   `contested`.
3. Triage. Lines marked `contested`, lines where the reviewer reports it
   cannot resolve a mark at 3x, and lines containing an unencoded or
   historical glyph go to James through the existing review UI
   (`benchmarks/transcription-drafts` workflow) or are excluded from gold.
   Excluded lines are listed in the fixture sidecar and never mapped to a
   convenient letter, as the policy requires.

What the reviewer records per line: final text, verdict, the pass 1, pass 2
and blind readings, difference classes, a one-sentence evidence note for each
non-trivial decision, per-token flags for Syriac points present or absent,
plene or defective spelling confirmed on the image, and list order confirmed,
and a confidence level (`certain`, `likely`, `uncertain`). `uncertain`
lines are treated as `contested`.

Agreement rate between the blind read and pass 2 gives the independent
accuracy estimate for the frontier pipeline on each script, which is more
useful than scoring pass 2 against gold derived from it.

### Output format and location

- Fixtures stay in the existing `GoldBenchmark` schema so Rust and the
  scorer need no change. New files: `benchmarks/gold/robinson-1854-pdf0017-full.json`
  and likewise for 66, 116, 341, 716 (phase A) and 191, 491, 641, 941, 1091
  (phase B). One file per page, whole page, every line anchored, in the 400
  dpi frontier raster frame:
  `coordinate_frame = "frontier-raster-400dpi-pdfNNNN-sha256-<raster sha>"`,
  `source_image` from `raster_size`. Line bounds come from the converter's
  `row_extent` geometry. Lines are in reading order with ids `pdfNNNN-cC-LLL`.
  The `authority` text names pass 1, pass 2 and reviewer models, the date,
  and the count of lines excluded.
- Review evidence in a sidecar, `benchmarks/gold-review/robinson-1854/pdf-NNNN.json`,
  with the queue, all readings, verdicts, notes and the excluded and
  contested lists. Rust does not read it; the assembler validates it.
- The three old fixtures stay in place and are not edited: baselines cite the
  p001 digest and policy calls fixtures immutable. A short note in
  `benchmarks/transcription-drafts/README.md` style (or a new
  `benchmarks/gold/README.md`) marks them superseded for foreign-script
  scoring. New baselines are recorded against new gold; old numbers are not
  comparable.

### Metric policy and test changes

- `docs/ocr-metric-policy.md`: add a "Gold conventions and provenance"
  section covering: whole-page frontier-reviewed fixtures and their
  `authority` template; the transcription conventions as stated rules
  (printed glyphs kept: ϑ stays ϑ, typographic apostrophes stay, the
  hair space before ; : ! ? is not transcribed, no bidi controls, no
  vav/yod insertion, raised dot after inseparable prefixes kept); the
  rule that lines with unresolved readings are excluded and listed;
  the separation between development and validation pages and that
  PDF 266 and 791 remain untouched; and that scores of a pipeline against
  gold derived from the same pass are an upper bound, with the blind-read
  agreement rate reported next to them. The acceptance tolerances do not
  change. The page-17 baseline is marked superseded and a new one is
  recorded once.
- Because the gold keeps ϑ and ’ but the model writes θ and ', either the
  frontier prompt is revised (prompt version 3) or the documented
  conversion is applied explicitly at import. Recommended in the decisions.
- Tests: the existing structural test already loads everything in
  `benchmarks/gold`. Add `tool/test-gold-tools.py` (queue tiers and diff
  classes, assembler rules: contested and excluded lines never reach a
  fixture, anchors inside the raster, NFC and no bidi controls, ids unique)
  and one Rust test that scores a pass 2 derived ALTO page against its own
  new fixture and expects zero error, to pin coordinate alignment in the
  400 dpi frame. Add a check in the assembler (`--check`) that
  `source_image` matches the raster digest.

### Tooling and size

- `tool/gold-queue.py`: queue builder and diff classifier, about 250 lines.
- `tool/gold-review.py`: Opus blind-read and reconcile driver, reusing the
  CLI wrapper, banding and caching from `frontier-verify.py`, about 300
  lines.
- `tool/gold-assemble.py`: decisions to fixtures plus sidecar and checks,
  about 200 lines, reusing `row_extent` from `frontier-to-alto.py`.
- `tool/test-gold-tools.py` about 150 lines, Rust test about 40 lines,
  policy text about 80 lines.
- Estimated engineering effort: one focused session for the tools, plus
  the review runs.

### Cost and time (estimates, not measured)

- Pass 2 rerun for three pages: about $8, 10 minutes.
- Reviewer blind read: about 30 bands per page, 300 bands for ten pages,
  roughly 20 to 30 seconds and perhaps $0.10 to $0.25 per band at Opus list
  price with images, so about $30 to $75 and 20 to 30 minutes at six workers.
  Reconcile calls cover perhaps 15 to 25 percent of lines, add $10 to $25.
  Phase A alone is about half of that.
- James's time: contested and excluded lines, expected to be 2 to 5 percent
  of lines (20 to 50 lines over ten pages), about 30 to 60 minutes through
  the review UI.
- Calendar: phase A in one day including the rerun; phase B the next.

## 3. Open decisions

1. Which pages: recommend phase A (five development pages) first, then
   phase B (five validation pages); leave final-test pages 266 and 791
   unreviewed.
2. Reviewer independence: recommend blind read of every band on gold pages,
   then reconcile, over reviewing only the flagged lines, because "agreed"
   lines contain shared errors (p001 lines 41 and 54).
3. Printed-glyph policy (ϑ, curly quotes): recommend gold keeps the printed
   glyphs, and move the frontier prompt to version 3 to emit them, rather
   than silent normalisation at scoring. If prompt change is not wanted
   now, apply an explicit documented conversion at import.
4. Fixture granularity: recommend one whole-page fixture per page in the
   existing schema, over excerpts, so line-level precision can be added to the
   policy later without recutting. Whole-page also makes the p001 style
   legacy line-ID fixtures unnecessary.
5. Old fixtures: recommend keep in place, unedited, marked superseded in
   docs, over moving them (the baselines and the structural test cite
   them).
6. Contested lines: recommend exclude from gold and list in the sidecar
   unless James settles them in the review UI. Do not guess.
7. Reviewer model: recommend Opus for the blind read and the reconcile
   step, with James as the final tie-break only. A cheaper model has
   previously misread vowel points (handoff notes).
8. Rerun pass 2 on the nine failed pages now or later: recommend rerun only
   716, 941 and 1091 now; rerun the rest before the full book, and fix
   the verify script to refuse to write records whose band errors exceed a
   small threshold, so a quota stop cannot produce a record that looks done.
