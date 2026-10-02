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
import difflib
import importlib.util
import json
import pathlib
import re
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


# --- Deterministic conventions (docs/ocr-metric-policy.md, "Gold conventions and provenance") ---

PTHAHA = "\u0730"  # SYRIAC PTHAHA ABOVE, the canonical form of Robinson's printed triangle
PTHAHA_VARIANTS = "\u25bd\u25bf\u2207\u0732"  # white down triangles, nabla, pthaha dotted
HEBREW_KEEP = {"\u05c1", "\u05c2"}  # shin and sin dot are printed even in running heads
SPACE_BEFORE = re.compile(r"[ \t]+(?=[,.;:!?)\u2019])")
YHWH = re.compile("(\u05d9[\u0590-\u05c7]*\u05d4)([\u0590-\u05c7]*)(\u05d5)([\u0590-\u05c7]*)(\u05d4)")
HOLAM = "\u05b9"


def canon_pthaha(text: str) -> str:
    """One encoding for the Syriac pthaha: U+0730; a printed triangle or the dotted form maps to it."""
    return "".join(PTHAHA if c in PTHAHA_VARIANTS else c for c in text)


def canon_yhwh(text: str) -> str:
    """One holam placement in the divine name: on the he (U+05D4 U+05B9), none on the vav."""

    def fix(m: re.Match) -> str:
        he1, marks1, vav, marks2, he2 = m.groups()
        if HOLAM in marks2:
            marks2 = marks2.replace(HOLAM, "")
            if HOLAM not in marks1:
                marks1 += HOLAM
        return he1 + marks1 + vav + marks2 + he2

    return YHWH.sub(fix, unicodedata.normalize("NFC", text))


def canon_spacing(text: str) -> str:
    """Print spacing: single spaces, none before , . ; : ! ? ) (the hair space is not transcribed),
    and 'e. g.' / 'i. e.' with the one narrow gap Robinson prints inside them."""
    text = re.sub(r"[ \t\u2009\u200a]+", " ", text).strip()
    text = SPACE_BEFORE.sub("", text)
    return re.sub(r"\b([eiEI])\.\s?([gGeE])\.", lambda m: f"{m.group(1)}. {m.group(2)}.", text)


def canon(text: str) -> str:
    """Gold text form of a reading: printed glyphs, pthaha, yhwh and spacing conventions (NFC)."""
    return unicodedata.normalize("NFC", canon_spacing(canon_yhwh(canon_pthaha(alto.printed_glyphs(text)))))


def is_wordchar(c: str) -> bool:
    return c.isalnum() or unicodedata.category(c)[0] == "M"


def space_fold(text: str) -> str:
    """Comparison only: drop whitespace unless it sits between two word characters, so
    'e. g.' / 'e.g.', '\u2018 a' / '\u2018a' and ', ' / ',' agree. Words still cannot merge."""
    text = re.sub(r"\s+", " ", text).strip()
    return "".join(
        c for i, c in enumerate(text)
        if c != " " or (0 < i < len(text) - 1 and is_wordchar(text[i - 1]) and is_wordchar(text[i + 1]))
    )


def cmpkey(text: str) -> str:
    """Key for 'exact agreement' between readers: canon, spacing folded, theta folded."""
    return space_fold(canon(text)).replace("\u03d1", "\u03b8")


def strip_head_points(text: str) -> str:
    """Running heads are printed unpointed; one rule shared with tool/frontier-to-alto.py."""
    return alto.unpoint_running_head(text)


def hebrew_tokens(text: str) -> list[str]:
    return [strip_marks(t) for t in text.split() if "HEBREW" in scripts(t)]


def hebrew_order_flag(a: str, b: str) -> str | None:
    """Flag when two readings hold the same Hebrew words in a different order (visual-order typing)."""
    ta, tb = hebrew_tokens(canon(a)), hebrew_tokens(canon(b))
    if len(ta) < 2 or ta == tb or sorted(ta) != sorted(tb):
        return None
    return "Hebrew word order reversed" if ta == tb[::-1] else "Hebrew word order differs"


def token_diff(a: str, b: str) -> list[dict]:
    """Whitespace tokens that differ between two readings (after the comparison key), for a later
    partial-line gold: [{'a': tokens of a, 'b': tokens of b, 'at': index in a}]."""
    ta, tb = canon(a).split(), canon(b).split()
    out = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, [space_fold(t) for t in ta], [space_fold(t) for t in tb], autojunk=False).get_opcodes():
        if tag != "equal":
            out.append({"at": i1, "a": ta[i1:i2], "b": tb[j1:j2]})
    return out


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
