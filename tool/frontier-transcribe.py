#!/usr/bin/env python3
"""Transcribe lexicon page rasters with a frontier vision model via the claude CLI.

Pass 1 of the frontier transcription stage. Each page raster is split into
columns by ink projection, each column is cut into chunks at inter-line
whitespace so no printed line is severed, and each chunk is read by the
model through ``claude -p`` with a JSON schema. The output is one JSON
record per page holding every chunk's geometry, image digest, model lines
and usage, plus the concatenated reading-order text.

Usage:
    tool/frontier-transcribe.py --edition robinson-1854 \
        --raster .cache/gesenius/frontier/robinson-1854/raster/pdf-0017.png \
        [--raster ...] --output corpus/frontier/robinson-1854 [--workers 4]

Chunk images are written beside the raster under ``chunks/<page>/`` and are
not committed; the JSON records are small and are meant to be committed.
Existing chunk results with a matching image digest and prompt digest are
reused, so reruns only pay for new or changed chunks.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import json
import os
import pathlib
import subprocess
import sys

import numpy as np
from PIL import Image

PROMPT_VERSION = 2
PROMPT = (
    "Read the image file {path}. It is a crop of one column of a printed 1854 "
    "Hebrew-English lexicon (Gesenius, Robinson translation). Transcribe every "
    "complete printed text line exactly, one array element per printed line, top "
    "to bottom. Preserve spelling, capitalisation, punctuation, printed "
    "abbreviations, section marks and end-of-line hyphenation. Hebrew, Aramaic, "
    "Arabic, Syriac, Greek and Ethiopic must be Unicode text with every visible "
    "vowel point, dagesh, accent and diacritic, written in logical order with no "
    "bidi control characters; keep the words of a right-to-left phrase in the "
    "order they are read, and keep the printed left-to-right order of separate "
    "items in a list. Where a page or column header, page number or catchword is "
    "present, include it as its own line. If a line is only partially visible at "
    "the top or bottom edge, omit it entirely rather than guessing. Never add, "
    "reorder, translate, normalise or omit anything that is fully visible. If the "
    "crop contains no text, return an empty array."
)
SCHEMA = json.dumps(
    {
        "type": "object",
        "properties": {"lines": {"type": "array", "items": {"type": "string"}}},
        "required": ["lines"],
    }
)


def sha256_file(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def ink_mask(img: Image.Image) -> np.ndarray:
    grey = np.asarray(img.convert("L"), dtype=np.uint8)
    # Otsu-free: page paper is light, ink is dark; threshold at a fixed fraction
    # below the page median so stains do not count as ink.
    median = np.median(grey)
    return grey < max(60, median - 70)


def runs(mask_1d: np.ndarray, min_len: int) -> list[tuple[int, int]]:
    """Return [start, end) runs of True at least min_len long."""
    out = []
    start = None
    for i, v in enumerate(mask_1d):
        if v and start is None:
            start = i
        elif not v and start is not None:
            if i - start >= min_len:
                out.append((start, i))
            start = None
    if start is not None and len(mask_1d) - start >= min_len:
        out.append((start, len(mask_1d)))
    return out


def detect_columns(ink: np.ndarray) -> list[tuple[int, int]]:
    """Return [x0, x1) column spans found by vertical ink projection."""
    h, w = ink.shape
    body = ink[int(h * 0.12) : int(h * 0.92)]
    col_ink = body.mean(axis=0)
    # 0.03 keeps text (about 0.15) and rejects the smear of a skewed column
    # rule, whose own run is too narrow to count as a span.
    text_cols = col_ink > 0.03
    spans = runs(text_cols, 40)
    if not spans:
        return [(0, w)]
    left = spans[0][0]
    right = spans[-1][1]
    # Gutters: gaps between text runs wider than 25 px, inside the text extent.
    gaps = []
    for (a0, a1), (b0, b1) in zip(spans, spans[1:]):
        if b0 - a1 >= 25:
            gaps.append((a1, b0))
    # Merge spans across narrow gaps to produce columns.
    columns = []
    start = left
    for g0, g1 in gaps:
        columns.append((start, g0))
        start = g1
    columns.append((start, right))
    # Drop slivers (rules, noise) narrower than 12% of the text width.
    width = right - left
    columns = [c for c in columns if c[1] - c[0] >= width * 0.12]
    if not columns:
        return [(left, right)]
    return columns


def chunk_rows(ink_col: np.ndarray, target: int, maximum: int) -> list[tuple[int, int]]:
    """Cut a column into [y0, y1) chunks at whitespace rows near the target height."""
    h = ink_col.shape[0]
    row_ink = ink_col.mean(axis=1)
    text_rows = row_ink > 0.002
    if not text_rows.any():
        return []
    ys = np.flatnonzero(text_rows)
    top, bottom = int(ys[0]), int(ys[-1]) + 1
    cuts = [top]
    y = top
    while bottom - y > maximum:
        lo, hi = y + int(target * 0.7), min(y + maximum, bottom)
        window = row_ink[lo:hi]
        # Prefer the centre of the widest blank run near the target.
        blank = window <= 0.002
        blank_runs = runs(blank, 4)
        if blank_runs:
            best = max(blank_runs, key=lambda r: (r[1] - r[0], -abs((r[0] + r[1]) // 2 + lo - (y + target))))
            cut = lo + (best[0] + best[1]) // 2
        else:
            cut = lo + int(np.argmin(window))
        cuts.append(cut)
        y = cut
    cuts.append(bottom)
    return list(zip(cuts, cuts[1:]))


def plan_chunks(raster: pathlib.Path, target: int, maximum: int, pad: int) -> tuple[Image.Image, list[dict]]:
    img = Image.open(raster)
    ink = ink_mask(img)
    h, w = ink.shape
    chunks = []
    for ci, (x0, x1) in enumerate(detect_columns(ink)):
        col_ink = ink[:, x0:x1]
        for ri, (y0, y1) in enumerate(chunk_rows(col_ink, target, maximum)):
            # Wide horizontal padding keeps marginal glyphs; tight vertical
            # padding keeps slivers of the neighbouring line out of the crop.
            bx0, bx1 = max(0, x0 - pad), min(w, x1 + pad)
            by0, by1 = max(0, y0 - 6), min(h, y1 + 6)
            chunks.append(
                {
                    "chunk_id": f"c{ci}-r{ri:02d}",
                    "column": ci,
                    "row": ri,
                    "bounds": {"x": bx0, "y": by0, "width": bx1 - bx0, "height": by1 - by0},
                }
            )
    return img, chunks


def call_claude(image_path: pathlib.Path, model: str, timeout: int) -> dict:
    prompt = PROMPT.format(path=image_path)
    cmd = [
        "claude",
        "-p",
        "--model",
        model,
        "--allowedTools",
        "Read",
        "--output-format",
        "json",
        "--json-schema",
        SCHEMA,
        prompt,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
    if proc.returncode != 0:
        raise RuntimeError(f"claude exited {proc.returncode}: {proc.stderr.strip()[:500]}")
    result = json.loads(proc.stdout)
    structured = result.get("structured_output")
    if not isinstance(structured, dict) or "lines" not in structured:
        raise RuntimeError(f"no structured output: {str(result.get('result'))[:300]}")
    usage = result.get("usage", {})
    return {
        "lines": [str(line) for line in structured["lines"]],
        "model": model,
        "session_id": result.get("session_id"),
        "duration_api_ms": result.get("duration_api_ms"),
        "cost_usd_list": result.get("total_cost_usd"),
        "input_tokens": usage.get("input_tokens"),
        "cache_read_input_tokens": usage.get("cache_read_input_tokens"),
        "cache_creation_input_tokens": usage.get("cache_creation_input_tokens"),
        "output_tokens": usage.get("output_tokens"),
    }


def transcribe_page(
    raster: pathlib.Path,
    edition: str,
    output_dir: pathlib.Path,
    chunk_dir: pathlib.Path,
    model: str,
    workers: int,
    target: int,
    maximum: int,
    pad: int,
    timeout: int,
) -> pathlib.Path:
    page_id = raster.stem  # e.g. pdf-0017
    pdf_page = int(page_id.split("-")[-1])
    out_path = output_dir / f"{page_id}.json"
    previous = {}
    if out_path.exists():
        try:
            for chunk in json.load(open(out_path, encoding="utf-8")).get("chunks", []):
                previous[(chunk["image_sha256"], chunk.get("prompt_version"))] = chunk
        except (OSError, ValueError, KeyError):
            previous = {}

    img, chunks = plan_chunks(raster, target, maximum, pad)
    page_chunk_dir = chunk_dir / page_id
    page_chunk_dir.mkdir(parents=True, exist_ok=True)
    for chunk in chunks:
        b = chunk["bounds"]
        path = page_chunk_dir / f"{chunk['chunk_id']}.png"
        if not path.exists():
            img.crop((b["x"], b["y"], b["x"] + b["width"], b["y"] + b["height"])).save(path, optimize=True)
        chunk["image_path"] = str(path)
        chunk["image_sha256"] = sha256_file(path)
        chunk["prompt_version"] = PROMPT_VERSION

    todo = [c for c in chunks if (c["image_sha256"], PROMPT_VERSION) not in previous]
    print(f"{page_id}: {len(chunks)} chunks, {len(todo)} to transcribe", file=sys.stderr, flush=True)

    def work(chunk: dict) -> tuple[dict, dict | None, str | None]:
        try:
            return chunk, call_claude(pathlib.Path(chunk["image_path"]), model, timeout), None
        except Exception as exc:  # noqa: BLE001 - recorded per chunk, run continues
            return chunk, None, str(exc)

    results: dict[str, dict] = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        for chunk, reading, error in pool.map(work, todo):
            if error:
                print(f"  {page_id} {chunk['chunk_id']}: ERROR {error}", file=sys.stderr, flush=True)
                results[chunk["chunk_id"]] = {"error": error}
            else:
                print(
                    f"  {page_id} {chunk['chunk_id']}: {len(reading['lines'])} lines, "
                    f"{reading['duration_api_ms']} ms",
                    file=sys.stderr,
                    flush=True,
                )
                results[chunk["chunk_id"]] = reading

    records = []
    for chunk in chunks:
        key = (chunk["image_sha256"], PROMPT_VERSION)
        record = dict(chunk)
        if key in previous:
            old = previous[key]
            record.update({k: old[k] for k in old if k not in record})
        else:
            record.update(results.get(chunk["chunk_id"], {"error": "not attempted"}))
        record["image_path"] = os.path.relpath(record["image_path"])
        records.append(record)

    text_lines = []
    for record in records:
        for line in record.get("lines", []):
            text_lines.append({"chunk_id": record["chunk_id"], "column": record["column"], "text": line})

    page = {
        "schema": "gesenius-frontier-transcription/1",
        "edition": edition,
        "pdf_page": pdf_page,
        "raster": os.path.relpath(raster),
        "raster_sha256": sha256_file(raster),
        "raster_size": {"width": img.width, "height": img.height},
        "pass": 1,
        "method": "claude-cli-print-json-schema",
        "prompt_version": PROMPT_VERSION,
        "prompt": PROMPT,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "chunks": records,
        "lines": text_lines,
        "errors": sum(1 for r in records if "error" in r),
        "cost_usd_list_total": round(sum(r.get("cost_usd_list") or 0 for r in records), 4),
    }
    output_dir.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_suffix(".json.tmp")
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(page, fh, ensure_ascii=False, indent=1)
        fh.write("\n")
    os.replace(tmp, out_path)
    return out_path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--edition", required=True)
    ap.add_argument("--raster", action="append", required=True, type=pathlib.Path)
    ap.add_argument("--output", required=True, type=pathlib.Path)
    ap.add_argument("--chunk-dir", type=pathlib.Path, default=None)
    ap.add_argument("--model", default="claude-fable-5-1")
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--target-height", type=int, default=850)
    ap.add_argument("--max-height", type=int, default=1000)
    ap.add_argument("--pad", type=int, default=24)
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument("--plan-only", action="store_true", help="write chunk images and geometry without calling the model")
    args = ap.parse_args()

    chunk_dir = args.chunk_dir or (args.raster[0].parent.parent / "chunks")
    for raster in args.raster:
        if args.plan_only:
            img, chunks = plan_chunks(raster, args.target_height, args.max_height, args.pad)
            page_chunk_dir = chunk_dir / raster.stem
            page_chunk_dir.mkdir(parents=True, exist_ok=True)
            for chunk in chunks:
                b = chunk["bounds"]
                img.crop((b["x"], b["y"], b["x"] + b["width"], b["y"] + b["height"])).save(
                    page_chunk_dir / f"{chunk['chunk_id']}.png", optimize=True
                )
            print(raster.stem, json.dumps([(c["chunk_id"], c["bounds"]) for c in chunks]))
            continue
        out = transcribe_page(
            raster,
            args.edition,
            args.output,
            chunk_dir,
            args.model,
            args.workers,
            args.target_height,
            args.max_height,
            args.pad,
            args.timeout,
        )
        print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
