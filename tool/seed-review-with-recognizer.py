#!/usr/bin/env python3
"""Replace machine draft suggestions with a trained recognizer's reading.

Reviewers currently start from the Tesseract pass, which scores 30.49%
character accuracy and reads 1 of 24 development headwords exactly. The
fine-tuned Kraken recognizer reaches 89.63% and 15 of 24. Seeding the queue
from the better engine turns most of the work from transcription into
confirmation.

Tesseract's reading is not discarded: it stays in the line's `hypotheses` beside
the recognizer's, so a reviewer can compare two independent engines. The draft
text becomes the recognizer's reading because it is pointed. Tesseract
frequently emits an unpointed consonant skeleton, and correcting wrong marks is
much less work than entering every mark by hand.

SAFETY. The review digest is sha256(draft.json + review.json), so rewriting
either file invalidates every review recorded against that sample. This tool
therefore refuses to touch any sample that has even one review at its current
digest, and reports what it skipped. Re-seeding reviewed work is not a supported
operation; re-crop it through the review service instead.

Nothing here is gold. The output is a machine suggestion awaiting human source
review, exactly as the Tesseract suggestion it replaces.
"""

import argparse
import collections
import hashlib
import json
import pathlib
import sys
import unicodedata

BIDI = {chr(c) for c in range(0x202A, 0x202F)} | {chr(c) for c in range(0x2066, 0x206A)} | set("؜‎‏")


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_recognizer(model: pathlib.Path, device: str):
    from kraken.train.vgsl import VGSLRecognitionModel, VGSLRecognitionTrainingConfig
    from kraken.lib import models as kmodels
    module = VGSLRecognitionModel.load_from_weights(str(model), VGSLRecognitionTrainingConfig())
    return kmodels.TorchSeqRecognizer(module.net, train=False, device=device)


def recognize(network, path: pathlib.Path, pad: int) -> str:
    from kraken import rpred
    from kraken.containers import Segmentation, BBoxLine
    from PIL import Image
    image = Image.open(path)
    width, height = image.size
    bounds = Segmentation(type="bbox", imagename=str(path), text_direction="horizontal-rl",
                          script_detection=False,
                          lines=[BBoxLine(id="line", bbox=(0, 0, width, height))])
    # bidi_reordering returns logical order, which is how reviews are stored.
    # These models train with `ketos train --reorder`, so the network emits
    # display order and must be reordered back.
    text = "".join(r.prediction for r in rpred.rpred(network, image, bounds, pad=pad,
                                                     bidi_reordering=True))
    text = "".join(c for c in text if c not in BIDI and not (ord(c) < 0x20))
    return unicodedata.normalize("NFC", text)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--model", type=pathlib.Path, required=True)
    parser.add_argument("--root", type=pathlib.Path,
                        default=pathlib.Path("benchmarks/transcription-drafts"))
    parser.add_argument("--journal", type=pathlib.Path,
                        default=pathlib.Path("corpus/review/transcription-reviews.jsonl"))
    parser.add_argument("--kind", action="append", choices=["headword", "hebrew-word"],
                        help="restrict to a candidate queue; repeatable")
    parser.add_argument("--engine", default="kraken-headword-finetune",
                        help="engine label recorded in hypotheses")
    parser.add_argument("--pad", type=int, default=16,
                        help="crop padding; 16 maximised exact matches on the reviewed set")
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--dry-run", action="store_true",
                        help="report what would change and write nothing")
    args = parser.parse_args()

    kinds = set(args.kind) if args.kind else {"headword", "hebrew-word"}

    reviewed = collections.defaultdict(set)
    for line in args.journal.read_text().splitlines():
        if line.strip():
            record = json.loads(line)
            reviewed[record["sample"]].add(record.get("source_digest"))

    model_sha = digest_bytes(args.model.read_bytes())
    network = None if args.dry_run else load_recognizer(args.model, args.device)

    seeded = skipped = lines_done = 0
    skipped_names = []
    changed_samples = []
    for directory in sorted(args.root.iterdir()):
        manifest_path, draft_path = directory / "review.json", directory / "draft.json"
        if not manifest_path.is_file() or not draft_path.is_file():
            continue
        manifest_bytes, draft_bytes = manifest_path.read_bytes(), draft_path.read_bytes()
        manifest, draft = json.loads(manifest_bytes), json.loads(draft_bytes)
        if manifest.get("kind") not in kinds:
            continue
        current_digest = digest_bytes(draft_bytes + manifest_bytes)
        if current_digest in reviewed.get(directory.name, ()):
            skipped += 1
            skipped_names.append(directory.name)
            continue

        crops = {line["line_id"]: line for line in manifest["lines"]}
        readings = {}
        for gold in draft["lines"]:
            crop = crops.get(gold["line_id"])
            if crop is None:
                sys.exit(f"{directory.name}/{gold['line_id']}: missing crop")
            crop_path = (directory / crop["crop"]).resolve()
            if digest_bytes(crop_path.read_bytes()) != crop["crop_sha256"]:
                sys.exit(f"{directory.name}/{gold['line_id']}: crop hash mismatch; not reseeding")
            readings[gold["line_id"]] = None if args.dry_run else recognize(network, crop_path, args.pad)

        if args.dry_run:
            seeded += 1
            lines_done += len(draft["lines"])
            changed_samples.append((directory.name, manifest["kind"], len(draft["lines"])))
            continue

        for gold in draft["lines"]:
            reading = readings[gold["line_id"]]
            previous = gold.get("text", "")
            crop = crops[gold["line_id"]]
            hypotheses = [h for h in crop.get("hypotheses", [])
                          if h.get("engine") != args.engine]
            if previous and not any(h.get("text") == previous for h in hypotheses):
                hypotheses.append({"engine": "tesseract-fused", "text": previous,
                                   "confidence": 0.0})
            hypotheses.append({"engine": args.engine, "text": reading,
                               "model_sha256": model_sha, "confidence": 0.0})
            crop["hypotheses"] = hypotheses
            gold["text"] = reading
            lines_done += 1

        draft["authority"] = (f"Machine {manifest['kind']} candidates from {args.engine}"
                              " awaiting human source review; not gold.")
        manifest["seeded_by"] = {"engine": args.engine, "model": str(args.model),
                                 "model_sha256": model_sha, "pad": args.pad}
        draft_path.write_text(json.dumps(draft, ensure_ascii=False, indent=2) + "\n")
        manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
        seeded += 1
        changed_samples.append((directory.name, manifest["kind"], len(draft["lines"])))

    print(json.dumps({
        "dry_run": args.dry_run,
        "engine": args.engine,
        "model": str(args.model),
        "model_sha256": model_sha,
        "samples_seeded": seeded,
        "lines_seeded": lines_done,
        "samples_skipped_because_reviewed": skipped,
        "skipped": skipped_names[:20],
        "seeded_samples": [{"sample": s, "kind": k, "lines": n} for s, k, n in changed_samples],
    }, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
