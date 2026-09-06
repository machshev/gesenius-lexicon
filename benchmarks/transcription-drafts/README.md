# Source transcription drafts

These directories preserve draft and review evidence. A sample remains outside
`benchmarks/gold/` until its source review has resolved every listed uncertainty.
Printed page 50 has now been promoted as
`benchmarks/gold/robinson-1854-p050-right-top.json`; its immutable draft and
review manifest remain here as provenance.

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
  substitution has been applied. Exact and NFC-equivalence scoring are defined
  in `docs/ocr-metric-policy.md`.
- The proposed text in an unresolved line is a review candidate, not an assertion
  that the glyph is settled. Consult `review.json` before promotion.

### Review and promotion

A reviewer should compare the visible draft with the source crop, inspect the
full source page where necessary, and record reviewer identity, date, corrections
and unresolved readings. At the user's request, the form now supports review of
a prefilled draft instead of requiring a blind initial transcription. Record this
as draft-assisted source checking, not independent transcription. Verify every line, including those without
an explicit uncertainty. Preserve the draft/review history. Promote only resolved
lines to gold, recording the accepted authority and retaining the exact source
anchors. Keep any unresolved lines here and report their exclusion. No second
review occurred during draft preparation. James subsequently checked all twelve
lines against their crops and resolved them in the append-only journal. The
promoted fixture records which readings were independent and which were
draft-assisted.

Next sample: the development page 700 Ethiopic/Syriac comparison region specified
in the sample inventory, followed by validation page 175. Preserve the frozen
final-test pages. Numeric acceptance tolerances must be recorded before tuning.

### Review in the local web interface

Run `cargo run -- review serve` and choose **Transcription review** in the header.
The form shows the draft and uncertainty list immediately. Compare it with the
crop and choose **Approve & next**, or edit it and **Save corrections & next**.
Use **Needs fixes / uncertain** with a note when a reading remains unresolved.
Saved records include reviewer, timestamp, source digest, displayed draft, final
reading, notes, revision and `review_method: draft_assisted`; they append to
`corpus/review/transcription-reviews.jsonl`. No JSON editing is needed. Existing
independent readings remain preserved; a new assisted review does not invent one.

Resolved records remain review evidence until a separate promotion checks the
current source digest, accepted authority and outstanding uncertainties. The
original draft and manifest remain unchanged. The form supports one
reviewer per line/source version; subsequent edits retain that reviewer identity.

The form's **Unicode keyboard**, beside the transcription on desktop, supplies script-specific letters and pointing.
Type a base letter before its marks, or select one existing letter to add a mark.
Use the language/script selector, character groups and Unicode-name search for
rarer glyphs. The palette also types into the review notes when that field was
last focused. Combining marks remain literal Unicode; no dictionary spelling or
normalization is applied. Keyboard editing regressions can be run with
`node --test tool/test-transcription-keyboard.cjs`.

### Mixed-direction language runs

The editor now separates a line into ordered editable runs, each with a language
and explicit LTR/RTL direction. The combined preview isolates those runs inside
an LTR line. Automatic initial splits use visible script only: foreign material
is labelled Unspecified until the reviewer chooses its language. Select text or
place a cursor, choose New run language, and use Create run at cursor / selection.
Join next run retains the first run's language/direction and supports full phrases.
The keyboard follows the focused run. Spaces and punctuation remain literal text;
no normalization or hidden direction controls are inserted.

Run metadata is saved beside the exact plain transcription in the review journal,
with a checked concatenation contract. Older reviews without runs load unchanged.
Run-editor regressions: `node tool/test-transcription-runs.cjs`.
