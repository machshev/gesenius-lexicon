#!/usr/bin/env python3
"""Score a Kraken recognizer against a reviewed real headword set.

`ketos test` reports one aggregate character accuracy. That number is
comparable across this project's experiments and is reported here unchanged,
but it hides the distinction the acceptance target is actually written in: a
headword is accepted only when every letter AND every point is right, and the
residual errors in this project are overwhelmingly point errors on correct
letters.

So this also emits predictions in the shape `gesenius benchmark-headwords`
consumes, which reports base-aligned mark precision and recall and separates
exact accuracy from consonant-only accuracy.

Input is an export directory produced by `gesenius export-headword-training`
or `tool/export-development-headwords.py`: it reads `ground-truth.jsonl`, so
every scored pair keeps its crop hash and review provenance.
"""

import argparse
import json
import pathlib
import subprocess
import sys
import unicodedata


def load_pairs(export: pathlib.Path, splits: set[str] | None):
    pairs = []
    for line in (export / "ground-truth.jsonl").read_text().splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        if splits and record["split"] not in splits:
            continue
        pairs.append(record)
    if not pairs:
        sys.exit(f"no pairs in {export} for splits {splits}")
    return pairs


def predict(model: pathlib.Path, pairs, device: str):
    """Recognize each crop as a single line covering the whole image."""
    from kraken.lib import models
    from kraken import rpred
    from kraken.containers import Segmentation, BBoxLine
    from PIL import Image

    network = models.load_any(str(model), device=device)
    out = []
    for index, record in enumerate(pairs):
        image = Image.open(record["image"])
        width, height = image.size
        bounds = Segmentation(
            type="bbox", imagename=record["image"], text_direction="horizontal-rl",
            script_detection=False, lines=[BBoxLine(id=f"line-{index}", bbox=(0, 0, width, height))],
        )
        text = "".join(r.prediction for r in rpred.rpred(network, image, bounds, pad=16,
                                                        bidi_reordering=False))
        out.append(unicodedata.normalize("NFC", text))
        if (index + 1) % 25 == 0:
            print(f"  recognized {index + 1}/{len(pairs)}", file=sys.stderr)
    return out


def levenshtein(a: str, b: str) -> int:
    previous = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        current = [i]
        for j, cb in enumerate(b, 1):
            current.append(min(previous[j] + 1, current[j - 1] + 1,
                               previous[j - 1] + (ca != cb)))
        previous = current
    return previous[-1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=pathlib.Path, required=True)
    parser.add_argument("--export", type=pathlib.Path, required=True)
    parser.add_argument("--split", action="append", default=None,
                        help="restrict to a split in the export; repeatable")
    parser.add_argument("--label", default=None, help="name for this run in the report")
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--output", type=pathlib.Path, default=None)
    args = parser.parse_args()

    pairs = load_pairs(args.export, set(args.split) if args.split else None)
    predictions = predict(args.model, pairs, args.device)

    errors = characters = exact = consonant_exact = 0
    per_page: dict[str, list[int]] = {}
    for record, hypothesis in zip(pairs, predictions):
        reference = unicodedata.normalize("NFC", record["nfc"])
        distance = levenshtein(reference, hypothesis)
        errors += distance
        characters += len(reference)
        hit = reference == hypothesis
        exact += hit
        strip = lambda s: "".join(c for c in unicodedata.normalize("NFD", s)
                                  if not unicodedata.combining(c))
        consonant_exact += strip(reference) == strip(hypothesis)
        page = per_page.setdefault(record["printed_page"], [0, 0])
        page[0] += hit
        page[1] += 1

    report = {
        "model": str(args.model),
        "export": str(args.export),
        "splits": sorted({r["split"] for r in pairs}),
        "label": args.label,
        "pairs": len(pairs),
        "characters": characters,
        "errors": errors,
        "character_accuracy": round(100.0 * (1 - errors / characters), 2),
        "exact_pointed": f"{exact}/{len(pairs)}",
        "exact_pointed_pct": round(100.0 * exact / len(pairs), 2),
        "consonant_exact": f"{consonant_exact}/{len(pairs)}",
        "consonant_exact_pct": round(100.0 * consonant_exact / len(pairs), 2),
        "pages": {p: f"{v[0]}/{v[1]}" for p, v in sorted(per_page.items(), key=lambda kv: int(kv[0]))},
    }
    print(json.dumps(report, indent=2, ensure_ascii=False))

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        detail = [{"source_span": r["source_span"], "printed_page": r["printed_page"],
                   "crop_sha256": r["crop_sha256"], "reference": r["nfc"],
                   "hypothesis": h, "exact": r["nfc"] == h}
                  for r, h in zip(pairs, predictions)]
        args.output.write_text(json.dumps({"report": report, "predictions": detail},
                                          indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
