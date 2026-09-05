# Robinson 1854 source-verifiable sample inventory

Status: bounded visual audit, 2026-09-05. This inventory identifies candidate
regions; it is not a gold transcription and does not satisfy the planned
150–300 verified-line target.

## Source and coordinate authority

- Edition: Robinson 1854, fifth edition.
- Scan: `loc-gdcmassbookdig.hebrewenglishlex00gese`.
- Source PDF SHA-256:
  `466b061e770f212cb7d888d8dadc2a54575fb115bf6de9cdb24b0c280461ccaa`.
- PDF page = printed page + 16 for the audited main-text pages.
- Coordinates below are `[x, y, width, height]` pixels in a direct Poppler
  rendering of the source PDF at 180 dpi, origin at the upper left. Page width
  is 919 or 928 pixels and height is 1570 pixels. These regions must be mapped
  to native source or pipeline coordinates when lines become gold.
- Transcription authority remains the printed scan. OCR and dictionaries may
  propose readings, but a reviewer must check every character against the scan.

The PDF hash above was recomputed with `sha256sum`, not copied without checking.
The inspected rasters were produced with:

```sh
pdftoppm -f PDF_PAGE -l PDF_PAGE -singlefile -r 180 -png SOURCE_PDF /tmp/robinson-audit/pdf-PDF_PAGE
```

| PDF page | Inspected raster | Dimensions | Raster SHA-256 |
|---:|---|---:|---|
| 17 | `/tmp/robinson-audit/pdf-17.png` | 928 x 1570 | `257d02abdf241575f9f5946fd2cd6258270faa94d79c5d1529174ef6c1fe8b56` |
| 66 | `/tmp/robinson-audit/pdf-66.png` | 928 x 1570 | `a9da07fc242a6d7eb98d3ea801f9ff8729e5a1c5ac67d916bb74124a977cf805` |
| 116 | `/tmp/robinson-audit/pdf-116.png` | 928 x 1570 | `c988f5aef13e8b8c6da8d28e240904a92d609660a23eccfac66c8c1756421d35` |
| 191 | `/tmp/robinson-audit/pdf-191.png` | 919 x 1570 | `7ead3d76074c34bc54b493fc8fdf345241bb6b0205536c2fa947d7b2cc32ed79` |
| 266 | `/tmp/robinson-audit/pdf-266.png` | 928 x 1570 | `d2a676b09b2efd90c0cdfe1f97714388680d3120490a22d40504e73f45ece76a` |
| 341 | `/tmp/robinson-audit/pdf-341.png` | 919 x 1570 | `a38291ea88a8626023107c71cb4d02986971b092d97d3cd59fe472c45a6d3d8a` |
| 491 | `/tmp/robinson-audit/pdf-491.png` | 919 x 1570 | `6ad0f7e8bd00969bdc4ea5dc2ef48058df4a665ca9e244e897368924c0fa5c97` |
| 716 | `/tmp/robinson-audit/pdf-716.png` | 928 x 1570 | `1be4ab1449a37988e0f82f473fc36634429610a4dd6ceb993d535191c63cfcba` |
| 791 | `/tmp/robinson-audit/pdf-791.png` | 919 x 1570 | `f0b830fd7df65e86676c53921e471adac4aea8af4101d6b77207737f574247d5` |
| 941 | `/tmp/robinson-audit/pdf-941.png` | 919 x 1570 | `886c2456cdbf7aeead6828baa91b4b1f861ce8a96989b3ece591d31852dc3137` |

These `/tmp` paths describe the audit artifacts and are intentionally not
benchmark inputs. Re-render and verify the recorded hashes before relying on
the pixel coordinates after software or source changes.

## Audited regions

