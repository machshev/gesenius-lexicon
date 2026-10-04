#!/usr/bin/env python3
"""Batch driver for full-book frontier transcription of the Robinson 1854 lexicon.

Per page, sequentially (concurrent runs hit the usage quota):
  rasterize (pdftoppm, 400 dpi) -> pass 1 (frontier-transcribe) -> pass 2
  (frontier-verify) -> check for errored bands -> one rerun of errored or
  incomplete work -> ALTO (frontier-to-alto).
Then, for the whole batch: import-frontier, validate, export jsonl/tei/sqlite,
and a batch report plus a spot-check sample list under docs/batches/.

Usage:
    tool/run-batch.py --batch 1 --next 50        # plan the first 50 undone content pages
    tool/run-batch.py --batch 2 --pages 100-130,140
    tool/run-batch.py --batch 1                  # resume: reuses the page list in docs/batches/batch-001.json
    tool/run-batch.py --status                   # content range, done and remaining counts

The default prompt version (2) is always used; the driver never passes
--prompt-version.

Idempotent: pages already present in corpus/machine are skipped, and pass 1 and
pass 2 results are cached by image digest, so a rerun only pays for missing work.
Records marked incomplete are rerun on resume.

Exit codes: 0 batch finished with no failed page, 1 some pages failed (the rest
were imported), 75 usage or rate limit stop (state saved; no import was done;
run the same command again later), 2 usage error.

Book layout (pdf pages, one-based; printed page = pdf page - 16):
  1-16      front matter (title, preface, addenda; pdf 5, 9, 15 already done)
  17-1171   the lexicon proper, LEXICON. to the end of Tav (printed 1-1155)
  1172-1176 appendix of anomalous verb forms (printed 1156-1160)
  1177-1186 blank leaves, deacidification notice, scan trailer
Content pages are 17-1176 (1160 pages). pdf 266 and 791 are the held-out final
test: they are in the corpus already and are never reviewed or scored.
"""
import argparse
import datetime as dt
import json
import os
import pathlib
import random
import re
import subprocess
import sys
import time

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parent.parent
EDITION = "robinson-1854"
CONTENT_FIRST, LEXICON_LAST, CONTENT_LAST = 17, 1171, 1176
HELD_OUT = {266, 791}
BIN = ROOT / "target/debug/gesenius"
FRONTIER = ROOT / "corpus/frontier" / EDITION
RASTER_DIR = ROOT / ".cache/gesenius/frontier" / EDITION / "raster"
CORPUS_FILE = ROOT / "corpus/machine" / f"{EDITION}.jsonl"
BATCH_DIR = ROOT / "docs/batches"
LOG_DIR = ROOT / ".cache/gesenius/batches"
EXIT_QUOTA = 75
QUOTA = re.compile(r"usage limit|quota|rate.?limit|limit reached|too many requests|overloaded|claude exited|hit your limit|resets? at", re.I)
HEB, ARA, SYR = (0x0590, 0x05FF), (0x0600, 0x06FF), (0x0700, 0x074F)


def sh(cmd, log, **kw):
    """Run a command, appending its output to the log; return (rc, stdout+stderr)."""
    log.write(f"\n$ {' '.join(str(c) for c in cmd)}\n")
    proc = subprocess.run([str(c) for c in cmd], cwd=ROOT, capture_output=True, text=True, **kw)
    out = proc.stdout + proc.stderr
    log.write(out)
    log.flush()
    return proc.returncode, out


def parse_pages(spec):
    pages = []
    for part in spec.split(","):
        a, _, b = part.strip().partition("-")
        pages += range(int(a), int(b or a) + 1)
    return sorted(set(pages))


def source_pdf():
    import tomllib
    sha = next(s["sha256"] for s in tomllib.load(open(ROOT / "sources.toml", "rb"))["sources"] if s["edition"] == EDITION)
    return ROOT / ".cache/gesenius/sources" / sha / "source.pdf"


def corpus_pages():
    """PDF pages that already have entries in the machine corpus."""
    pages = set()
    if CORPUS_FILE.exists():
        for line in open(CORPUS_FILE, encoding="utf-8"):
            for sp in entry_spans(json.loads(line)):
                for c in sp.get("coordinates", []):
                    pages.add(c["source_page"])
    return pages


def entry_spans(entry):
    return [sp for b in entry.get("blocks", []) for sp in b.get("spans", [])]


