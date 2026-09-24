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
import time

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
    # The text extent uses a low threshold so sparse title pages still span
    # their full width. Gutter detection uses 0.03, which keeps body text
    # (about 0.15) and rejects the smear of a skewed column rule, whose own
    # run is too narrow to count as a span.
    extent = runs(col_ink > 0.001, 40)
    if not extent:
        return [(0, w)]
    left = extent[0][0]
    right = extent[-1][1]
    spans = runs(col_ink > 0.03, 40)
    # A column gutter is a gap of at least 25 px whose centre lies in the
    # middle band of the text extent. Letter-spaced title lines and word gaps
    # also produce gaps, so anything outside that band, or more than one
    # candidate, means the page is read as a single column.
    width = right - left
    gutters = []
    for (a0, a1), (b0, b1) in zip(spans, spans[1:]):
        centre = (a1 + b0) / 2
        if b0 - a1 >= 25 and 0.35 <= (centre - left) / width <= 0.65:
            gutters.append((a1, b0))
    if len(gutters) != 1:
        return [(left, right)]
    g0, g1 = gutters[0]
    return [(left, g0), (g1, right)]


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


def detect_spanning_header(ink: np.ndarray, columns: list[tuple[int, int]]) -> tuple[int, int] | None:
    """Return the [y0, y1) rows of the material set across both columns at the
    top of the page, if any.

    The running head (page number between the column catchwords), and on a
    section's first page the heading, its rule and the section letter, are set
    across the gutter, so their rows have ink inside the gutter. A vertical
    column rule also inks the gutter, but over most of the page height, so
    gutter pixel columns inked on more than 5% of rows, and their neighbours,
    are masked out first; the skewed rule spreads over several such columns. What remains is the
    header ink. Rows with header ink within the top 30% of the page are merged
    into one header while the gaps between them stay under 200 px.
    """
    if len(columns) != 2:
        return None
    h, w = ink.shape
    (l0, l1), (r0, r1) = columns
    # Trim the gutter edges, where letters overhang the column bounds, and
    # dilate the rule mask so the rule's blurred edges do not count either.
    gutter = ink[:, l1 + 10 : r0 - 10]
    if gutter.shape[1] == 0:
        return None
    rule = gutter.mean(axis=0) > 0.05
    rule = np.convolve(rule.astype(int), np.ones(7, dtype=int), mode="same") > 0
    gutter = gutter[:, ~rule]
    if gutter.shape[1] == 0:
        return None
    count = gutter.sum(axis=1)
    top = bottom = None
    for y0, y1 in runs(count >= 3, 4):
        if y0 > h * 0.3:
            break
        if top is None:
            top, bottom = y0, y1
        elif y0 - bottom < 200:
            bottom = y1
        else:
            break
    if top is None:
        return None
    return top, bottom


def header_crop_rows(ink: np.ndarray, columns: list[tuple[int, int]], header: tuple[int, int]) -> tuple[int, int]:
    """Extend header rows to the whole printed rows they belong to.

    Gutter ink covers only the middle of a glyph set across the gutter, such
    as the section letter under a heading. Only the header crop is extended;
    the body still starts at the gutter-derived bottom, so the body chunk plan
    does not move by a few pixels whenever this rule changes.
    """
    top, bottom = header
    h = ink.shape[0]
    row_ink = ink[:, columns[0][0] : columns[-1][1]].mean(axis=1) > 0.002
    while top > 0 and row_ink[top - 1]:
        top -= 1
    while bottom < h and row_ink[bottom]:
        bottom += 1
    return top, bottom


def plan_chunks(raster: pathlib.Path, target: int, maximum: int, pad: int) -> tuple[Image.Image, list[dict]]:
    img = Image.open(raster)
    ink = ink_mask(img)
    h, w = ink.shape
    chunks = []
    columns = detect_columns(ink)
    header = detect_spanning_header(ink, columns)
    body_top = 0
    if header is not None:
        y0, y1 = header_crop_rows(ink, columns, header)
        x0, x1 = columns[0][0], columns[-1][1]
        bx0, bx1 = max(0, x0 - pad), min(w, x1 + pad)
        by0, by1 = max(0, y0 - 6), min(h, y1 + 6)
        chunks.append(
            {
                "chunk_id": "header",
                "column": -1,
                "row": 0,
                "bounds": {"x": bx0, "y": by0, "width": bx1 - bx0, "height": by1 - by0},
            }
        )
        body_top = header[1]
    for ci, (x0, x1) in enumerate(columns):
        col_ink = ink[body_top:, x0:x1]
        for ri, (y0, y1) in enumerate((a + body_top, b + body_top) for a, b in chunk_rows(col_ink, target, maximum)):
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
    # Transient CLI failures (rate limits, overload) exit 1 with nothing on
    # stderr; the reason is in the JSON on stdout. Retry with backoff.
    for attempt in range(4):
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
        if proc.returncode == 0:
            break
        time.sleep(30 * 2**attempt)
    if proc.returncode != 0:
        detail = (proc.stderr.strip() or proc.stdout.strip())[:500]
        raise RuntimeError(f"claude exited {proc.returncode}: {detail}")
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
                if "lines" in chunk and "error" not in chunk:
                    previous[(chunk["image_sha256"], chunk.get("prompt_version"))] = chunk
        except (OSError, ValueError, KeyError):
            previous = {}

    img, chunks = plan_chunks(raster, target, maximum, pad)
    page_chunk_dir = chunk_dir / page_id
    page_chunk_dir.mkdir(parents=True, exist_ok=True)
    for chunk in chunks:
        b = chunk["bounds"]
        # The geometry is part of the file name so a changed chunk plan never
        # reuses a stale crop.
        path = page_chunk_dir / f"{chunk['chunk_id']}-{b['width']}x{b['height']}+{b['x']}+{b['y']}.png"
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
