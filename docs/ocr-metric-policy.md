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
exact CER still counts them. Alignment uses linear working memory. Exact
foreign-word accuracy, aligned script confusions, and independent segmentation
metrics remain future work.

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