def entry_count():
    return sum(1 for _ in open(CORPUS_FILE, encoding="utf-8")) if CORPUS_FILE.exists() else 0


def pass2_path(p):
    return FRONTIER / "pass2" / f"pdf-{p:04d}.json"


def pass1_path(p):
    return FRONTIER / f"pdf-{p:04d}.json"


def alto_path(p):
    return FRONTIER / "alto" / f"pdf-{p:04d}.json"


def load(path):
    try:
        return json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError):
        return None


def pass1_errors(p):
    rec = load(pass1_path(p))
    if rec is None:
        return ["no pass 1 record"]
    return [c["error"] for c in rec.get("chunks", []) if "error" in c]


def pass2_state(p):
    """(complete, errored band count, error texts) of a pass 2 record."""
    rec = load(pass2_path(p))
    if rec is None:
        return False, 0, ["no pass 2 record"]
    texts = []
    nbands = 0
    for c in rec.get("chunks", []):
        for b in c.get("bands", []):
            if "error" in b.get("reread", {}):
                nbands += 1
                texts.append(b["reread"]["error"])
            for ln in b.get("lines", []):
                if "error" in ln.get("judgement", {}):
                    texts.append(ln["judgement"]["error"])
    complete = rec.get("status") == "complete" and not texts and not rec.get("errors") and not rec.get("judge_errors")
    return complete, nbands, texts


def looks_like_quota(texts):
    return any(QUOTA.search(t or "") for t in texts)


class QuotaStop(Exception):
    pass


class PageFailed(Exception):
    pass


def ensure_raster(p, log):
    out = RASTER_DIR / f"pdf-{p:04d}.png"
    if out.exists():
        return out
    RASTER_DIR.mkdir(parents=True, exist_ok=True)
    rc, o = sh(["pdftoppm", "-f", p, "-l", p, "-r", 400, "-png", "-singlefile", source_pdf(), RASTER_DIR / f"pdf-{p:04d}"], log)
    if rc or not out.exists():
        raise PageFailed(f"rasterize failed: {o[-200:]}")
    return out


def run_page(p, log):
    """Run one page to ALTO. Returns the per-page report dict; raises QuotaStop or PageFailed."""
    rep = {"page": p, "bands_rerun": [], "pass1_rerun": False, "pass2_rerun": False}
    t0 = time.time()
    raster = ensure_raster(p, log)
    if alto_path(p).exists() and pass2_state(p)[0] and not pass1_errors(p):
        rep["note"] = "resumed: complete records and ALTO already present"
        rep["seconds"] = 0.0
        return rep

    # Pass 1 (cached by chunk digest; failed chunks are retried on rerun).
    t1 = [sys.executable, "tool/frontier-transcribe.py", "--edition", EDITION, "--raster", raster.relative_to(ROOT), "--output", FRONTIER.relative_to(ROOT)]
    for attempt in range(2):
        sh(t1, log)
        errs = pass1_errors(p)
        if not errs:
            break
        if looks_like_quota(errs):
            raise QuotaStop(f"pass 1 page {p}: {errs[0][:200]}")
        rep["pass1_rerun"] = True
    else:
        raise PageFailed(f"pass 1 left {len(errs)} errored chunk(s): {errs[0][:200]}")

    # Pass 2, then the check, with at most one rerun of errored or incomplete bands.
    t2 = [sys.executable, "tool/frontier-verify.py", "--transcription", pass1_path(p).relative_to(ROOT), "--output", (FRONTIER / "pass2").relative_to(ROOT)]
    for attempt in range(2):
        rc, out = sh(t2, log)
        complete, nbad, texts = pass2_state(p)
        chk_rc, _ = sh([sys.executable, "tool/check-frontier-pass2.py"], log)  # lists errored bands in the log
        if complete and rc == 0:
            break
        if looks_like_quota(texts) or "usage or rate limit" in out:
            raise QuotaStop(f"pass 2 page {p}: {(texts or [out[-200:]])[0][:200]}")
        rep["pass2_rerun"] = True
        rep["bands_rerun"].append({"attempt": attempt + 1, "errored_bands": nbad, "failed_judgements": len(texts) - nbad})
        if attempt == 1:
            raise PageFailed(f"pass 2 still incomplete after one rerun: {(texts or ['?'])[0][:200]}")
    # A first-attempt failure that the rerun repaired is still recorded above.

    rc, out = sh([sys.executable, "tool/frontier-to-alto.py", "--transcription", pass2_path(p).relative_to(ROOT), "--output", (FRONTIER / "alto").relative_to(ROOT)], log)
    if rc or not alto_path(p).exists():
        raise PageFailed(f"ALTO conversion failed: {out[-300:]}")
    rep["seconds"] = round(time.time() - t0, 1)
    return rep


