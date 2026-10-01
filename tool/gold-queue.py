#!/usr/bin/env python3
"""Review queue for one pass 2 record: per band and line, the readings, a diff class and a tier.

Usage:
    tool/gold-queue.py corpus/frontier/robinson-1854/pass2/pdf-0066.json [...] [--json]

Each band holds its lines with the pass 1 text, the pass 2 reread, the final
pass 2 text, status, adjudicator verdict and a diff class between pass 1 and
pass 2. Tier 1: status other than agreed, any script change, vav/yod-only or
order-only difference, Arabic/Syriac/Ethiopic text. Tier 2: agreed pointed
Hebrew. Tier 3: agreed Latin and Greek only. Records whose pass 2 is
incomplete are refused with the same check as tool/check-frontier-pass2.py.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import sys
import unicodedata

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent


def load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # type: ignore[union-attr]
    return module


alto = load("frontier_to_alto", "frontier-to-alto.py")
fv = load("frontier_verify", "frontier-verify.py")

FOREIGN = ("HEBREW", "ARABIC", "SYRIAC", "ETHIOPIC")
RTL_ODD = ("ARABIC", "SYRIAC", "ETHIOPIC")


def scripts(text: str) -> set[str]:
    return {unicodedata.name(c, "?").split()[0] for c in text if c.isalpha()}


def strip_marks(text: str) -> str:
    return "".join(c for c in unicodedata.normalize("NFD", text) if not unicodedata.combining(c))


def fold(text: str) -> str:
    """Comparison key: NFC, hair spaces dropped, the printed-glyph conversion,
    and cursive theta folded to theta (the model cannot be trusted on it)."""
    return fv.nfc(alto.printed_glyphs(text)).replace("ϑ", "θ")


def bare(text: str) -> str:
    """The text without punctuation, symbols and spaces; combining marks stay."""
    return "".join(c for c in text if unicodedata.category(c)[0] not in "PZS")


def classify(a: str, b: str) -> str:
    """Why two readings of a line differ; 'none' when they agree after folding."""
    a, b = fold(a), fold(b)
    if a == b:
        return "none"
    if bare(a) == bare(b):
        return "punctuation"
    if scripts(a) != scripts(b):
        return "script change"
    if sorted(a.split()) == sorted(b.split()):
        return "order only"
    if a.casefold() == b.casefold():
        return "case"
    sa, sb = strip_marks(a), strip_marks(b)
    if sa == sb:
        return "points only"
    if sa.replace("ו", "").replace("י", "") == sb.replace("ו", "").replace("י", ""):
        return "vav/yod only"
    return "letter change"


def tier(line: dict) -> int:
    kind = line["diff"]
    text = line["text"]
    if (
        line["status"] != "agreed"
        or kind in ("script change", "vav/yod only", "order only")
        or any(s in RTL_ODD for s in scripts(text))
    ):
        return 1
    return 2 if "HEBREW" in scripts(text) else 3


def check_complete(record: dict) -> None:
    if record.get("pass") != 2:
        raise SystemExit("not a pass 2 record")
    problem = alto.pass2_problem(record)
    if problem:
        raise SystemExit(f"refusing pdf {record.get('pdf_page')}: {problem}")


def build_queue(record: dict) -> list[dict]:
    """Bands with their lines, in the reading order of record['lines'].

    Lines that no band produced are returned under a band of None, which
    cannot be re-read and is never gold.
    """
    check_complete(record)
    items: list[dict] = []
    n = 0
    for chunk in record["chunks"]:
        groups = [(b, b.get("lines", [])) for b in chunk["bands"]] + [(None, chunk.get("unbanded_lines", []))]
        for band, lines in groups:
            if not lines:
                continue
            item = {"band": band, "chunk_id": chunk["chunk_id"], "column": chunk["column"], "lines": []}
            for line in lines:
                final = record["lines"][n]
                if fv.nfc(line["text"]) != final["text"]:
                    raise SystemExit(f"band lines disagree with the final lines at line {n}")
                entry = {
                    "index": n,
                    "pass1": line.get("draft"),
                    "reread": line.get("reread"),
                    "text": final["text"],
                    "status": final["status"],
                    "verdict": final.get("verdict"),
                    "note": final.get("note"),
                    "diff": classify(line["draft"], final["text"]) if line.get("draft") else "pass 1 absent",
                }
                entry["tier"] = tier(entry)
                item["lines"].append(entry)
                n += 1
            items.append(item)
    if n != len(record["lines"]):
        raise SystemExit(f"queue holds {n} lines, record has {len(record['lines'])}")
    return items


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("pass2", nargs="+", type=pathlib.Path)
    ap.add_argument("--json", action="store_true", help="print the full queue as JSON")
    args = ap.parse_args()
    for path in args.pass2:
        record = json.load(open(path, encoding="utf-8"))
        queue = build_queue(record)
        if args.json:
            print(json.dumps(queue, ensure_ascii=False, indent=1))
            continue
        tiers = [0, 0, 0, 0]
        for item in queue:
            for line in item["lines"]:
                tiers[line["tier"]] += 1
        print(f"{path.name}: {len(queue)} bands, lines tier 1/2/3 = {tiers[1]}/{tiers[2]}/{tiers[3]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
