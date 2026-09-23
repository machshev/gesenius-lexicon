#!/usr/bin/env python3
"""Pass 2 of the frontier transcription stage: enlarged re-reading and adjudication.

Every pass 1 chunk is cut into short bands of a few printed lines at blank
rows between line cores. Each band is cropped from the page raster, enlarged
2x and read again by the model with the same diplomatic prompt but without
sight of the pass 1 draft, so the second reading is independent. The band
readings are aligned to the pass 1 lines of the chunk by text similarity.
Lines that agree are accepted. Lines that differ are sent back a third time
with both readings and the enlarged band so the model can decide character
by character against the image and say why. Lines only one pass produced are
kept and flagged for a reviewer.

Usage:
    tool/frontier-verify.py --transcription corpus/frontier/robinson-1854/pdf-0066.json \
        [--transcription ...] --output corpus/frontier/robinson-1854/pass2 [--workers 6]

The pass 2 record keeps every band's geometry, image digest, both readings,
the adjudication and usage, plus the final reading-order lines with their
status, so the scorer and the entry segmenter can consume it like a pass 1
record. Band results are reused by image digest and prompt version.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import difflib
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import unicodedata

import numpy as np
from PIL import Image

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("frontier_transcribe", HERE / "frontier-transcribe.py")
ft = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ft)  # type: ignore[union-attr]

PROMPT_VERSION = 1
READ_PROMPT = (
    "Read the image file {path}. It is an enlarged crop of a few printed lines from "
    "one column of an 1854 Hebrew-English lexicon (Gesenius, Robinson translation). "
    "Transcribe every complete printed text line exactly, one array element per "
    "printed line, top to bottom. Preserve spelling, capitalisation, punctuation, "
    "printed abbreviations, section marks and end-of-line hyphenation. Hebrew, "
    "Aramaic, Arabic, Syriac, Greek and Ethiopic must be Unicode text with every "
    "visible vowel point, dagesh, shin or sin dot, accent and diacritic, and with "
    "no point that is not printed; write them in logical order with no bidi control "
    "characters, keep the words of a right-to-left phrase in the order they are "
    "read, and keep the printed left-to-right order of separate items in a list. "
    "Transcribe the letters actually printed even when they are not the dictionary "
    "spelling: never insert or drop a vav or yod, and never replace a form with a "
    "more familiar one. Distinguish Arabic from Syriac by the letter shapes. If a "
    "line is only partially visible at the top or bottom edge, omit it entirely "
    "rather than guessing. Never add, reorder, translate, normalise or omit "
    "anything that is fully visible. If the crop contains no text, return an "
    "empty array."
)
JUDGE_PROMPT = (
    "Read the image file {path}. It is an enlarged crop of a few printed lines from "
    "one column of an 1854 Hebrew-English lexicon (Gesenius, Robinson translation). "
    "Two independent transcriptions of the same printed line disagree. Compare each "
    "of them character by character against the printed line in the image and "
    "return the exact printed text as a single line. Check in particular: the "
    "printed left-to-right order of separate items in a list, whether a vav or yod "
    "is actually printed (plene versus defective spelling), every vowel point, "
    "dagesh, shin or sin dot and accent, whether a word is Arabic or Syriac, and "
    "whether Syriac points are printed at all. Take neither reading on trust; if "
    "both are wrong, write what is printed. Use Unicode in logical order with no "
    "bidi control characters. Explain the decisive difference in one short note.\n"
    "Line {index} of {count} in the crop, counting only complete lines.\n"
    "Reading A: {a}\n"
    "Reading B: {b}"
)
READ_SCHEMA = json.dumps(
    {
        "type": "object",
        "properties": {"lines": {"type": "array", "items": {"type": "string"}}},
        "required": ["lines"],
    }
)
JUDGE_SCHEMA = json.dumps(
    {
        "type": "object",
        "properties": {
            "text": {"type": "string"},
            "verdict": {"type": "string", "enum": ["a", "b", "neither"]},
            "note": {"type": "string"},
        },
        "required": ["text", "verdict", "note"],
    }
)


PUNCT_BEFORE = re.compile(r"\s+([;:!?,.)\]])")
PUNCT_AFTER = re.compile(r"([(\[])\s+")


def nfc(text: str) -> str:
    """Comparison and output form: NFC, single spaces, no thin space before
    closing punctuation. The 1854 typesetting puts a hair space before ; : ! ?
    and often between a number and its comma; the model reproduces it as a
    space, which carries no information in Unicode text."""
    text = unicodedata.normalize("NFC", " ".join(text.split()))
    text = PUNCT_BEFORE.sub(r"\1", text)
    return PUNCT_AFTER.sub(r"\1", text)


def ratio(a: str, b: str) -> float:
    return difflib.SequenceMatcher(None, a, b, autojunk=False).ratio()


def plan_bands(img: Image.Image, per_band: int, core: float, min_core: int) -> list[tuple[int, int]]:
    """Cut a chunk image into [y0, y1) bands of about per_band printed lines.

    Line cores are rows dense enough to hold letter bodies; points and
    descenders between cores are sparse. Cuts fall on the blankest row
    between two cores so no glyph or point is severed.
    """
    ink = ft.ink_mask(img)
    row = ink.mean(axis=1)
    cores = ft.runs(row > core, min_core)
    if not cores:
        return []
    text = np.flatnonzero(row > 0.002)
    top, bottom = int(text[0]), int(text[-1]) + 1
    cuts = [top]
    for i in range(per_band, len(cores), per_band):
        a1 = cores[i - 1][1]
        b0 = cores[i][0]
        if b0 <= a1:
            continue
        cuts.append(a1 + int(np.argmin(row[a1:b0])))
    cuts.append(bottom)
    return [(y0, y1) for y0, y1 in zip(cuts, cuts[1:]) if y1 > y0]


def run_claude(prompt: str, schema: str, model: str, timeout: int) -> tuple[dict, dict]:
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
        schema,
        prompt,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
    if proc.returncode != 0:
        raise RuntimeError(f"claude exited {proc.returncode}: {proc.stderr.strip()[:500]}")
    result = json.loads(proc.stdout)
    structured = result.get("structured_output")
    if not isinstance(structured, dict):
        raise RuntimeError(f"no structured output: {str(result.get('result'))[:300]}")
    usage = result.get("usage", {})
    meta = {
        "model": model,
        "session_id": result.get("session_id"),
        "duration_api_ms": result.get("duration_api_ms"),
        "cost_usd_list": result.get("total_cost_usd"),
        "input_tokens": usage.get("input_tokens"),
        "cache_read_input_tokens": usage.get("cache_read_input_tokens"),
        "cache_creation_input_tokens": usage.get("cache_creation_input_tokens"),
        "output_tokens": usage.get("output_tokens"),
    }
    return structured, meta


def read_band(path: pathlib.Path, model: str, timeout: int) -> dict:
    structured, meta = run_claude(READ_PROMPT.format(path=path), READ_SCHEMA, model, timeout)
    if "lines" not in structured:
        raise RuntimeError("no lines in structured output")
    return {"lines": [str(x) for x in structured["lines"]], **meta}


def judge_line(path: pathlib.Path, index: int, count: int, a: str, b: str, model: str, timeout: int) -> dict:
    prompt = JUDGE_PROMPT.format(path=path, index=index, count=count, a=a, b=b)
    structured, meta = run_claude(prompt, JUDGE_SCHEMA, model, timeout)
    return {
        "text": str(structured.get("text", "")),
        "verdict": structured.get("verdict"),
        "note": str(structured.get("note", "")),
        **meta,
    }


def align(draft: list[str], reread: list[str]) -> list[tuple[int | None, int | None]]:
    """Pair draft and re-read line indices; None marks a line only one pass produced."""
    key_d = [nfc(x) for x in draft]
    key_r = [nfc(x) for x in reread]
    pairs: list[tuple[int | None, int | None]] = []
    sm = difflib.SequenceMatcher(None, key_d, key_r, autojunk=False)
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            pairs.extend((i, j) for i, j in zip(range(i1, i2), range(j1, j2)))
        else:
            # Pair differing lines greedily by similarity inside the block.
            di = list(range(i1, i2))
            rj = list(range(j1, j2))
            while di and rj:
                best = max(((ratio(key_d[i], key_r[j]), i, j) for i in di for j in rj), key=lambda t: t[0])
                if best[0] < 0.5:
                    break
                pairs.append((best[1], best[2]))
                di.remove(best[1])
                rj.remove(best[2])
            pairs.extend((i, None) for i in di)
            pairs.extend((None, j) for j in rj)
    pairs.sort(key=lambda p: (p[0] if p[0] is not None else -1, p[1] if p[1] is not None else -1))
    return pairs


def verify_page(
    transcription: pathlib.Path,
    output_dir: pathlib.Path,
    model: str,
    workers: int,
    per_band: int,
    scale: int,
    pad: int,
    timeout: int,
) -> pathlib.Path:
    page = json.load(open(transcription, encoding="utf-8"))
    page_id = transcription.stem
    out_path = output_dir / f"{page_id}.json"
    previous_reads: dict[tuple[str, int], dict] = {}
    previous_judgements: dict[tuple[str, int, str, str], dict] = {}
    if out_path.exists():
        try:
            old = json.load(open(out_path, encoding="utf-8"))
            for chunk in old.get("chunks", []):
                for band in chunk.get("bands", []):
                    if "reread" in band and "error" not in band["reread"]:
                        previous_reads[(band["image_sha256"], band["prompt_version"])] = band["reread"]
                    for line in band.get("lines", []):
                        j = line.get("judgement")
                        if j and "error" not in j:
                            previous_judgements[(band["image_sha256"], band["prompt_version"], line["draft"], line["reread"])] = j
        except (OSError, ValueError, KeyError):
            pass

    raster_path = pathlib.Path(page["raster"])
    raster = Image.open(raster_path)
    band_dir = raster_path.parent.parent / "chunks" / page_id / "pass2"
    band_dir.mkdir(parents=True, exist_ok=True)

    chunks_out = []
    read_jobs: list[dict] = []
    for chunk in page["chunks"]:
        draft = chunk.get("lines", [])
        record = {
            "chunk_id": chunk["chunk_id"],
            "column": chunk["column"],
            "row": chunk["row"],
            "bounds": chunk["bounds"],
            "draft_lines": draft,
            "bands": [],
        }
        chunks_out.append(record)
        if not draft:
            continue
        cb = chunk["bounds"]
        chunk_img = raster.crop((cb["x"], cb["y"], cb["x"] + cb["width"], cb["y"] + cb["height"]))
        for bi, (y0, y1) in enumerate(plan_bands(chunk_img, per_band, 0.02, 12)):
            by0 = max(0, cb["y"] + y0 - pad)
            by1 = min(raster.height, cb["y"] + y1 + pad)
            bounds = {"x": cb["x"], "y": by0, "width": cb["width"], "height": by1 - by0}
            path = band_dir / f"{chunk['chunk_id']}-b{bi:02d}-{bounds['width']}x{bounds['height']}+{bounds['x']}+{by0}-x{scale}.png"
            if not path.exists():
                crop = raster.crop((bounds["x"], by0, bounds["x"] + bounds["width"], by1))
                crop.resize((crop.width * scale, crop.height * scale), Image.LANCZOS).save(path, optimize=True)
            band = {
                "band_id": f"{chunk['chunk_id']}-b{bi:02d}",
                "bounds": bounds,
                "scale": scale,
                "image_path": os.path.relpath(path),
                "image_sha256": ft.sha256_file(path),
                "prompt_version": PROMPT_VERSION,
            }
            record["bands"].append(band)
            key = (band["image_sha256"], PROMPT_VERSION)
            if key in previous_reads:
                band["reread"] = previous_reads[key]
            else:
                read_jobs.append(band)

    print(f"{page_id}: {sum(len(c['bands']) for c in chunks_out)} bands, {len(read_jobs)} to read", file=sys.stderr, flush=True)

    def do_read(band: dict) -> tuple[dict, dict]:
        try:
            return band, read_band(pathlib.Path(band["image_path"]), model, timeout)
        except Exception as exc:  # noqa: BLE001
            return band, {"error": str(exc)}

    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        for band, reread in pool.map(do_read, read_jobs):
            band["reread"] = reread
            status = f"ERROR {reread['error']}" if "error" in reread else f"{len(reread['lines'])} lines, {reread['duration_api_ms']} ms"
            print(f"  {page_id} {band['band_id']}: {status}", file=sys.stderr, flush=True)

    # Align the concatenated band re-readings of each chunk with its draft.
    judge_jobs: list[tuple[dict, dict]] = []
    for record in chunks_out:
        draft = record["draft_lines"]
        if not draft:
            continue
        reread_lines: list[tuple[dict, str]] = []
        for band in record["bands"]:
            if "error" in band["reread"]:
                continue
            reread_lines.extend((band, line) for line in band["reread"]["lines"])
        pairs = align(draft, [text for _, text in reread_lines])
        for di, rj in pairs:
            band = reread_lines[rj][0] if rj is not None else None
            line = {
                "draft": draft[di] if di is not None else None,
                "reread": reread_lines[rj][1] if rj is not None else None,
            }
            if di is not None and rj is not None:
                if nfc(line["draft"]) == nfc(line["reread"]):
                    line["status"] = "agreed"
                    line["text"] = line["draft"]
                else:
                    line["status"] = "disagreed"
                    line["band_id"] = band["band_id"]
                    key = (band["image_sha256"], PROMPT_VERSION, line["draft"], line["reread"])
                    if key in previous_judgements:
                        line["judgement"] = previous_judgements[key]
                    else:
                        judge_jobs.append((band, line))
            elif di is not None:
                line["status"] = "draft_only"
                line["text"] = line["draft"]
            else:
                line["status"] = "reread_only"
                line["band_id"] = band["band_id"]
                line["text"] = line["reread"]
            # Attach to the band that produced the re-reading, else the first band.
            target = band if band is not None else (record["bands"][0] if record["bands"] else None)
            if target is None:
                record.setdefault("unbanded_lines", []).append(line)
            else:
                target.setdefault("lines", []).append(line)

    print(f"{page_id}: {len(judge_jobs)} disagreements to adjudicate", file=sys.stderr, flush=True)

    def do_judge(job: tuple[dict, dict]) -> tuple[dict, dict]:
        band, line = job
        try:
            count = len(band["reread"]["lines"])
            index = band["reread"]["lines"].index(line["reread"]) + 1
            return line, judge_line(pathlib.Path(band["image_path"]), index, count, line["draft"], line["reread"], model, timeout)
        except Exception as exc:  # noqa: BLE001
            return line, {"error": str(exc)}

    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        for line, judgement in pool.map(do_judge, judge_jobs):
            line["judgement"] = judgement
            if "error" in judgement:
                print(f"  judge ERROR {judgement['error']}", file=sys.stderr, flush=True)
            else:
                print(f"  judge {judgement['verdict']}: {judgement['note'][:100]}", file=sys.stderr, flush=True)

    # Final lines in reading order.
    final_lines = []
    counts = {"agreed": 0, "adjudicated": 0, "draft_only": 0, "reread_only": 0, "unresolved": 0}
    total_cost = 0.0
    for record in chunks_out:
        seq = []
        for band in record["bands"]:
            total_cost += (band.get("reread") or {}).get("cost_usd_list") or 0
            seq.extend(band.get("lines", []))
        seq.extend(record.get("unbanded_lines", []))
        for line in seq:
            status = line["status"]
            if status == "disagreed":
                j = line.get("judgement", {})
                total_cost += j.get("cost_usd_list") or 0
                if j and "error" not in j and j.get("text"):
                    line["text"] = j["text"]
                    status = "adjudicated"
                else:
                    line["text"] = line["draft"]
                    status = "unresolved"
                line["status"] = status
            counts[status] += 1
            line["text"] = nfc(line["text"])
            entry = {"chunk_id": record["chunk_id"], "column": record["column"], "text": line["text"], "status": status}
            if status == "adjudicated":
                entry["draft"] = line["draft"]
                entry["reread"] = line["reread"]
                entry["verdict"] = line["judgement"]["verdict"]
                entry["note"] = line["judgement"]["note"]
            elif status in ("draft_only", "reread_only", "unresolved"):
                entry["draft"] = line["draft"]
                entry["reread"] = line["reread"]
            final_lines.append(entry)

    out = {
        "schema": "gesenius-frontier-transcription/1",
        "edition": page["edition"],
        "pdf_page": page["pdf_page"],
        "raster": page["raster"],
        "raster_sha256": page["raster_sha256"],
        "raster_size": page["raster_size"],
        "pass": 2,
        "pass1": os.path.relpath(transcription),
        "method": "claude-cli-independent-reread-and-adjudication",
        "prompt_version": PROMPT_VERSION,
        "read_prompt": READ_PROMPT,
        "judge_prompt": JUDGE_PROMPT,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "chunks": chunks_out,
        "lines": final_lines,
        "counts": counts,
        "errors": sum(1 for c in chunks_out for b in c["bands"] if "error" in b.get("reread", {})),
        "cost_usd_list_total": round(total_cost, 4),
    }
    output_dir.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_suffix(".json.tmp")
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1)
        fh.write("\n")
    os.replace(tmp, out_path)
    print(f"{page_id}: {counts}, list cost ${total_cost:.2f}", file=sys.stderr, flush=True)
    return out_path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--transcription", action="append", required=True, type=pathlib.Path)
    ap.add_argument("--output", required=True, type=pathlib.Path)
    ap.add_argument("--model", default="claude-fable-5-1")
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--lines-per-band", type=int, default=4)
    ap.add_argument("--scale", type=int, default=2)
    ap.add_argument("--pad", type=int, default=8)
    ap.add_argument("--timeout", type=int, default=600)
    args = ap.parse_args()
    for transcription in args.transcription:
        print(
            verify_page(
                transcription,
                args.output,
                args.model,
                args.workers,
                args.lines_per_band,
                args.scale,
                args.pad,
                args.timeout,
            )
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
