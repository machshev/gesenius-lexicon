#!/usr/bin/env python3
"""Independent review of a pass 2 record for gold: blind read, reconcile, optional second reader.

Usage:
    tool/gold-review.py corpus/frontier/robinson-1854/pass2/pdf-0066.json [...] \
        [--output benchmarks/gold-review/robinson-1854] [--model opus] [--codex auto|off]

Per band (image from the 400 DPI raster, 3x when it holds Hebrew, Arabic,
Syriac or Ethiopic, else 2x):
 1. Blind read by the claude CLI: the band image only, never the earlier
    readings. It covers every band, so the evidence does not depend on pass 2
    status flags.
 2. Reconcile, only for lines where the blind read differs from pass 2 after
    folding: both readings and pass 1 are shown with the image; verdict
    pass2, blind, edited or contested, with the evidence.
 3. Second reader, only for contested items: a different model family via
    `codex exec` reads the band image with minimal instructions and no
    context. Contested means the blind read disagrees with pass 2, pass 1 and
    pass 2 agree but differ from an old fixture, or the reviewer is not
    certain. If codex is missing or fails, those lines stay contested.
A line is gold only when the readers converge; contested, unclear and
unreviewed lines are never gold. Results are written per page as a sidecar and
reused by image digest and prompt version, so reruns pay only for what is
missing. Records with incomplete pass 2 are refused.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

from PIL import Image

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("gold_queue", HERE / "gold-queue.py")
gq = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gq)  # type: ignore[union-attr]
fv, alto = gq.fv, gq.alto

PROMPT_VERSION = 1
DEFAULT_OUTPUT = HERE.parent / "benchmarks/gold-review/robinson-1854"
BLIND_EXTRA = (
    " Keep the printed form of each letter: a cursive theta printed as the "
    "looped form is written ϑ, a plain theta θ. For every line say how "
    "certain you are of every letter and mark, and set unclear when a vowel "
    "point, accent or glyph cannot be resolved or is not encoded in Unicode."
)
BLIND_SCHEMA = json.dumps(
    {
        "type": "object",
        "properties": {
            "lines": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "confidence": {"type": "string", "enum": ["certain", "likely", "uncertain"]},
                        "unclear": {"type": "boolean"},
                    },
                    "required": ["text", "confidence", "unclear"],
                },
            }
        },
        "required": ["lines"],
    }
)
RECONCILE_PROMPT = (
    "Read the image file {path}. It is an enlarged crop of a few printed lines from one "
    "column of an 1854 Hebrew-English lexicon (Gesenius, Robinson translation). One of "
    "the printed lines has been read three times and the readings differ. Compare each "
    "against the image character by character and decide what is printed. Check in "
    "particular the printed order of items in a list, whether a vav or yod is printed "
    "(plene or defective), every vowel point, dagesh, shin or sin dot and accent, whether "
    "a word is Arabic or Syriac, and whether Syriac points are printed. Take no reading on "
    "trust; if all are wrong, write what is printed (verdict edited). Use Unicode in "
    "logical order with no bidi control characters and keep the printed form of theta. "
    "Name the character-level evidence for each difference in the note. Use contested if "
    "the image does not settle it, and unclear if a mark or glyph cannot be resolved or has "
    "no Unicode encoding.\n"
    "Reading 1 (blind): {blind}\nReading 2 (earlier pass): {pass2}\nReading 3 (first pass): {pass1}"
)
RECONCILE_SCHEMA = json.dumps(
    {
        "type": "object",
        "properties": {
            "verdict": {"type": "string", "enum": ["pass2", "blind", "edited", "contested"]},
            "text": {"type": "string"},
            "note": {"type": "string"},
            "confidence": {"type": "string", "enum": ["certain", "likely", "uncertain"]},
            "unclear": {"type": "boolean"},
        },
        "required": ["verdict", "text", "note", "confidence", "unclear"],
    }
)
CODEX_PROMPT = (
    "Transcribe every complete printed text line in the attached image, top to bottom, exactly "
    "as printed, one string per line. Hebrew, Arabic, Syriac, Greek and Ethiopic as Unicode with "
    "every visible vowel point and diacritic, in logical order, no bidi control characters. "
    "Never insert, drop or normalise anything. Omit partly visible lines at the edges."
)
CODEX_SCHEMA = {
    "type": "object",
    "properties": {"lines": {"type": "array", "items": {"type": "string"}}},
    "required": ["lines"],
    "additionalProperties": False,
}


def run_codex(image: pathlib.Path, timeout: int) -> tuple[dict, dict]:
    """One codex reading of a band image: the image and minimal instructions only."""
    if not shutil.which("codex"):
        raise RuntimeError("codex is not installed")
    with tempfile.TemporaryDirectory() as tmp:
        schema, last = pathlib.Path(tmp, "schema.json"), pathlib.Path(tmp, "last.json")
        schema.write_text(json.dumps(CODEX_SCHEMA))
        cmd = [
            "codex", "exec", "--skip-git-repo-check", "--ephemeral", "--ignore-user-config", "--ignore-rules",
            "-s", "read-only", "-C", tmp, "--output-schema", str(schema), "-o", str(last), "-i", str(image.resolve()),
        ]
        proc = subprocess.run(cmd, input=CODEX_PROMPT, capture_output=True, text=True, timeout=timeout, check=False)
        if proc.returncode != 0:
            raise RuntimeError(f"codex exited {proc.returncode}: {(proc.stderr.strip() or proc.stdout.strip())[-400:]}")
        try:
            lines = [str(x) for x in json.loads(last.read_text())["lines"]]
        except (OSError, ValueError, KeyError, TypeError) as exc:
            raise RuntimeError(f"no codex lines: {exc}") from exc
    return {"lines": lines}, {"reader": "codex"}


def scale_for(texts: list[str]) -> int:
    return 3 if any(gq.scripts(t) & set(gq.FOREIGN) for t in texts) else 2


def band_image(raster: Image.Image, band: dict, scale: int, out_dir: pathlib.Path) -> tuple[pathlib.Path, str]:
    b = band["bounds"]
    path = out_dir / f"{band['band_id']}-{b['width']}x{b['height']}+{b['x']}+{b['y']}-x{scale}.png"
    if not path.exists():
        crop = raster.crop((b["x"], b["y"], b["x"] + b["width"], b["y"] + b["height"]))
        crop.resize((crop.width * scale, crop.height * scale), Image.LANCZOS).save(path, optimize=True)
    return path, fv.ft.sha256_file(path)


def old_fixture_lines(pdf_page: int) -> list[str]:
    for path in sorted((HERE.parent / "benchmarks/gold").glob("*.json")):
        fx = json.load(open(path, encoding="utf-8"))
        if fx.get("source_page") == pdf_page:
            return [gq.fold(ln["text"]) for ln in fx["lines"]]
    return []


def differs_from_fixture(line: dict, fixture: list[str]) -> bool:
    """Pass 1 and pass 2 agree but the nearest old fixture line says otherwise."""
    if not fixture or line["pass1"] is None or gq.fold(line["pass1"]) != gq.fold(line["text"]):
        return False
    key = gq.fold(line["text"])
    best = max(fixture, key=lambda f: fv.ratio(key, f))
    return fv.ratio(key, best) >= 0.8 and best != key


def converged(candidate: str, reading: str | None) -> bool:
    return reading is not None and gq.fold(candidate) == gq.fold(reading)


def decide(line: dict, codex: dict | None, codex_reading: str | None) -> tuple[str, str]:
    """(decision, reason) for one reviewed line. Gold needs converging readers."""
    if line.get("reason"):
        return "contested", line["reason"]
    if not line["triggers"]:
        return "gold", "blind read agrees with pass 2"
    if codex is None:
        return "contested", "second reader unavailable: " + ", ".join(line["triggers"])
    if "error" in codex:
        return "contested", "second reader failed: " + ", ".join(line["triggers"])
    if converged(line["candidate"], codex_reading):
        return "gold", "second reader converges: " + ", ".join(line["triggers"])
    return "contested", "second reader differs: " + ", ".join(line["triggers"])


def review_page(path: pathlib.Path, out_dir: pathlib.Path, args: argparse.Namespace) -> bool:
    record = json.load(open(path, encoding="utf-8"))
    queue = gq.build_queue(record)  # refuses incomplete records
    page_id = path.stem
    out_path = out_dir / f"{page_id}.json"
    cache: dict = {}
    if out_path.exists():
        try:
            old = json.load(open(out_path, encoding="utf-8"))
            if old.get("prompt_version") == PROMPT_VERSION and old.get("raster_sha256") == record["raster_sha256"]:
                cache = {k: v for k, v in old.get("cache", {}).items() if "error" not in v}
        except (OSError, ValueError):
            pass
    raster_path = pathlib.Path(record["raster"])
    raster = Image.open(raster_path)
    img_dir = raster_path.parent.parent / "chunks" / page_id / "gold"
    img_dir.mkdir(parents=True, exist_ok=True)
    fixture = old_fixture_lines(record["pdf_page"])
    use_codex = args.codex == "auto" and bool(shutil.which("codex"))
    if args.codex == "auto" and not use_codex:
        print("WARNING: codex is not installed; contested items get no second reader and stay contested", file=sys.stderr)

    def cached(key: str, fn):
        if key not in cache:
            try:
                cache[key] = fn()
            except Exception as exc:  # noqa: BLE001
                return {"error": str(exc)}
        return cache[key]

    bands = []
    for item in queue:
        band = item["band"]
        if band is None or (args.band and band["band_id"] not in args.band):
            bands.append({"item": item, "image": None})
            continue
        scale = scale_for([ln["text"] for ln in item["lines"]])
        image, digest = band_image(raster, band, scale, img_dir)
        bands.append({"item": item, "image": image, "digest": digest, "scale": scale, "id": band["band_id"]})
    todo = [b for b in bands if b["image"]]
    print(f"{page_id}: {len(queue)} bands, {len(todo)} selected for review", file=sys.stderr, flush=True)

    def blind(b: dict) -> dict:
        prompt = fv.READ_PROMPT.format(path=b["image"]) + BLIND_EXTRA
        return cached(f"blind:{b['digest']}", lambda: _claude(prompt, BLIND_SCHEMA))

    def _claude(prompt: str, schema: str) -> dict:
        structured, meta = fv.run_claude(prompt, schema, args.model, args.timeout)
        return {**structured, **meta}

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        for b, result in zip(todo, pool.map(blind, todo)):
            b["blind"] = result
            print(f"  {b['id']}: blind {'ERROR ' + result['error'] if 'error' in result else str(len(result['lines'])) + ' lines'}", file=sys.stderr, flush=True)

    # Align blind lines to the band's lines and find the lines to reconcile.
    jobs = []
    lines_out: list[dict] = []
    band_extra: dict[str, list[str]] = {}
    for b in bands:
        item = b["item"]
        for ln in item["lines"]:
            ln.update(band_id=b.get("id"), chunk_id=item["chunk_id"], column=item["column"], triggers=[], blind=None, blind_confidence=None)
            lines_out.append(ln)
            if not b["image"]:
                ln["reason"] = "not reviewed" if item["band"] else "no band image"
        if not b["image"]:
            continue
        if "error" in b["blind"]:
            for ln in item["lines"]:
                ln["reason"] = "blind read failed: " + b["blind"]["error"]
            continue
        blind_lines = b["blind"]["lines"]
        pairs = fv.align([ln["text"] for ln in item["lines"]], [x["text"] for x in blind_lines])
        for di, rj in pairs:
            if di is None:
                band_extra.setdefault(b["id"], []).append(blind_lines[rj]["text"])
                continue
            ln = item["lines"][di]
            ln["candidate"] = ln["text"]
            if rj is None:
                ln["reason"] = "no blind counterpart"
                continue
            ln["blind"], ln["blind_confidence"] = blind_lines[rj]["text"], blind_lines[rj]["confidence"]
            if blind_lines[rj]["unclear"]:
                ln["reason"] = "blind reader cannot resolve a mark or glyph"
            elif gq.fold(ln["blind"]) != gq.fold(ln["text"]):
                ln["triggers"].append("blind read differs from pass 2")
                jobs.append((b, ln))
            elif fv.nfc(alto.printed_glyphs(ln["blind"])) != fv.nfc(alto.printed_glyphs(ln["text"])):
                ln["reason"] = f"theta form unresolved: blind {ln['blind']!r}, pass 2 {ln['text']!r}"
                ln["glyph"] = "theta"
            elif ln["blind_confidence"] == "uncertain":
                ln["triggers"].append(f"blind confidence {ln['blind_confidence']}")
            if differs_from_fixture(ln, fixture):
                ln["triggers"].append("pass 1 and pass 2 agree but differ from old fixture")

    def reconcile(job: tuple[dict, dict]) -> dict:
        b, ln = job
        prompt = RECONCILE_PROMPT.format(path=b["image"], blind=ln["blind"], pass2=ln["text"], pass1=ln["pass1"])
        key = f"reconcile:{b['digest']}:{gq.fold(ln['blind'])}:{gq.fold(ln['text'])}:{gq.fold(ln['pass1'] or '')}"
        return cached(key, lambda: _claude(prompt, RECONCILE_SCHEMA))

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        for (b, ln), r in zip(jobs, pool.map(reconcile, jobs)):
            ln["reconcile"] = r
            if "error" in r:
                ln["reason"] = "reconcile failed: " + r["error"]
            elif r["unclear"] or r["verdict"] == "contested":
                ln["reason"] = f"reviewer verdict {r['verdict']}" + (", unclear" if r["unclear"] else "")
            else:
                ln["candidate"] = {"pass2": ln["text"], "blind": ln["blind"]}.get(r["verdict"], r["text"])
                if r["confidence"] == "uncertain":
                    ln["triggers"].append(f"reviewer confidence {r['confidence']}")

    # Second reader, only for bands holding a contested item that can still become gold.
    need = [b for b in todo if any(ln["triggers"] and not ln.get("reason") for ln in b["item"]["lines"])]
    codex_by_band: dict[str, dict] = {}
    if use_codex:
        def second(b: dict) -> dict:
            return cached(f"codex:{b['digest']}", lambda: run_codex(b["image"], args.timeout)[0])

        with concurrent.futures.ThreadPoolExecutor(max_workers=min(args.workers, 3)) as pool:
            for b, r in zip(need, pool.map(second, need)):
                codex_by_band[b["id"]] = r
                print(f"  {b['id']}: codex {'ERROR ' + r['error'] if 'error' in r else str(len(r['lines'])) + ' lines'}", file=sys.stderr, flush=True)
    for b in todo:
        codex = codex_by_band.get(b["id"])
        reading = {}
        if codex and "error" not in codex:
            for di, rj in fv.align([ln.get("candidate", ln["text"]) for ln in b["item"]["lines"]], codex["lines"]):
                if di is not None and rj is not None:
                    reading[di] = codex["lines"][rj]
        for k, ln in enumerate(b["item"]["lines"]):
            ln["codex"] = reading.get(k)
            ln["decision"], ln["decision_reason"] = decide(ln, codex, reading.get(k)) if "blind" in b else ("contested", ln.get("reason"))
    for ln in lines_out:
        if "decision" not in ln:
            ln["decision"], ln["decision_reason"] = "contested", ln.get("reason") or "not reviewed"
        ln["text_gold"] = alto.printed_glyphs(ln["candidate"]) if ln["decision"] == "gold" else None

    errors = sum(1 for b in todo if "error" in b["blind"]) + sum(1 for ln in lines_out if "error" in ln.get("reconcile", {}))
    unreviewed = sum(1 for ln in lines_out if ln.get("reason") in ("not reviewed", "no band image"))
    agree: dict[str, list[int]] = {}
    for ln in lines_out:
        if ln["blind"] is not None:
            names = sorted(gq.scripts(ln["text"]) & set(gq.FOREIGN)) or ["LATIN/GREEK"]
            for n in names:
                a = agree.setdefault(n, [0, 0])
                a[0] += 1
                a[1] += gq.fold(ln["blind"]) == gq.fold(ln["text"])
    out = {
        "schema": "gesenius-gold-review/1",
        "edition": record["edition"],
        "pdf_page": record["pdf_page"],
        "pass2": os.path.relpath(path),
        "pass2_generated_at": record.get("generated_at"),
        "raster": record["raster"],
        "raster_sha256": record["raster_sha256"],
        "raster_size": record["raster_size"],
        "prompt_version": PROMPT_VERSION,
        "reviewer_model": args.model,
        "second_reader": "codex" if use_codex else None,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "status": "complete" if not errors and not unreviewed else "incomplete",
        "errors": errors,
        "unreviewed": unreviewed,
        "counts": {k: sum(1 for ln in lines_out if ln["decision"] == k) for k in ("gold", "contested")},
        "blind_agreement_by_script": {k: {"lines": v[0], "agreed": v[1]} for k, v in sorted(agree.items())},
        "blind_extra_lines": band_extra,
        "bands": [
            {"band_id": b["id"], "image_path": os.path.relpath(b["image"]), "image_sha256": b["digest"], "scale": b["scale"],
             "blind": b["blind"], "codex": codex_by_band.get(b["id"])}
            for b in todo
        ],
        "lines": lines_out,
        "cache": cache,
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_suffix(".json.tmp")
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1)
        fh.write("\n")
    os.replace(tmp, out_path)
    print(f"{page_id}: {out['counts']}, status {out['status']}, agreement {out['blind_agreement_by_script']}", file=sys.stderr, flush=True)
    return out["status"] == "complete"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("pass2", nargs="+", type=pathlib.Path)
    ap.add_argument("--output", type=pathlib.Path, default=DEFAULT_OUTPUT)
    ap.add_argument("--model", default="opus")
    ap.add_argument("--codex", choices=["auto", "off"], default="auto")
    ap.add_argument("--band", action="append", help="review only this band id (smoke tests); the rest stay unreviewed")
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--timeout", type=int, default=600)
    args = ap.parse_args()
    bad = [p.stem for p in args.pass2 if not review_page(p, args.output, args)]
    if bad:
        print(f"INCOMPLETE review for {', '.join(bad)}; rerun to retry only what is missing", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
