#!/usr/bin/env python3
"""Convert frontier page transcriptions into the pipeline's in-memory ALTO page.

The frontier records keep chunk geometry but not line geometry. Entry
segmentation needs line positions and left edges to find indented entry
starts, so this measures them from the page raster: inside each chunk, rows
dense enough to hold letter bodies are smoothed so points merge with their
letters, cores nearer than 0.6 of a line pitch are merged, and over-tall
cores are split at their thinnest row. The transcribed lines of the chunk
are then aligned to those rows in order, by comparing each line's length
with each row's ink extent, allowing a row to be skipped or shared when the
counts differ. Word boxes are proportional to character counts along the
measured line extent; they locate headword crops, not glyph boundaries.

The output JSON holds the canonical `page` (the final reading), optional
`hypotheses` (the pass 1 draft on the same geometry), and the provenance the
importer needs: raster path and digest, engine and model identity, and the
per-line status from pass 2.

Usage:
    tool/frontier-to-alto.py --transcription corpus/frontier/robinson-1854/pass2/pdf-0066.json \
        [--transcription ...] --output corpus/frontier/robinson-1854/alto
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import sys
import unicodedata

import numpy as np
from PIL import Image

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("frontier_transcribe", HERE / "frontier-transcribe.py")
ft = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ft)  # type: ignore[union-attr]

SCHEMA = "gesenius-frontier-alto/1"
PITCH = 56  # printed line pitch at 400 DPI
CONFIDENCE = {
    "agreed": 0.99,
    "adjudicated": 0.95,
    "draft_only": 0.8,
    "reread_only": 0.8,
    "unresolved": 0.7,
    "pass1": 0.9,
}


POSSESSIVE = re.compile(r"(?<=\w)'(?=s\b)")


def printed_glyphs(text: str) -> str:
    """Documented conversion from frontier output to gold and ALTO text (NFC).

    The model types a straight apostrophe where the 1854 print has a
    typographic one; between a word and a possessive s the mapping is
    unambiguous, so it is applied. Every other straight quote, and the
    cursive theta (the model always writes theta, so its form cannot be
    recovered from the text), are left for the reviewer. See
    docs/ocr-metric-policy.md, "Gold conventions and provenance".
    """
    return POSSESSIVE.sub("\u2019", unicodedata.normalize("NFC", text))


def detect_rows(ink: np.ndarray) -> list[tuple[int, int]]:
    row = ink.mean(axis=1)
    smooth = np.convolve(row, np.ones(9) / 9, mode="same")
    cores = ft.runs(smooth > 0.03, 14)
    merged: list[tuple[int, int]] = []
    for a, b in cores:
        if merged and (a + b) / 2 - sum(merged[-1]) / 2 < 0.6 * PITCH:
            merged[-1] = (merged[-1][0], b)
        else:
            merged.append((a, b))
    rows = []
    for a, b in merged:
        while b - a > 1.6 * PITCH:
            lo, hi = a + int(0.6 * PITCH), b - int(0.6 * PITCH)
            cut = lo + int(np.argmin(smooth[lo:hi])) if hi > lo else (a + b) // 2
            rows.append((a, cut))
            a = cut
        rows.append((a, b))
    return rows


def row_extent(ink: np.ndarray, y0: int, y1: int) -> tuple[int, int] | None:
    """Return the [x0, x1) ink extent of a row, ignoring specks at either end.

    Paper specks beside the first letter would hide a paragraph indent, so
    ink clusters at the ends that are separated from the rest by a gap and
    hold fewer than 40 ink pixels are dropped; the smallest printed mark at
    a line start, an asterisk, holds several hundred.
    """
    profile = ink[y0:y1].sum(axis=0)
    cols = np.flatnonzero(profile >= 2)
    if cols.size == 0:
        return None
    breaks = np.flatnonzero(np.diff(cols) > 6)
    starts = np.concatenate(([0], breaks + 1))
    ends = np.concatenate((breaks, [cols.size - 1]))
    mass = [int(profile[cols[s] : cols[e] + 1].sum()) for s, e in zip(starts, ends)]
    lo, hi = 0, len(mass) - 1
    while lo < hi and mass[lo] < 40:
        lo += 1
    while hi > lo and mass[hi] < 40:
        hi -= 1
    return int(cols[starts[lo]]), int(cols[ends[hi]]) + 1


def align(lengths: list[float], widths: list[float]) -> list[int]:
    """Assign each line an index into rows, monotonically.

    Moves: match line i to row j; skip row j; or put line i on the same row
    as line i-1. Cost of a match is the difference between the line's share
    of a full line's characters and the row's share of the column width.
    """
    n, m = len(lengths), len(widths)
    if n == m:
        return list(range(n))
    inf = float("inf")
    # cost[i][j]: lines [0, i) placed, rows [0, j) consumed, last line on row j-1
    cost = [[inf] * (m + 1) for _ in range(n + 1)]
    back: list[list[tuple[int, int] | None]] = [[None] * (m + 1) for _ in range(n + 1)]
    cost[0][0] = 0.0
    for i in range(n + 1):
        for j in range(m + 1):
            c = cost[i][j]
            if c == inf:
                continue
            if i < n and j < m:
                v = c + abs(lengths[i] - widths[j])
                if v < cost[i + 1][j + 1]:
                    cost[i + 1][j + 1], back[i + 1][j + 1] = v, (i, j)
            if j < m and i < n:
                # skip row j (a stray mark or a rule) only before placing line i
                v = c + 0.8
                if v < cost[i][j + 1]:
                    cost[i][j + 1], back[i][j + 1] = v, (i, j)
            if i < n and j > 0 and i > 0:
                v = c + 0.8 + lengths[i]
                if v < cost[i + 1][j]:
                    cost[i + 1][j], back[i + 1][j] = v, (i, j)
    # trailing rows may be skipped
    j = min(range(m + 1), key=lambda k: cost[n][k] + 0.8 * (m - k))
    out = [0] * n
    i = n
    while i > 0:
        pi, pj = back[i][j]  # type: ignore[misc]
        if pi == i - 1:
            out[i - 1] = j - 1
        i, j = pi, pj
    return out


def polygon(x0: int, y0: int, x1: int, y1: int) -> list[dict]:
    return [{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}]


def words_for(text: str, x0: int, x1: int, y0: int, y1: int, confidence: float, line_id: str) -> list[dict]:
    tokens = text.split()
    total = sum(len(t) for t in tokens) + max(len(tokens) - 1, 0)
    out = []
    pos = 0
    span = max(x1 - x0, 1)
    for k, token in enumerate(tokens):
        a = x0 + int(span * pos / max(total, 1))
        pos += len(token)
        b = x0 + max(int(span * pos / max(total, 1)), a - x0 + 1)
        pos += 1
        out.append({"id": f"{line_id}-w{k + 1:02d}", "polygon": polygon(a, y0, b, y1), "text": token, "confidence": confidence})
    return out


def chunk_tier(chunk: dict) -> int:
    """The tier a chunk belongs to: the tier field, else the chunk id
    (`t1-c0-r00`, `section-1`; unprefixed chunks are tier 0)."""
    if chunk.get("tier") is not None:
        return int(chunk["tier"])
    m = re.match(r"(?:t|section-)(\d+)", chunk["chunk_id"])
    return int(m.group(1)) if m else 0


HEAD_KEEP = {"\u05c1", "\u05c2"}  # shin and sin dot are printed even in running heads


def unpoint_running_head(text: str) -> str:
    """Running heads are printed unpointed. Drop Hebrew vowel points, dagesh,
    meteg, qamats qatan and accents (U+0591 to U+05C7) but keep the shin and
    sin dots and the maqaf. NFC. Applied to the header region only, after
    pass 2, so cached records need no new calls."""
    out = [
        c for c in unicodedata.normalize("NFD", text)
        if not ("\u0591" <= c <= "\u05c7" and c not in HEAD_KEEP and c != "\u05be")
    ]
    return unicodedata.normalize("NFC", "".join(out))


def is_full_width(chunk: dict) -> bool:
    return chunk["column"] < 0


def region_id_for(chunk: dict) -> str:
    """header for the running head, section-N for a mid-page section heading,
    column-N for tier 0 columns and tN-column-M for later tiers."""
    tier = chunk_tier(chunk)
    if is_full_width(chunk):
        return "header" if tier == 0 else f"section-{tier}"
    prefix = "" if tier == 0 else f"t{tier}-"
    return f"{prefix}column-{chunk['column'] + 1}"


def chunk_order(chunk: dict) -> tuple:
    """Reading order: running head, tier 0 columns, then for each later tier its
    section heading followed by its columns."""
    tier = chunk_tier(chunk)
    return (tier, 0 if is_full_width(chunk) else 1, chunk["column"], chunk["row"])


def region_order(region_id: str) -> tuple:
    m = re.match(r"(?:t(\d+)-)?column-(\d+)$", region_id)
    if region_id == "header":
        return (-1, 0, 0)
    if region_id.startswith("section-"):
        return (int(region_id[8:]), 0, 0)
    return (int(m.group(1) or 0), 1, int(m.group(2)))


def build(record: dict) -> dict:
    raster_path = pathlib.Path(record["raster"])
    img = Image.open(raster_path)
    ink = ft.ink_mask(img)
    pass2 = record.get("pass") == 2
    chunks = {c["chunk_id"]: c for c in record["chunks"]}
    grouped: dict[str, list[dict]] = {}
    for line in record["lines"]:
        grouped.setdefault(line["chunk_id"], []).append(line)

    regions: dict[str, dict] = {}
    draft_regions: dict[str, dict] = {}
    statuses = []
    order = sorted(grouped, key=lambda cid: chunk_order(chunks[cid]))
    for cid in order:
        chunk = chunks[cid]
        b = chunk["bounds"]
        cx0, cy0 = b["x"], b["y"]
        crop = ink[cy0 : cy0 + b["height"], cx0 : cx0 + b["width"]]
        lines = grouped[cid]
        region_id = region_id_for(chunk)
        region = regions.setdefault(region_id, {"id": region_id, "lines": [], "_box": [10**9, 10**9, 0, 0]})
        draft_region = draft_regions.setdefault(region_id, {"id": region_id, "lines": [], "_box": [10**9, 10**9, 0, 0]})
        rows = detect_rows(crop)
        if not rows:
            rows = [(0, b["height"])]
        if chunk["column"] < 0:
            # The running head is one printed row holding catchword, page
            # number and catchword, which the model returns as separate lines.
            ya, yb = rows[0][0], rows[-1][1]
            ext = row_extent(crop, ya, yb) or (0, b["width"])
            step = (ext[1] - ext[0]) / len(lines)
            placed = [
                (ya, yb, int(ext[0] + step * k), int(ext[0] + step * (k + 1))) for k in range(len(lines))
            ]
        else:
            extents = [row_extent(crop, a, z) or (0, 1) for a, z in rows]
            full = max(1, int(np.median([len(ln["text"]) for ln in lines])) if lines else 1)
            full = max(full, max(len(ln["text"]) for ln in lines) * 0.9)
            lengths = [len(ln["text"]) / full for ln in lines]
            widths = [(e[1] - e[0]) / b["width"] for e in extents]
            assignment = align(lengths, widths)
            placed = []
            for k, j in enumerate(assignment):
                ya, yb = rows[j]
                xa, xb = extents[j]
                sharing = [i for i, jj in enumerate(assignment) if jj == j]
                if len(sharing) > 1:
                    # Several lines on one measured row: split the row vertically.
                    h = (yb - ya) / len(sharing)
                    s = sharing.index(k)
                    ya, yb = int(ya + h * s), int(ya + h * (s + 1))
                placed.append((ya, yb, xa, xb))
        for k, (line, (ya, yb, xa, xb)) in enumerate(zip(lines, placed)):
            line_id = f"{cid}-l{k + 1:02d}"
            x0, x1 = cx0 + xa, cx0 + xb
            # The core rows only: indentation is judged in line heights, so
            # padding the box would make real indents look shallow.
            y0, y1 = cy0 + ya, cy0 + yb
            status = line.get("status", "pass1")
            confidence = CONFIDENCE[status]
            if line.get("needs_review"):
                confidence = min(confidence, 0.75)
            elif line.get("verdict") == "neither":
                confidence = min(confidence, 0.85)
            text = printed_glyphs(line["text"])
            if region_id == "header":
                text = unpoint_running_head(text)
            region["lines"].append(
                {
                    "id": line_id,
                    "polygon": polygon(x0, y0, x1, y1),
                    "words": words_for(text, x0, x1, y0, y1, confidence, line_id),
                    "text": text,
                    "confidence": confidence,
                }
            )
            box = region["_box"]
            box[0], box[1], box[2], box[3] = min(box[0], x0), min(box[1], y0), max(box[2], x1), max(box[3], y1)
            statuses.append(
                {
                    "region_id": region_id,
                    "line_id": line_id,
                    "status": status,
                    "needs_review": bool(line.get("needs_review")),
                    **({"note": line["note"]} if line.get("note") else {}),
                }
            )
            # The pass 1 draft on the same geometry; lines pass 1 never
            # produced have no draft and are left out of the hypothesis.
            draft = line.get("draft", text) if pass2 else text
            draft = printed_glyphs(draft) if draft is not None else None
            if draft is not None and region_id == "header":
                draft = unpoint_running_head(draft)
            if draft is not None:
                draft_region["lines"].append(
                    {
                        "id": line_id,
                        "polygon": polygon(x0, y0, x1, y1),
                        "words": words_for(draft, x0, x1, y0, y1, 0.9, line_id),
                        "text": draft,
                        "confidence": 0.9,
                    }
                )

    def finish(rs: dict[str, dict]) -> list[dict]:
        out = []
        for key in sorted(rs, key=region_order):
            r = rs[key]
            box = r.pop("_box")
            if not r["lines"]:
                continue
            r["polygon"] = polygon(box[0], box[1], box[2], box[3])
            out.append({"id": r["id"], "polygon": r["polygon"], "lines": r["lines"]})
        return out

    page = {"width": img.width, "height": img.height, "regions": finish(regions)}
    out = {
        "schema": SCHEMA,
        "edition": record["edition"],
        "source_page": record["pdf_page"],
        "raster": record["raster"],
        "raster_sha256": record["raster_sha256"],
        "transcription": None,
        "pass": record.get("pass", 1),
        "engine": "claude-cli",
        "model": sorted({c.get("model") for c in record["chunks"] if c.get("model")} or {"unknown"})[0],
        "prompt_digest": hashlib.sha256(
            (record.get("prompt") or "").encode() + (record.get("read_prompt") or "").encode() + (record.get("judge_prompt") or "").encode()
        ).hexdigest(),
        "page": page,
        "line_status": statuses,
    }
    if pass2:
        # Pass 2 chunks carry no model field; pass 1 chunks do.
        p1 = json.load(open(record["pass1"], encoding="utf-8"))
        out["model"] = sorted({c.get("model") for c in p1["chunks"] if c.get("model")})[0]
        draft_regions_full = {
            region["id"]: {"id": region["id"], "polygon": region["polygon"], "lines": draft_regions[region["id"]]["lines"]}
            for region in page["regions"]
        }
        out["hypotheses"] = [
            {
                "label": "pass1",
                "page": {"width": img.width, "height": img.height, "regions": list(draft_regions_full.values())},
            }
        ]
    return out


def pass2_problem(record: dict) -> str | None:
    """Why a pass 2 record is not finished, or None. Records written before
    the status field existed are judged by their errored bands."""
    if record.get("pass") != 2:
        return None
    bands = sum(1 for c in record.get("chunks", []) for b in c.get("bands", []) if "error" in b.get("reread", {}))
    bands = max(bands, record.get("errors") or 0)
    judged = record.get("judge_errors") or 0
    if record.get("status") == "incomplete" or bands or judged:
        return f"pass 2 is incomplete ({bands} errored band(s), {judged} failed adjudication(s), status={record.get('status')!r})"
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--transcription", action="append", required=True, type=pathlib.Path)
    ap.add_argument("--output", required=True, type=pathlib.Path)
    ap.add_argument("--allow-incomplete", action="store_true", help="convert pass 2 records that have errored bands")
    args = ap.parse_args()
    for path in args.transcription:
        problem = pass2_problem(json.load(open(path, encoding="utf-8")))
        if problem and not args.allow_incomplete:
            print(f"refusing {path}: {problem}; rerun tool/frontier-verify.py or pass --allow-incomplete", file=sys.stderr)
            return 1
    args.output.mkdir(parents=True, exist_ok=True)
    for path in args.transcription:
        record = json.load(open(path, encoding="utf-8"))
        out = build(record)
        out["transcription"] = os.path.relpath(path)
        target = args.output / f"pdf-{record['pdf_page']:04d}.json"
        with open(target, "w", encoding="utf-8") as fh:
            json.dump(out, fh, ensure_ascii=False, indent=1)
            fh.write("\n")
        n = sum(len(r["lines"]) for r in out["page"]["regions"])
        print(f"{target}: {n} lines", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
