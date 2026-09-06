# OCR metric and benchmark policy

The benchmark preserves diplomatic Unicode text. Overall CER compares exact
Unicode scalars and WER compares whitespace-delimited words. Neither score
normalizes historical glyphs or silently repairs combining marks.

`base_letter_cer` and `combining_mark_cer` are diagnostics. They first use canonical
decomposition, so composed and decomposed forms do not create a diagnostic
difference. They do not replace exact CER. The per-script diagnostic attributes
a combining mark to its preceding base character, so pointed Hebrew and accented
Greek remain in the script sample rather than disappearing as `Inherited`.
`reference_characters_by_script` records the denominator for every reported
script score; omitted scripts have no reference sample and are not evidence of
accuracy. Per-script CER filters the corresponding reference and hypothesis
script streams; it is a coverage diagnostic, not an aligned script-confusion
matrix.

The mark diagnostic aligns canonically decomposed alphabetic bases by edit
distance, breaking ties in substitution/deletion/insertion order, then compares
the marks attached to those bases. Marks on substituted bases can match: read this
diagnostic alongside `base_letter_cer`, not as complete word accuracy. Orphan marks
and marks attached to digits or punctuation are excluded from this diagnostic;
exact CER still counts them. Alignment uses linear working memory. Geometry-only
line diagnostics are described below; fully transcribed-region precision,
entry-boundary and oracle-candidate evaluation remain future work.

## Canonical equivalence and historical glyphs

`canonical_equivalence` reports CER/WER after NFC normalization of each stream,
with its own normalized reference character/word denominators. The diplomatic
`cer` and `wer` remain unchanged. NFC permits canonically equivalent compositions
and mark orderings; it does not remove pointing, change case, expand compatibility
ligatures, equate lookalike scripts, or repair spellings. No NFKC/NFKD, dictionary
normalization or language-specific equivalence table is applied. For example,
precomposed `ἄ` and `α` plus smooth breathing and acute have a zero canonical
error, while losing either mark remains an error. The keyboard can therefore
preserve literal combining sequences without hiding canonical equivalence in the
report. This is a scoring operation, not a rewrite of gold or OCR evidence.

A historical glyph is transcribed as an assigned Unicode character only when
its source identification is resolved and recorded in the fixture authority.
Distinct unencoded drawn forms remain unresolved with their crops; exclude the
unresolved line from authoritative gold rather than silently mapping every shape
to a convenient letter or scoring a placeholder as a recognized character.
The existing page-1 fixture's three historical Aleph shapes mapped to Phoenician
Aleph remain a documented legacy interpretation. Neither exact nor NFC scoring
recovers the distinctions it has already discarded. Do not use it to claim
historical-shape accuracy or held-out coverage. No further historical-glyph
substitutions are accepted as score equivalences.

## Aligned scripts and exact foreign tokens

`aligned.characters_by_script` aligns the complete exact-scalar streams **before**
counting by script. It reports reference/hypothesis counts, matches,
substitutions, deletions and insertions. `aligned.substitutions_by_script` maps
reference script to hypothesis script, including same-script substitutions.
Thus `α a` versus `a α` exposes Grek→Latn and Latn→Grek substitutions even though
independently filtered script streams are identical. Insertions are charged to
the hypothesis script; matches, substitutions and deletions to the reference
script. Introduced scripts retain a zero reference count. Combining marks inherit
the most recent non-mark scalar's script; a leading orphan retains its own Unicode
script. Spaces/punctuation and inherited/unknown characters are retained under
Zyyy/Zinh/Zzzz where applicable. This is visible-script evidence, not semantic
language identification. A matching mark can have different inherited context in
the two streams; matches are still exact scalars credited to the reference script.

`aligned.foreign_words` aligns the complete whitespace-token streams, then counts
tokens containing at least one non-Latin alphabetic base. It is an exact
**foreign-containing token** metric, not dictionary segmentation: punctuation,
points, accents, spelling and attached Latin material must all match. A mixed
Greek/Syriac token counts once overall and once in each script's support; per-script
counts are therefore not necessarily additive. Hebrew-script Aramaic remains
Hebr. Latin transliterations are not foreign-script tokens and remain covered by
overall CER/WER and base/mark diagnostics.

The token report includes reference/hypothesis counts, exact matches,
`accuracy = exact_matches / reference_words`, and
`precision = exact_matches / hypothesis_words`. A zero denominator produces
`null`, never a perfect score. Missing reference scripts have no measured accuracy;
a new hypothesis-only script is visible with zero reference support and zero
precision. The exact-token score intentionally does not use NFC; read the
separate canonical score for encoding differences.

