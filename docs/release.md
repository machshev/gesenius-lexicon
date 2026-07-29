# OCR draft release checklist

An OCR-draft release does not claim that every entry is human-verified. It does require complete processing and auditable quality.

## Owner decisions before the pilot

- Select the exact Tregelles 1857 scan.
- Add its catalogue URL/local path, rights statement, scan ID, page count, printed-page offset/labels, and lowercase SHA-256 to `sources.toml`.
- Visually confirm the 24 candidate printed pages for each edition in `pilot.toml`; retain the categories even if individual pages change.
- Select a licence-compatible Kraken base recognition model, record its source/licence, and verify its checksum.

Neither the pipeline nor comparison report chooses a canonical edition.

## Pilot gate

- Ground-truth all 24 pages in each edition.
- Keep page-level train/validation/test separation.
- Fine-tune Kraken and publish overall/per-script CER and WER.
- Measure layout accuracy and entry-boundary precision/recall.
- Compare completeness, scan quality, parsing quality, and editorial differences.
- Complete `models/model-card.template.toml` for each released model.

## Full draft gate

- Every registered source passes `gesenius source verify`.
- Every source page is processed by one content-addressed run.
- Every recognized line is assigned to an entry, front matter, or `unparsed`.
- `gesenius validate --run-root ...` reports zero errors.
- Full-book benchmark metrics and the edition comparison report are attached.
- JSONL, TEI, and SQLite are regenerated with a fixed `SOURCE_DATE_EPOCH`.
- TEI passes the pinned RELAX NG profile.
- SQLite passes integrity and foreign-key checks.
- A second rebuild has identical artifact hashes.
- Artifacts and release notes say “OCR draft” and explain the review-state counts.

Large scans, page images, and model weights are release/cache artifacts, not Git content.