def script_of(text):
    counts = {"he": 0, "ar": 0, "syr": 0}
    for ch in text:
        o = ord(ch)
        for k, (a, b) in (("he", HEB), ("ar", ARA), ("syr", SYR)):
            if a <= o <= b:
                counts[k] += 1
    k = max(counts, key=counts.get)
    return k if counts[k] else None


def make_samples(pages, n=30, seed=1):
    """About n final pass 2 lines from the batch, weighted toward Hebrew, Arabic and Syriac."""
    pool = {"he": [], "ar": [], "syr": []}
    for p in pages:
        if p in HELD_OUT:
            continue
        rec = load(pass2_path(p))
        if not rec:
            continue
        for c in rec["chunks"]:
            for b in c["bands"]:
                for i, ln in enumerate(b.get("lines", [])):
                    s = script_of(ln.get("text", ""))
                    if s:
                        pool[s].append({
                            "pdf_page": p, "script": s, "text": ln["text"], "status": ln.get("status"),
                            "band_id": b["band_id"], "band_line": i + 1,
                            "band_image": b.get("image_path"), "band_bbox_page_px": b["bounds"],
                            "page_raster": rec["raster"],
                        })
    rng = random.Random(seed)
    # Prefer lines the model had to adjudicate or flagged; rare scripts are taken in full up to their quota.
    quota = {"syr": 8, "ar": 8, "he": n - 16}
    out = []
    for s in ("syr", "ar", "he"):
        pool[s].sort(key=lambda r: (r["status"] == "agreed", rng.random()))
        take = pool[s][: quota[s]]
        out += take
    short = n - len(out)
    if short > 0:
        rest = [r for r in pool["he"] if r not in out]
        out += rest[:short]
    return sorted(out, key=lambda r: (r["pdf_page"], r["band_id"], r["band_line"]))


def segmentation_flags(pages):
    """Per-page entry counts from the corpus, flagging empty pages and oversized entries."""
    starts, lines_per_entry = {}, {}
    for line in open(CORPUS_FILE, encoding="utf-8"):
        e = json.loads(line)
        pg = {c["source_page"] for sp in entry_spans(e) for c in sp.get("coordinates", [])}
        ids = {c.get("line_id") for sp in entry_spans(e) for c in sp.get("coordinates", [])}
        for q in pg:
            starts[q] = starts.get(q, 0) + 1
        lines_per_entry[e["id"]] = (len(ids), sorted(pg), e.get("headword"))
    flags = []
    for p in pages:
        if starts.get(p, 0) == 0:
            flags.append({"page": p, "flag": "no entries touch this page"})
    for eid, (n, pg, hw) in lines_per_entry.items():
        if n > 60 and any(q in pages for q in pg):
            flags.append({"entry": eid, "flag": f"huge entry: {n} lines over pdf pages {pg}"})
    return flags, {p: starts.get(p, 0) for p in pages}


