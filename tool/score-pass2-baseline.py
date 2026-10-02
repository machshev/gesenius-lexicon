#!/usr/bin/env python3
"""Score existing pass 2 frontier ALTO records against whole-page gold.

Runs `gesenius benchmark` per page (no transcription is re-run) and prints
per-page and per-script CER/WER as markdown. Held-out pages 266 and 791 are
refused. Usage: tool/score-pass2-baseline.py [pdf ...]
"""
import json, subprocess, sys, tempfile
from pathlib import Path
from xml.sax.saxutils import quoteattr

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/debug/gesenius"
DEFAULT = [17, 66, 116, 341, 716, 191, 491, 641, 941, 1091]
HELD_OUT = {266, 791}

def poly_box(p):
    xs = [q["x"] for q in p]; ys = [q["y"] for q in p]
    return min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys)

def attrs(p):
    x, y, w, h = poly_box(p)
    return f'HPOS="{x}" VPOS="{y}" WIDTH="{w}" HEIGHT="{h}"'

def to_alto(page):
    o = ['<?xml version="1.0" encoding="UTF-8"?><alto xmlns="http://www.loc.gov/standards/alto/ns-v4#"><Layout>'
         f'<Page WIDTH="{page["width"]}" HEIGHT="{page["height"]}"><PrintSpace>']
    for r in page["regions"]:
        o.append(f'<TextBlock ID={quoteattr(r["id"])} {attrs(r["polygon"])}>')
        for l in r["lines"]:
            o.append(f'<TextLine ID={quoteattr(l["id"])} {attrs(l["polygon"])}>')
            ws = l["words"] or [{"id": l["id"] + "-w", "polygon": l["polygon"], "text": l["text"], "confidence": l["confidence"]}]
            for i, w in enumerate(ws):
                if i: o.append("<SP/>")
                o.append(f'<String ID={quoteattr(w["id"])} {attrs(w["polygon"])} CONTENT={quoteattr(w["text"])} WC="{w["confidence"]}"/>')
            o.append("</TextLine>")
        o.append("</TextBlock>")
    o.append("</PrintSpace></Page></Layout></alto>")
    return "".join(o)

def score(pdf, tmp):
    if pdf in HELD_OUT:
        sys.exit(f"refusing held-out page {pdf}")
    gold = ROOT / f"benchmarks/gold/robinson-1854-pdf{pdf:04d}-full.json"
    rec = json.loads((ROOT / f"corpus/frontier/robinson-1854/alto/pdf-{pdf:04d}.json").read_text())
    assert rec["pass"] == 2 and rec["source_page"] == pdf
    g = json.loads(gold.read_text())
    assert g["source_image"]["coordinate_frame"].endswith(rec["raster_sha256"])
    alto = tmp / f"{pdf}.alto.xml"; alto.write_text(to_alto(rec["page"]))
    ident = tmp / f"{pdf}.id.json"
    ident.write_text(json.dumps({"edition": g["edition"], "source_page": g["source_page"],
        "source_sha256": g["source_sha256"], "coordinate_frame": g["source_image"]["coordinate_frame"]}))
    out = subprocess.run([str(BIN), "benchmark", "--gold", str(gold), "--alto", str(alto),
        "--hypothesis-identity", str(ident)], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return len(g["lines"]), json.loads(out)

def main():
    pages = [int(a) for a in sys.argv[1:]] or DEFAULT
    results = {}
    with tempfile.TemporaryDirectory() as t:
        for p in pages:
            results[p] = score(p, Path(t))
    print(report(results))

def report(results):
    scripts = ["Latn", "Hebr", "Grek", "Arab", "Syrc", "Zyyy"]
    L = ["Per page (exact Unicode CER/WER, aligned by source coordinates):", "",
         "| pdf | gold lines | missing | ref chars | ref words | CER | WER | NFC CER |", "|---|---|---|---|---|---|---|---|"]
    tot = {"lines": 0, "miss": 0, "chars": 0, "words": 0, "ce": 0.0, "we": 0.0}
    sc = {s: dict(ref=0, err=0, sub=0, dele=0, ins=0) for s in scripts}
    fw = {s: [0, 0] for s in scripts}
    for pdf, (n, r) in results.items():
        m = r["metrics"]
        miss = len(r["missing_lines"])
        L.append(f"| {pdf} | {n} | {miss} | {m['reference_characters']} | {m['reference_words']} | "
                 f"{m['cer']:.4f} | {m['wer']:.4f} | {m['canonical_equivalence']['cer']:.4f} |")
        tot["lines"] += n; tot["miss"] += miss; tot["chars"] += m["reference_characters"]
        tot["words"] += m["reference_words"]; tot["ce"] += m["cer"] * m["reference_characters"]
        tot["we"] += m["wer"] * m["reference_words"]
        for s, c in m["aligned"]["characters_by_script"].items():
            d = sc.setdefault(s, dict(ref=0, err=0, sub=0, dele=0, ins=0))
            d["ref"] += c["reference_characters"]; d["sub"] += c["substitutions"]
            d["dele"] += c["deletions"]; d["ins"] += c["insertions"]
            d["err"] += c["substitutions"] + c["deletions"] + c["insertions"]
        for s, c in m["aligned"]["foreign_words"]["by_script"].items():
            f = fw.setdefault(s, [0, 0]); f[0] += c["reference_words"]; f[1] += c["exact_matches"]
    L.append(f"| **total** | {tot['lines']} | {tot['miss']} | {tot['chars']} | {tot['words']} | "
             f"{tot['ce']/tot['chars']:.4f} | {tot['we']/tot['words']:.4f} | |")
    L += ["", "Per script, totals over all pages (aligned character counts; CER = (sub+del+ins)/ref chars; "
          "Latn includes English and transliteration; Zyyy is digits/punctuation/space; token accuracy is exact "
          "foreign-containing-token accuracy, not defined for Latn):", "",
          "| script | ref chars | sub | del | ins | CER | ref tokens | token acc |", "|---|---|---|---|---|---|---|---|"]
    for s, d in sc.items():
        if not d["ref"] and not d["err"]: continue
        cer = f"{d['err']/d['ref']:.4f}" if d["ref"] else "n/a"
        f = fw.get(s, [0, 0])
        acc = f"{f[1]/f[0]:.4f}" if f[0] else "n/a"
        L.append(f"| {s} | {d['ref']} | {d['sub']} | {d['dele']} | {d['ins']} | {cer} | {f[0] or ''} | {acc} |")
    L += ["", "Per page per script CER (aligned; ref chars in brackets):", "",
          "| pdf | " + " | ".join(s for s in scripts if s != "Zyyy") + " |", "|---|" + "---|" * (len(scripts) - 1)]
    for pdf, (n, r) in results.items():
        cells = []
        for s in scripts:
            if s == "Zyyy": continue
            c = r["metrics"]["aligned"]["characters_by_script"].get(s)
            cells.append("-" if not c else f"{(c['substitutions']+c['deletions']+c['insertions'])/c['reference_characters']:.4f} [{c['reference_characters']}]" if c["reference_characters"] else f"ins {c['insertions']}")
        L.append(f"| {pdf} | " + " | ".join(cells) + " |")
    return "\n".join(L)

if __name__ == "__main__":
    main()