Both new alignments use unit-cost Levenshtein with Hirschberg reconstruction,
reported as `levenshtein_hirschberg_v1`. Split the reference at its midpoint,
choose the earliest minimum-cost hypothesis split, and in one-row/column base
cases prefer diagonal, deletion, then insertion while tracing backwards. Time is
quadratic and working storage plus output is linear in stream length. Repeated
or ambiguous text can admit multiple equally optimal alignments; these counts
are reproducible diagnostics, not proof of token correspondence in every such
case. Legacy base/mark diagnostics retain their existing tie rule. Older reports
without `aligned` or `canonical_equivalence` deserialize as absent measurements,
not zero error.

## Initial acceptance tolerances before recognition tuning

The initial measured-change gate allows **0 increase** in aggregate diplomatic
CER or WER on the same frozen development regions, then on validation regions.
For every supported script it also allows **0 increase** in aligned character
errors (substitutions + deletions + insertions), wrong-script substitution counts,
and missed exact foreign-token counts. Aggregate base-letter and combining-mark
CER likewise allow **0 increase**. At least one primary measure (exact CER, exact
WER, or missed exact foreign-token count) must strictly decrease to claim an
accuracy improvement; equality alone is not an improvement. NFC gains are reported
separately and cannot excuse diplomatic regressions.

These are conservative initial experiment gates, not estimated population error
bounds or proof of whole-book support. Compare identical reference support,
aggregate counts before taking rates, and inspect changed source examples. Zero
or missing coverage is ineligible for an accuracy claim. The representative gold
sample and source-checked baseline must exist before applying these gates to
recognizer selection. If measured tradeoffs later require different tolerances,
record the revised contract before a new independent evaluation; a final-test
result used to change policy retires that test material to development. Runtime,
routing-specific recall and false-route limits still need separate measurements
before their corresponding later milestones.

## Source alignment contract

A benchmark fixture has an immutable edition, source PDF page, and PDF digest.
The legacy `evaluate_alto` API only joins lines by engine-generated line ID. Its
result is always `source_identity: unverified`, including for old fixtures, and
must not be represented as proof that the evaluated ALTO came from the gold
page.

New callers use `evaluate_alto_with_identity` and pass an asserted
`SourceIdentity`. The evaluator rejects a different edition, page, or digest.
The assertion establishes a checked comparison contract; it does not itself
cryptographically bind an arbitrary ALTO file to the source PDF.

Coordinate-aligned fixtures provide every gold line with finite, non-degenerate
same-page rectangular bounds inside declared source-image dimensions and name the
image coordinate frame. They require matching asserted source identity and
coordinate frame, and the evaluator rejects ALTO with different image dimensions.
This is a caller assertion, not proof that an ALTO transform uses that frame. It
selects each whole hypothesis line whose bounding box has any positive overlap
with a gold rectangle, keeps the ALTO region/line order and each hypothesis line
only once, then compares the two ordered streams. No overlap threshold is applied
until a representative sample supports one. This permits engine line-ID changes,
a source line split into multiple hypotheses, and several gold lines merged into
one hypothesis. ALTO text is already stored in logical order, so no visual-order
reversal is applied to RTL content.

Legacy scoring preserves its line-break text exactly. Coordinate scoring reports
`coordinate_whitespace_normalized`: it trims selected lines and joins their
boundaries with one space before CER/WER. This boundary policy makes split and
merged-line text comparable but is not a claim that the original whitespace was
identical.

Anchors should cover fully transcribed source regions. If a selected OCR line
extends into untranscribed neighbouring material, it can appear as an extra-text
error; do not use such a partial region for a headline accuracy result. Missing
anchor coverage, missing source identity, or an old line-ID-only fixture remains
visible in the result rather than being treated as verified accuracy.

## Line segmentation diagnostics

Coordinate-aligned results also report `line_segmentation`, using the same
positive axis-aligned bounding-box overlap rule as text selection. This adds no
new tuned threshold and does not change CER/WER selection. It records:

- Source-anchor count, selected OCR-line count and missing source-line count.
- One-to-one correspondences: exactly one OCR line touches a source line, and
  that OCR line touches no other source line.
- Split candidates: multiple OCR lines touch one source line.
- Merge candidates: one OCR line touches multiple source lines.
- The full overlap graph, with flattened reading-order indices to distinguish
  repeated engine line IDs.

Split candidates may also be duplicate OCR lines; merge candidates may be overly
large bounding boxes. Both labels describe geometry evidence rather than an
independently verified segmentation mistake. Many-to-many overlaps appear in both
candidate lists, never as one-to-one. Mere edge contact is not positive overlap.
A perfectly transcribed merged line can have zero CER/WER and still appear as a
merge candidate; conversely, one-to-one geometry does not imply correct text.

OCR lines outside every anchor are counted as `unassessed_hypothesis_lines` and
retained in the graph with no source-line matches. They are not automatically
false positives: gold line anchors do not declare that intervening or surrounding
page areas are fully transcribed. Measuring extra-line precision requires explicit
fully transcribed evaluation regions, which remain future work along with
entry-boundary scoring. Legacy line-ID results report `line_segmentation: null`;
missing geometry is not a perfect segmentation score.
