#!/usr/bin/env python3
"""Assemble whole-page gold fixtures from gold-review sidecars.

Usage:
    tool/gold-assemble.py benchmarks/gold-review/robinson-1854/pdf-0066.json [...] \
        [--out benchmarks/gold] [--check]

Only lines the review marked gold reach a fixture, in reading order, anchored
in the 400 DPI frontier raster frame with the row geometry that
tool/frontier-to-alto.py measures. Contested, unclear and unreviewed lines are
excluded and listed on stderr. Refused: an incomplete sidecar, an incomplete
pass 2 record, a sidecar older than its pass 2 record, a raster whose digest
differs from the record. With --check nothing is written; the command fails if
a fixture on disk differs from what the sidecar produces.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import sys
import tomllib
import unicodedata

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("gold_queue", HERE / "gold-queue.py")
gq = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gq)  # type: ignore[union-attr]
alto, fv = gq.alto, gq.fv

BIDI = set("‎‏؜‪‫‬‭‮⁦⁧⁨⁩")


def source_sha256(edition: str) -> str:
    sources = tomllib.load(open(HERE.parent / "sources.toml", "rb"))["sources"]
    return next(s["sha256"] for s in sources if s["edition"] == edition)


def authority(review: dict, record: dict, excluded: int, total: int) -> str:
    pass1 = json.load(open(record["pass1"], encoding="utf-8"))
    m1 = sorted({c.get("model") for c in pass1["chunks"] if c.get("model")})
    bands = [b for c in record["chunks"] for b in c["bands"]]
    m2 = sorted({b["reread"].get("model") for b in bands if b.get("reread", {}).get("model")})
    second = f" and second reader {review['second_reader']}" if review.get("second_reader") else ""
    return (
        f"Whole page, frontier transcription reviewed independently. Pass 1 {', '.join(m1)}; pass 2 reread and "
        f"adjudication {', '.join(m2)}; blind read and reconciliation by {review['reviewer_model']}{second}, "
        f"{review['generated_at'][:10]}. A line is included only where two of three readers (pass 2, blind, second reader) agree on the final text "
        f"under the comparison key; {excluded} of {total} "
        "lines were contested or unreviewed and are excluded (listed in the review sidecar "
        f"{review['pass2'].replace('corpus/frontier', 'benchmarks/gold-review').replace('/pass2/', '/')}). "
        "Text follows docs/ocr-metric-policy.md, section Gold conventions and provenance."
    )


def line_ids(record: dict) -> list[str]:
    """Per final line, the ALTO id of its row geometry: chunk id and position in the chunk."""
    seen: dict[str, int] = {}
    ids = []
    for line in record["lines"]:
        seen[line["chunk_id"]] = seen.get(line["chunk_id"], 0) + 1
        ids.append(f"{line['chunk_id']}-l{seen[line['chunk_id']]:02d}")
    return ids


def assemble(review: dict, record: dict) -> tuple[dict, list[dict]]:
    if review.get("status") != "complete":
        raise SystemExit(f"refusing pdf {review.get('pdf_page')}: review is {review.get('status')!r} ({review.get('errors')} errors, {review.get('unreviewed')} unreviewed)")
    gq.check_complete(record)
    if review["pass2_generated_at"] != record.get("generated_at"):
        raise SystemExit("review is older than its pass 2 record; rerun tool/gold-review.py")
    raster = pathlib.Path(record["raster"])
    if fv.ft.sha256_file(raster) != record["raster_sha256"] or review["raster_sha256"] != record["raster_sha256"]:
        raise SystemExit(f"raster digest mismatch for {raster}")
    if [ln["index"] for ln in review["lines"]] != list(range(len(record["lines"]))):
        raise SystemExit("review lines do not cover the record's lines in order")
    geometry = {
        ln["id"]: ln["polygon"]
        for region in alto.build(record)["page"]["regions"]
        for ln in region["lines"]
    }
    width, height = record["raster_size"]["width"], record["raster_size"]["height"]
    ids, kept, excluded = line_ids(record), [], []
    for ln, alto_id, final in zip(review["lines"], ids, record["lines"]):
        if ln["text"] != final["text"]:
            raise SystemExit(f"review line {ln['index']} no longer matches the pass 2 text")
        if ln["decision"] != "gold":
            excluded.append(ln)
            continue
        text = ln["text_gold"]
        if not text or text != unicodedata.normalize("NFC", text) or BIDI & set(text):
            raise SystemExit(f"line {ln['index']}: gold text empty, not NFC or holds bidi controls")
        poly = geometry[alto_id]
        x0, y0, x1, y1 = poly[0]["x"], poly[0]["y"], poly[2]["x"], poly[2]["y"]
        if not (0 <= x0 < x1 <= width and 0 <= y0 < y1 <= height):
            raise SystemExit(f"line {ln['index']}: anchor outside the {width}x{height} raster")
        kept.append((final["column"], ln["index"], text, {"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0}))
    kept.sort(key=lambda k: (k[0], k[1]))
    pdf = record["pdf_page"]
    counters: dict[int, int] = {}
    lines = []
    for column, _, text, bounds in kept:
        counters[column] = counters.get(column, 0) + 1
        lines.append(
            {
                "line_id": f"pdf{pdf:04d}-c{column + 1}-{counters[column]:03d}",
                "text": text,
                "source": {"source_page": pdf, "bounds": bounds},
            }
        )
    if not lines:
        raise SystemExit(f"pdf {pdf}: no gold lines")
    fixture = {
        "id": f"{record['edition']}:pdf{pdf:04d}:full",
        "edition": record["edition"],
        "source_page": pdf,
        "source_sha256": source_sha256(record["edition"]),
        "authority": authority(review, record, len(excluded), len(review["lines"])),
        "source_image": {
            "width": width,
            "height": height,
            "coordinate_frame": f"frontier-raster-400dpi-pdf{pdf:04d}-sha256-{record['raster_sha256']}",
        },
        "lines": lines,
    }
    return fixture, excluded


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("review", nargs="+", type=pathlib.Path)
    ap.add_argument("--out", type=pathlib.Path, default=HERE.parent / "benchmarks/gold")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    stale = 0
    for path in args.review:
        review = json.load(open(path, encoding="utf-8"))
        record = json.load(open(review["pass2"], encoding="utf-8"))
        fixture, excluded = assemble(review, record)
        target = args.out / f"robinson-1854-pdf{review['pdf_page']:04d}-full.json"
        text = json.dumps(fixture, ensure_ascii=False, indent=2) + "\n"
        for ln in excluded:
            print(f"  excluded line {ln['index']}: {ln['decision_reason']}", file=sys.stderr)
        print(f"{target.name}: {len(fixture['lines'])} lines, {len(excluded)} excluded", file=sys.stderr)
        if args.check:
            if not target.exists() or target.read_text(encoding="utf-8") != text:
                print(f"STALE {target}", file=sys.stderr)
                stale += 1
        else:
            args.out.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8")
    return 1 if stale else 0


if __name__ == "__main__":
    sys.exit(main())
