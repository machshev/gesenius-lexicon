#!/usr/bin/env python3
"""Score a frontier page transcription against a gold fixture.

Gold fixtures list printed lines in order; the frontier record lists model
lines in column reading order. Each gold line is matched to the page line
with the highest similarity ratio; unmatched gold lines count as missing.
Reports exact-line matches, character edits over matched lines, and prints
every mismatch so the error classes can be inspected.

Usage:
    tool/score-frontier-transcription.py --gold benchmarks/gold/X.json \
        --transcription corpus/frontier/robinson-1854/pdf-0017.json
"""

from __future__ import annotations

import argparse
import difflib
import json
import unicodedata


def nfc(text: str) -> str:
    return unicodedata.normalize("NFC", " ".join(text.split()))


def edits(a: str, b: str) -> int:
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    return sum(max(i2 - i1, j2 - j1) for tag, i1, i2, j1, j2 in sm.get_opcodes() if tag != "equal")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--gold", required=True)
    ap.add_argument("--transcription", required=True)
    ap.add_argument("--min-ratio", type=float, default=0.5)
    args = ap.parse_args()

    gold = [nfc(line["text"]) for line in json.load(open(args.gold, encoding="utf-8"))["lines"]]
    page = json.load(open(args.transcription, encoding="utf-8"))
    hyp = [nfc(line["text"]) for line in page["lines"]]

    used: set[int] = set()
    total_chars = total_edits = exact = missing = 0
    for g in gold:
        best_i, best_r = -1, 0.0
        for i, h in enumerate(hyp):
            if i in used:
                continue
            r = difflib.SequenceMatcher(None, g, h, autojunk=False).ratio()
            if r > best_r:
                best_i, best_r = i, r
        total_chars += len(g)
        if best_i < 0 or best_r < args.min_ratio:
            missing += 1
            total_edits += len(g)
            print(f"MISSING gold: {g}")
            continue
        used.add(best_i)
        e = edits(g, hyp[best_i])
        total_edits += e
        if e == 0:
            exact += 1
        else:
            print(f"{e:3} gold: {g}\n    hyp:  {hyp[best_i]}")
    print(
        f"gold lines {len(gold)}  exact {exact}  missing {missing}  "
        f"char_acc {1 - total_edits / max(total_chars, 1):.4f}  edits {total_edits}/{total_chars}  "
        f"hyp lines {len(hyp)}  page errors {page.get('errors')}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
