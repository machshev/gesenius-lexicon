#!/usr/bin/env python3
"""List every pass 2 record with its errored-band and draft_only counts (read-only).

Usage: tool/check-frontier-pass2.py [pass2-dir]
Exits 1 if any record is incomplete.
"""
import importlib.util
import json
import pathlib
import sys

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("frontier_to_alto", HERE / "frontier-to-alto.py")
alto = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(alto)

DEFAULT = HERE.parent / "corpus/frontier/robinson-1854/pass2"


def summarize(record: dict) -> dict:
    bands = [b for c in record.get("chunks", []) for b in c.get("bands", [])]
    return {
        "bands": len(bands),
        "errored": sum(1 for b in bands if "error" in b.get("reread", {})),
        "draft_only": sum(1 for ln in record.get("lines", []) if ln.get("status") == "draft_only"),
        "lines": len(record.get("lines", [])),
        "problem": alto.pass2_problem(record),
    }


def main() -> int:
    root = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT
    bad = 0
    print(f"{'record':<14}{'bands':>6}{'errored':>9}{'draft_only':>12}{'lines':>7}  state")
    for path in sorted(root.glob("pdf-*.json")):
        s = summarize(json.load(open(path, encoding="utf-8")))
        bad += bool(s["problem"])
        print(f"{path.name:<14}{s['bands']:>6}{s['errored']:>9}{s['draft_only']:>12}{s['lines']:>7}  {'INCOMPLETE' if s['problem'] else 'ok'}")
    print(f"{bad} incomplete record(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