| Printed / PDF page | Partition | Region | Source-observed coverage and reason | Unresolved glyphs or limits |
|---|---|---:|---|---|
| 1 / 17 | excluded | `[35,300,850,1130]` | Latn prose; pointed Hebr; polytonic Grek; Arab; Syrc; opening headwords and mixed-direction comparisons; three shapes identified by the surrounding prose as historical Phoenician Aleph forms | Record the observed shapes diplomatically before deciding whether Unicode `Phnx` is an acceptable interpretation. Already tuned page and existing gold entry, so excluded from final test. |
| 50 / 66 | development | `[490,90,390,1370]` | Latn and pointed Hebr throughout; Grek near top and bottom; several Arab comparisons; dense ordinary two-column entry | Small Arabic points and Hebrew vowel points need tighter crops. No Syrc observed. |
| 100 / 116 | development | `[480,70,405,1390]` | Latn, pointed Hebr, polytonic Grek, and Arab; headword changes and mixed comparison lines | Some small Greek breathings and Arabic marks are unresolved at full-page scale. No Syrc observed. |
| 175 / 191 | validation | `[30,70,850,1390]` | Latn, pointed Hebr, Grek, Arab, and Syrc; dense comparisons, headwords, and a page-edge blemish | Syriac points and an Arabic ligature require tight-crop verification. |
| 250 / 266 | final-test | `[480,70,410,1380]` | Latn, pointed Hebr, Grek, Arab, and Syrc; mixed-direction citations and dense running prose | Syriac word after `Syr.` and fine Hebrew points are unresolved at this scale. |
| 325 / 341 | development | `[35,70,850,1390]` | Latn, pointed Hebr, Grek, Arab, and Syrc; dense prose and comparison strings | `Chald.` examples use square Hebr; no Armi glyphs were observed. Arabic/Syriac points require close transcription. |
| 475 / 491 | validation | `[35,70,850,1390]` | Latn, pointed Hebr, Grek, and Arab; dense senses and an Arabic root comparison | The Arabic token at the upper right needs a tight crop. No Syrc observed. |
| 700 / 716 | development | `[60,180,820,1280]` | Latn, pointed Hebr, Grek, Arab, Syrc, and one Ethi comparison; many headwords and mixed-direction lines | Ethiopic letters near the left-column middle and Syriac points need character-level verification. |
| 775 / 791 | final-test | `[35,70,850,1390]` | Latn, pointed Hebr, polytonic Grek, Syrc, and Ethi; Chaldee material is square Hebr | A connected token near `elsewhere` is provisionally Syrc; verify with a tight crop. No Arab run was verified, despite the old pilot claim. |
| 925 / 941 | validation | `[35,70,850,1390]` | Latn, pointed Hebr, Grek, Arab, and Syrc; late headwords, language labels, botanical comparison, and edge degradation | Fine Syrc/Arab points and the Greek botanical words require tight crops. Chaldee is square Hebr. |

The audit deliberately sampled complete page context before choosing candidate
regions. Coordinates are broad enough to preserve labels and reading-order
evidence; line-level gold work must add exact rectangles and stable anchors.

Priority tight regions for the next transcription pass use the same 180 dpi
coordinate space:

- Printed 700 / PDF 716, `[70,575,370,250]`: the left-column cluster beginning
  with the `Eth.` comparison, followed by a Syriac form and an `Aram.`-labelled
  square-Hebrew form. Preserve the labels in the crop so visible script and
  semantic language can be recorded separately.
- Printed 775 / PDF 791, `[450,615,395,230]`: the right-column Chaldee paragraph
  containing square Hebrew, a connected Syriac form, polytonic Greek, and an
  Ethiopic form. The Syriac identification remains provisional pending a
  character-level crop review.

## Coverage gaps before a representative benchmark exists

- No line in this inventory has yet been diplomatically transcribed or independently
  reviewed. The count of verified gold lines added here is zero.
- Page-level partition unions are uneven. Development and final test each cover
  all six observed Unicode scripts: final-test page 250 supplies Arab and page
  775 supplies Ethi. Validation lacks Ethi. This says nothing about usable line
  counts; preserve the frozen pages and report low or zero per-script counts.
- Damage coverage is limited to page-edge blemishes and uneven contrast. The
  candidate pages 550 and 850 still need visual verification before either is
  described as damaged.
- Front matter, addenda, index, long cross-column entries, and pages 2, 15, 400,
  625, 1000, 1075, 1140, 1155, and 1160 remain unaudited.
- Typography still needing an explicit scoring policy includes historical
  Phoenician forms, small-cap language labels, italic transliteration, combining
  points, polytonic Greek accents, and mixed RTL/LTR punctuation.
- Historical Aleph shapes were observed only on excluded printed page 1. The
  six-script partition union therefore does not provide held-out coverage of
  those glyphs; freeze a separate unseen historical-glyph page before claiming
  final-test coverage for them.

## Next sampling gate

Create exact line crops from the development and validation regions first,
record each crop hash and diplomatic transcription, and have a second reviewer
resolve uncertain glyphs. Freeze any additional final-test pages before looking
at their OCR output. Only then count verified lines toward the 150–300 target.
