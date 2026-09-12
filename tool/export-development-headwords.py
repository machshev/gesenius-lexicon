#!/usr/bin/env python3
"""Export reviewed development-partition headwords as an evaluation-only set.

`gesenius export-headword-training` deliberately skips the development and
final-test partitions, because neither may enter a fitting manifest. The
development pages are nevertheless reviewed, resolved and never used to select
a checkpoint, so they are the only reviewed material that can measure a
recognizer without the circularity of scoring on the selection set.

This writes the same `NAME.png` / `NAME.gt.txt` layout the exporter produces so
a `ketos test` command copied from an experiment report works unchanged. It
applies the exporter's own admission rules; anything it cannot admit is a hard
failure rather than a silent omission.

The output is evaluation material, not gold and not a fitting manifest. Nothing
here may enter training.
"""

import argparse
import hashlib
import json
import pathlib
import shutil
import sys
import tomllib
import unicodedata

BIDI = set("؜‎‏") | {chr(c) for c in range(0x202A, 0x202F)} | {
    chr(c) for c in range(0x2066, 0x206A)
}
TERMINAL_SKIP = {"not_headword", "excluded"}


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path,
                        default=pathlib.Path("benchmarks/transcription-drafts"))
    parser.add_argument("--journal", type=pathlib.Path,
                        default=pathlib.Path("corpus/review/transcription-reviews.jsonl"))
    parser.add_argument("--splits", type=pathlib.Path,
                        default=pathlib.Path("benchmarks/sample-inventory/robinson-1854-splits.toml"))
    parser.add_argument("--partition", default="development",
                        choices=["development"],
                        help="Final test stays untouched; it is not selectable here.")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    if args.output.exists():
        sys.exit(f"export destination already exists: {args.output}")

    splits_bytes = args.splits.read_bytes()
    splits = tomllib.loads(splits_bytes.decode())
    pages = set(splits[args.partition])

    reviews: dict[tuple[str, str], list[dict]] = {}
    for line in args.journal.read_text().splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        reviews.setdefault((record["sample"], record["line_id"]), []).append(record)

    selected = []
    crops: set[str] = set()
    for directory in sorted(args.root.iterdir()):
        manifest_path = directory / "review.json"
        draft_path = directory / "draft.json"
        if not manifest_path.is_file() or not draft_path.is_file():
            continue
        manifest_bytes = manifest_path.read_bytes()
        draft_bytes = draft_path.read_bytes()
        manifest = json.loads(manifest_bytes)
        draft = json.loads(draft_bytes)
        if manifest.get("kind") != "headword":
            continue
        if manifest.get("partition") != args.partition:
            continue
        if manifest["printed_page"] not in pages:
            sys.exit(f"{directory.name}: review partition disagrees with the splits manifest")
        if draft["source_sha256"] != splits["source_sha256"]:
            sys.exit(f"{directory.name}: source hash disagrees with the splits manifest")
        source_digest = digest_bytes(draft_bytes + manifest_bytes)

        by_id = {crop["line_id"]: crop for crop in manifest["lines"]}
        for gold in draft["lines"]:
            line_id = gold["line_id"]
            crop = by_id.get(line_id)
            if crop is None:
                sys.exit(f"{directory.name}/{line_id}: missing crop for draft line")
            candidates = [r for r in reviews.get((directory.name, line_id), [])
                          if r.get("source_digest") == source_digest]
            if not candidates:
                sys.exit(f"{directory.name}/{line_id}: no human review at the current source digest")
            review = candidates[-1]
            state = review.get("state")
            if state in TERMINAL_SKIP:
                continue
            if (state != "resolved" or not review.get("reviewer", "").strip()
                    or not review.get("text", "").strip() or review.get("revision", 0) == 0):
                sys.exit(f"{directory.name}/{line_id}: stale or unresolved review ({state})")
            if any(c in BIDI or (ord(c) < 0x20) for c in review["text"]):
                sys.exit(f"{directory.name}/{line_id}: review text contains control or bidi characters")
            crop_path = (directory / crop["crop"]).resolve()
            if not crop_path.is_relative_to(directory.resolve()) or crop_path.suffix != ".png":
                sys.exit(f"{directory.name}/{line_id}: crop must be a PNG within its sample directory")
            crop_bytes = crop_path.read_bytes()
            if digest_bytes(crop_bytes) != crop["crop_sha256"]:
                sys.exit(f"{directory.name}/{line_id}: crop hash mismatch")
            if crop["crop_sha256"] in crops:
                sys.exit(f"{directory.name}/{line_id}: duplicate crop hash")
            crops.add(crop["crop_sha256"])
            selected.append((directory, draft, manifest, gold, crop, review, crop_bytes))

    if not selected:
        sys.exit("no resolved development headword reviews")

    split = args.partition
    out = args.output
    (out / split).mkdir(parents=True)
    final_root = out.resolve()
    records = []
    alphabet: dict[str, int] = {}
    for index, (directory, draft, manifest, gold, crop, review, crop_bytes) in enumerate(selected):
        name = f"{split}/headword-{index:06}"
        nfc = unicodedata.normalize("NFC", review["text"])
        (out / f"{name}.png").write_bytes(crop_bytes)
        (out / f"{name}.gt.txt").write_text(f"{nfc}\n")
        for character in nfc:
            key = f"U+{ord(character):04X}"
            alphabet[key] = alphabet.get(key, 0) + 1
        records.append({
            "edition": draft["edition"],
            "printed_page": manifest["printed_page"],
            "source_page": draft["source_page"],
            "line_id": gold["line_id"],
            "split": split,
            "sample_kind": manifest["kind"],
            "image": str(final_root / f"{name}.png"),
            "ground_truth": str(final_root / f"{name}.gt.txt"),
            "source_span": f"{directory.name}#{gold['line_id']}",
            "source_sha256": draft["source_sha256"],
            "crop_sha256": crop["crop_sha256"],
            "ground_truth_sha256": digest_bytes(f"{nfc}\n".encode()),
            "diplomatic": review["text"],
            "nfc": nfc,
            "review": review,
            "split_sha256": digest_bytes(splits_bytes),
            "normalization": "NFC; logical order; no bidi controls",
            "authority": "reviewed development partition; evaluation only, never fitting",
        })

    (out / "ground-truth.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in records))
    (out / "alphabet-audit.json").write_text(
        json.dumps({split: dict(sorted(alphabet.items()))}, indent=2))
    (out / "page-splits.toml").write_bytes(splits_bytes)
    (out / f"{split}-paths.txt").write_text(
        "".join(f"{final_root / f'{split}/headword-{i:06}.png'}\n" for i in range(len(records))))
    print(json.dumps({
        "partition": split,
        "headwords": len(records),
        "characters": sum(len(r["nfc"]) for r in records),
        "pages": sorted({r["printed_page"] for r in records}, key=int),
        "output": str(final_root),
    }, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
