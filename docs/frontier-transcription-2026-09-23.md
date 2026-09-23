# Frontier-model page transcription, pilot run 2026-09-23

Status: pass 1 complete for all 24 Robinson 1854 pilot pages. Records are
committed under `corpus/frontier/robinson-1854/`. This supersedes the Kraken
headword-recognizer trajectory as the route to the structured Unicode
datafile; see the decision below.

## Why the direction changed

The project goal is a structured Unicode datafile of the Gesenius lexicon,
including the argued etymology in entry bodies, not a general OCR engine. On
2026-09-23 a survey found no existing Unicode transcription of either
English Gesenius: Blue Letter Bible serves Tregelles entries as scanned
images, the e-Sword "module" links to archive.org, the Internet Archive OCR
text contains no Hebrew characters, and Wikisource has only the Grammar.
BDB is fully digitised but states conclusions without Gesenius's reasoning,
and Haqor already carries it.

The Kraken route had reached 89.6% character accuracy on headwords alone
after 344 reviewed pairs, with the reviewed set as the binding constraint.
A vision model reading full column crops reads every script on the page,
including the entry bodies, with no training data. The existing gold
fixtures were themselves frontier-drafted, so this was already the most
accurate reader in the repository.

## Method

`tool/frontier-transcribe.py` splits each 400 DPI raster into columns by
vertical ink projection (a single gutter of at least 25 px in the middle
band of the text extent; otherwise one column), cuts each column into
chunks of at most 1000 px at inter-line whitespace so no printed line is
severed, pads 24 px horizontally and 6 px vertically, and reads each chunk
through `claude -p` with a JSON schema requiring an array of lines. The
prompt asks for diplomatic transcription, Unicode for every script with all
visible points, logical order without bidi controls, and omission of any
line cut at a chunk edge. Chunk PNGs are named by geometry and cached under
`.cache/gesenius/frontier/`; results are reused by image digest and prompt
version, so reruns pay only for changed chunks.

Each page record holds the raster digest, every chunk's bounds, image
digest, model lines, session id, duration, token counts and list-price
cost, plus the concatenated reading-order lines.

## Result

| Measure | Value |
| --- | --- |
| Pages | 24 (PDF 5, 9, 15, 17, 18, 31, 66, 116, 191, 266, 341, 416, 491, 566, 641, 716, 791, 866, 941, 1016, 1091, 1156, 1171, 1176) |
| Chunks | 183, median 19.7 s, mean 23.5 s each |
| Lines produced | 2,310 |
| Chunk failures | 0 after resume |
| List-price cost recorded by the CLI | $47.75, about $2.00 per page |

Scored with `tool/score-frontier-transcription.py` against the three gold
fixtures, matching each gold line to its most similar page line:

| Gold fixture | Gold lines | Exact | Character accuracy |
| --- | --- | --- | --- |
| Page 1, whole Aleph entry (PDF 17) | 79 | 59 | 95.6% |
| Page 50, right column top (PDF 66) | 12 | 9 | 99.1% |
| Page 700, left column comparisons (PDF 716) | 9 | 8 | 99.7% |

Every gold line was matched; none was missing. English prose, references
and Greek were essentially perfect. The residual differences on page 1 fall
into these classes:

- **Gold errors.** Zooming the page at 2x shows the fixture is wrong in at
  least four places: it has הִקְטִיל where the page prints הִתְקַטֵּל, misreads
  two words in the following Arabic and Hebrew line, and gives the Syriac
  for "flower" as ܐܒܪ where the page has ܗܒܒܐ. The model reading is closer
  in each case. The gold was frontier-drafted and visually confirmed, so it
  is not a reliable judge on foreign-script tokens.
- **Trivial normalisation.** A space before a semicolon, a curly versus
  straight apostrophe, theta glyph variants (ϑ/θ), and a combining mark
  chosen differently for the two-dots-above numeral sign.
- **Order of items in a list.** Two lines have adjacent Hebrew items
  swapped, for example "אֲנַחְנוּ, נַחְנוּ". This is a systematic
  right-to-left sequencing error within a left-to-right line.
- **Holem versus holem-vav.** Two lines drop the vav in a plene spelling
  (אֲדַרְכֹּן for אֲדַרְכּוֹן).
- **Dictionary-driven normalisation.** On page 50 the model wrote אֵלִים
  where the page prints the defective אֵלִם, twice. The gold conventions
  explicitly forbid inserting the yod. This is the most important class
  because it is invisible without the image.
- **Arabic and Syriac.** One Arabic verb was read as Syriac, and one
  Syriac word acquired invented vowels. Syriac points were also omitted on
  page 700 (the one miss there).

Two chunk-plan artefacts remain: a heading spanning both columns
("LEXICON." on page 1) is split into fragments, and the model returned no
lines for the left fragment. Blank chunks at the foot of the final page are
genuinely blank.

## Comparison with the previous route

| System | Scope | Clean-set result |
| --- | --- | --- |
| Tesseract isolated word pass, in the pipeline | headwords | 30.5% chars, 1 of 24 exact |
| Kraken fine-tune, 50k synthetic + 274 real | headwords | 89.6% chars, 15 of 24 exact |
| Frontier pass 1, no training | whole page, all scripts | 95.6% to 99.7% chars by fixture |

The headword and page numbers are not the same unit, but the headword
lines inside the page results are read at least as well as the rest, and
the page result covers the entry bodies the headword route never touched.

## Decision

Pass 1 frontier transcription replaces Tesseract-plus-Kraken as the
recognition stage. The Rust rasteriser, source verification, entry
segmentation, review UI and JSONL/TEI/SQLite exports remain the delivery
path. The Kraken training, synthetic renderer and headword review queues
are retired from the critical path; their artefacts and reports stay for
reference.

## Next

1. **Pass 2, token verification.** Re-read every non-Latin token at 2x
   zoom with its line as context, compare against pass 1, and flag
   disagreements. This targets list order, plene/defective spelling,
   dropped points and Arabic/Syriac guesses, which together account for
   nearly all real errors.
2. **Spanning headers.** Detect rows at the top of a page whose ink spans
   both columns and read them as a separate full-width chunk.
3. **Feed lines into entry segmentation.** Map the frontier lines onto the
   existing corpus model so pilot pages can be materialised and exported.
4. **Regenerate the gold** through pass 1 plus pass 2 plus reviewer check,
   since the current fixtures are unreliable on foreign scripts.
5. **Full book.** At about 8 chunks and 3 to 4 minutes of wall time per
   page with six workers, the remaining 1,162 pages are roughly 9,300
   chunks, about 60 to 70 hours of CLI time at this concurrency, or about
   $2,300 at list price if run through the API instead.