def write_report(rep, path_json):
    BATCH_DIR.mkdir(parents=True, exist_ok=True)
    tmp = path_json.with_suffix(".json.tmp")
    json.dump(rep, open(tmp, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    os.replace(tmp, path_json)


def write_markdown(rep, path_md):
    done = [r for r in rep["pages_report"] if r.get("status") == "done"]
    failed = [r for r in rep["pages_report"] if r.get("status") == "failed"]
    secs = [r["seconds"] for r in done if r.get("seconds")]
    L = [f"# Batch {rep['batch']:03d}", "", f"Status: {rep['status']}", f"Pages: {rep['pages'][0]}-{rep['pages'][-1]} ({len(rep['pages'])} planned)", ""]
    L += [f"- Done: {len(done)}", f"- Failed: {len(failed)}"]
    for r in failed:
        L.append(f"  - pdf {r['page']}: {r['reason']}")
    if secs:
        L.append(f"- Wall time per page: mean {sum(secs) / len(secs):.0f} s, max {max(secs):.0f} s (total {sum(secs) / 60:.0f} min over pages run this session)")
    imp = rep.get("import")
    if imp:
        L += [f"- Entries: {imp['entries_before']} -> {imp['entries_after']} (delta {imp['entries_after'] - imp['entries_before']})",
              f"- Validate: {imp['validate_errors']} errors, {imp['validate_warnings']} warnings",
              f"- Import: {imp.get('summary', '')}"]
    rerun = [r for r in rep["pages_report"] if r.get("bands_rerun") or r.get("pass1_rerun")]
    L += ["", "## Reruns"]
    L += [f"- pdf {r['page']}: pass1 rerun={r['pass1_rerun']}, pass2 reruns={r['bands_rerun']}" for r in rerun] or ["- none"]
    L += ["", "## Segmentation flags"]
    L += [f"- {f.get('page') or f.get('entry')}: {f['flag']}" for f in rep.get("segmentation_flags", [])] or ["- none"]
    if rep.get("samples_file"):
        L += ["", f"Spot-check sample lines: `{rep['samples_file']}`"]
    open(path_md, "w", encoding="utf-8").write("\n".join(L) + "\n")


def status_cmd():
    done = corpus_pages()
    content = set(range(CONTENT_FIRST, CONTENT_LAST + 1))
    left = sorted(content - done)
    print(f"content pages: pdf {CONTENT_FIRST}-{CONTENT_LAST} ({len(content)}); lexicon proper {CONTENT_FIRST}-{LEXICON_LAST}, appendix {LEXICON_LAST + 1}-{CONTENT_LAST}")
    print(f"done in corpus: {len(content & done)}; left: {len(left)}; next: {left[:10]}")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--batch", type=int)
    ap.add_argument("--pages", help="pdf pages, e.g. 19-30,32-65")
    ap.add_argument("--next", type=int, help="take the first N content pages not yet in the corpus")
    ap.add_argument("--status", action="store_true")
    ap.add_argument("--no-import", action="store_true", help="stop after ALTO (no import, validate or export)")
    args = ap.parse_args()
    if args.status:
        return status_cmd()
    if args.batch is None:
        ap.error("--batch is required")
    name = f"batch-{args.batch:03d}"
    jpath, mpath = BATCH_DIR / f"{name}.json", BATCH_DIR / f"{name}.md"
    prev = load(jpath)
    done_before = corpus_pages()
    if args.pages:
        pages = parse_pages(args.pages)
    elif args.next:
        pages = [p for p in range(CONTENT_FIRST, CONTENT_LAST + 1) if p not in done_before and p not in HELD_OUT][: args.next]
    elif prev:
        pages = prev["pages"]
    else:
        ap.error("give --pages or --next (or resume an existing batch)")
    if prev and prev["pages"] != pages:
        print(f"warning: {name} already planned for different pages; replacing the plan", file=sys.stderr)
    bad = [p for p in pages if p in HELD_OUT or not CONTENT_FIRST <= p <= CONTENT_LAST]
    if bad:
        ap.error(f"pages outside the content range or held out: {bad}")
    if not BIN.exists():
        print("build first: cargo build", file=sys.stderr)
        return 2

    LOG_DIR.mkdir(parents=True, exist_ok=True)
    log = open(LOG_DIR / f"{name}.log", "a", encoding="utf-8")
    log.write(f"\n=== {dt.datetime.now().isoformat(timespec='seconds')} start {name} pages {pages[0]}-{pages[-1]}\n")
    todo = [p for p in pages if p not in done_before]
    # Carry forward earlier per-page results on resume.
    results = {r["page"]: r for r in (prev or {}).get("pages_report", []) if r.get("status") == "done"} if prev else {}
    rep = {"batch": args.batch, "pages": pages, "status": "running", "started": dt.datetime.now().isoformat(timespec="seconds"),
           "skipped_already_in_corpus": [p for p in pages if p in done_before], "pages_report": []}
    quota_stop = None
    for p in todo:
        if p in results and alto_path(p).exists():
            continue
        print(f"[{name}] pdf {p} ...", flush=True)
        t0 = time.time()
        try:
            r = run_page(p, log)
            r["status"] = "done"
            r["seconds"] = r.get("seconds") or round(time.time() - t0, 1)
            print(f"[{name}] pdf {p} done in {r['seconds']:.0f} s", flush=True)
        except QuotaStop as e:
            quota_stop = f"{e}"
            r = {"page": p, "status": "quota_stopped", "reason": quota_stop, "seconds": round(time.time() - t0, 1)}
            results[p] = r
            print(f"[{name}] QUOTA STOP at pdf {p}: {quota_stop}", file=sys.stderr, flush=True)
            break
        except PageFailed as e:
            r = {"page": p, "status": "failed", "reason": str(e), "seconds": round(time.time() - t0, 1)}
            print(f"[{name}] pdf {p} FAILED: {e}", file=sys.stderr, flush=True)
        results[p] = r
        rep["pages_report"] = [results[q] for q in sorted(results)]
        write_report(rep, jpath)
    rep["pages_report"] = [results[q] for q in sorted(results)]

    if quota_stop:
        rep["status"] = "stopped_quota"
        rep["stop_reason"] = quota_stop
        write_report(rep, jpath)
        write_markdown({**rep, "pages": pages}, mpath)
        print(f"stopped on usage limit; resume with: tool/run-batch.py --batch {args.batch}", file=sys.stderr)
        return EXIT_QUOTA

    ok = sorted(p for p in pages if p in done_before or results.get(p, {}).get("status") == "done")
    failed = [results[p] for p in pages if results.get(p, {}).get("status") == "failed"]
    new_ok = [p for p in ok if p not in done_before]
    if args.no_import or not new_ok:
        rep["status"] = "alto_only" if new_ok else "nothing_to_do"
        write_report(rep, jpath)
        write_markdown({**rep, "pages": pages}, mpath)
        return 1 if failed else 0

    # Import the new pages plus any already-imported neighbour so entries carry across page breaks.
    neighbours = sorted({q for p in new_ok for q in (p - 1, p + 1) if q in done_before and alto_path(q).exists()})
    import_pages = sorted(set(new_ok) | set(neighbours))
    entries_before = entry_count()
    rc, out = sh([BIN, "import-frontier", "--edition", EDITION, *[alto_path(p).relative_to(ROOT) for p in import_pages]], log)
    if rc:
        print(f"import failed: {out[-500:]}", file=sys.stderr)
        rep["status"] = "import_failed"
        write_report(rep, jpath)
        return 1
    after = entry_count()
    vrc, vout = sh([BIN, "validate", "--json"], log)
    verrs = vwarn = None
    try:
        v = json.loads(vout[vout.index("{"):vout.rindex("}") + 1])
        verrs = v.get("errors", v.get("error_count"))
        vwarn = v.get("warnings", v.get("warning_count"))
        verrs = len(verrs) if isinstance(verrs, list) else verrs
        vwarn = len(vwarn) if isinstance(vwarn, list) else vwarn
    except ValueError:
        m = re.search(r"(\d+) errors?, (\d+) warnings?", vout)
        if m:
            verrs, vwarn = int(m.group(1)), int(m.group(2))
    exports = {}
    for fmt in ("jsonl", "tei", "sqlite"):
        erc, _ = sh([BIN, "export", "--format", fmt, "--output", f"artifacts/{fmt}"], log)
        exports[fmt] = "ok" if erc == 0 else f"failed ({erc})"
    flags, per_page = segmentation_flags(new_ok)
    samples = make_samples(new_ok)
    sfile = BATCH_DIR / f"{name}-samples.json"
    json.dump({"batch": args.batch, "note": "bbox is x,y,width,height in the 400 dpi page raster; band_image is the 2x enlarged crop read in pass 2", "lines": samples},
              open(sfile, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    rep.update({
        "status": "complete" if not failed else "complete_with_failures",
        "finished": dt.datetime.now().isoformat(timespec="seconds"),
        "import": {"pages_imported": import_pages, "entries_before": entries_before, "entries_after": after,
                   "validate_exit": vrc, "validate_errors": verrs, "validate_warnings": vwarn,
                   "summary": next((l for l in reversed(out.strip().splitlines()) if re.search(r"[A-Za-z0-9]", l)), ""), "exports": exports},
        "entries_per_page": per_page, "segmentation_flags": flags,
        "samples_file": str(sfile.relative_to(ROOT)), "sample_count": len(samples),
    })
    write_report(rep, jpath)
    write_markdown({**rep, "pages": pages}, mpath)
    print(json.dumps({k: rep[k] for k in ("status", "import")}, indent=1))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
