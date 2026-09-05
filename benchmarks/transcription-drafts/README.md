# Source transcription drafts

These samples are **not accepted gold**. Keep them outside `benchmarks/gold/`
until a second reviewer has checked the source crops and resolved every listed
uncertainty. Do not report their scores as recognition accuracy or count them
as independently verified lines.

## Printed page 50, right-column opening

`robinson-1854-p050/draft.json` uses the coordinate benchmark schema so reviewed
lines can later be promoted without inventing engine line IDs. `review.json`
records crop hashes, page partition, commands, scripts and unresolved readings.
The twelve committed PNGs are direct source crops, without resizing, thresholding
or contrast changes. Transcription used the page context, a native 360 dpi region,
and an enlarged inspection view; no OCR hypothesis or dictionary was consulted.
The source PDF and prior 180 dpi raster hashes were recomputed and matched the
sample inventory. The new 360 dpi raster is 1855 × 3139 pixels: **do not simply
double the rounded 180 dpi page dimensions** or assert this coordinate frame for
an independently deskewed pipeline image.

The sample is twelve consecutive complete printed lines, containing Latin prose,
five lines with pointed Hebrew, and one line with polytonic Greek. The words
“Heb.” and “Syriac” on line 11 are Latin-script prose, not Hebrew/Syriac glyphs.
This sample has no Arabic, Syriac, Ethiopic or historical-glyph coverage.

### Draft transcription conventions

- Preserve spelling, punctuation, printed abbreviations and line boundaries.
  In particular, do not insert a yod in the apparent defective Hebrew plurals.
- Store Hebrew runs in logical reading order within left-to-right prose; do not
  add directional control characters to the scored text.
- Use one ordinary space between typographic words. Font size, italics and small
  caps remain visible in the crop; `Plur.` represents the printed small-cap label.
- Represent base letters and observed points with Unicode. No dictionary-based
  repair, accent removal, compatibility normalization or historical-glyph
  substitution has been applied. A final Unicode/equivalence scoring policy is
  still open in the accuracy plan.
- The proposed text in an unresolved line is a review candidate, not an assertion
  that the glyph is settled. Consult `review.json` before promotion.

### Review and promotion

A second reviewer should transcribe from the crops first, then compare the draft,
inspect the full source page where necessary, and record reviewer identity,
date, disagreements and resolutions. Verify every line, including those without
an explicit uncertainty. Preserve the draft/review history. Promote only resolved
lines to gold, recording the accepted authority and retaining the exact source
anchors. Keep any unresolved lines here and report their exclusion. No second
review has occurred in this session; accepted gold count remains zero.

Next sample: the development page 700 Ethiopic/Syriac comparison region specified
in the sample inventory, followed by validation page 175. Preserve the frozen
final-test pages. Numeric acceptance tolerances must be recorded before tuning.
