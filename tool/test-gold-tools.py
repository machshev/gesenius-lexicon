"""Run with python3 tool/test-gold-tools.py. Synthetic data only; no model calls."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


gq = load("gold_queue", "gold-queue.py")
gr = load("gold_review", "gold-review.py")
ga = load("gold_assemble", "gold-assemble.py")
TEXTS = ["Hello world", "אל", "Third line"]


def make_page(root: Path, status_error: bool = False) -> Path:
    """A 300x200 raster with three printed rows and a matching pass 2 record."""
    (root / "raster").mkdir()
    pixels = np.full((200, 300), 255, dtype=np.uint8)
    for y in (30, 80, 130):
        pixels[y : y + 20, 20:250] = 0
    raster = root / "raster" / "pdf-9001.png"
    Image.fromarray(pixels).convert("RGB").save(raster)
    pass1 = root / "pdf-9001-pass1.json"
    pass1.write_text(json.dumps({"chunks": [{"model": "m1"}]}))
    band = {
        "band_id": "c0-r00-b00",
        "bounds": {"x": 10, "y": 10, "width": 280, "height": 180},
        "reread": {"lines": TEXTS, "model": "m2", **({"error": "boom"} if status_error else {})},
        "lines": [{"draft": t, "reread": t, "status": "agreed", "text": t} for t in TEXTS],
    }
    record = {
        "edition": "robinson-1854",
        "pdf_page": 9001,
        "pass": 2,
        "pass1": str(pass1),
        "generated_at": "2026-10-01T00:00:00+00:00",
        "raster": str(raster),
        "raster_sha256": gq.fv.ft.sha256_file(raster),
        "raster_size": {"width": 300, "height": 200},
        "chunks": [{"chunk_id": "c0-r00", "column": 0, "row": 0, "bounds": {"x": 10, "y": 10, "width": 280, "height": 180}, "bands": [band]}],
        "lines": [{"chunk_id": "c0-r00", "column": 0, "text": t, "status": "agreed"} for t in TEXTS],
        "status": "complete",
    }
    path = root / "pdf-9001.json"
    path.write_text(json.dumps(record, ensure_ascii=False))
    return path


class ClassifyTests(unittest.TestCase):
    def test_diff_classes(self):
        self.assertEqual(gq.classify("a b", "a  b"), "none")
        self.assertEqual(gq.classify("Hi, there", "Hi there."), "punctuation")
        self.assertEqual(gq.classify("one Two", "Two one"), "order only")
        self.assertEqual(gq.classify("Plutarch", "plutarch"), "case")
        self.assertEqual(gq.classify("הַל", "הֵל"), "points only")
        self.assertEqual(gq.classify("אור", "אר"), "vav/yod only")
        self.assertEqual(gq.classify("أق", "ܐܡ"), "script change")
        self.assertEqual(gq.classify("abc", "abd"), "letter change")

    def test_theta_form_and_possessive_fold(self):
        self.assertEqual(gq.classify("χϑές", "χθές"), "none")
        self.assertEqual(gq.classify("David's", "David’s"), "none")

    def test_tiers(self):
        base = {"status": "agreed", "diff": "none"}
        self.assertEqual(gq.tier({**base, "text": "Plain English"}), 3)
        self.assertEqual(gq.tier({**base, "text": "x אל"}), 2)
        self.assertEqual(gq.tier({**base, "text": "ܐܡ"}), 1)
        self.assertEqual(gq.tier({**base, "status": "adjudicated", "text": "Plain"}), 1)
        self.assertEqual(gq.tier({**base, "diff": "order only", "text": "Plain"}), 1)


class GlyphTests(unittest.TestCase):
    def test_only_the_possessive_apostrophe_is_converted(self):
        p = gq.alto.printed_glyphs
        self.assertEqual(p("David's skill"), "David’s skill")
        self.assertEqual(p("Jeb'a"), "Jeb'a")
        self.assertEqual(p("spec. ' to be"), "spec. ' to be")
        self.assertEqual(p("χθές"), "χθές")
        self.assertEqual(p("a’s"), "a’s")


class QueueTests(unittest.TestCase):
    def test_incomplete_record_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            record = json.load(open(make_page(Path(tmp), status_error=True)))
            with self.assertRaises(SystemExit):
                gq.build_queue(record)

    def test_queue_covers_every_line_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            queue = gq.build_queue(json.load(open(make_page(Path(tmp)))))
            self.assertEqual([ln["index"] for q in queue for ln in q["lines"]], [0, 1, 2])


class DecideTests(unittest.TestCase):
    def test_gold_needs_converging_readers(self):
        line = {"triggers": ["blind read differs from pass 2"], "candidate": "abc"}
        self.assertEqual(gr.decide(line, {"lines": []}, "abc")[0], "gold")
        self.assertEqual(gr.decide(line, {"lines": []}, "abd")[0], "contested")
        self.assertEqual(gr.decide(line, None, None)[0], "contested")
        self.assertEqual(gr.decide(line, {"error": "x"}, None)[0], "contested")
        self.assertEqual(gr.decide({"triggers": [], "candidate": "abc"}, None, None)[0], "gold")
        self.assertEqual(gr.decide({"triggers": [], "candidate": "abc", "reason": "unclear"}, None, None)[0], "contested")

    def test_fixture_trigger(self):
        fixture = [gq.fold("Plutarch, Ant.")]
        line = {"pass1": "Plutarch. Ant.", "text": "Plutarch. Ant."}
        self.assertTrue(gr.differs_from_fixture(line, fixture))
        self.assertFalse(gr.differs_from_fixture({**line, "pass1": "Plutarch Ant."}, fixture))
        self.assertFalse(gr.differs_from_fixture(line, []))


class ReviewAndAssembleTests(unittest.TestCase):
    def review(self, root, blind_third="Third lyne", codex=True):
        page = make_page(root)
        calls = []

        def fake_claude(prompt, schema, model, timeout):
            calls.append(model)
            if "lines" in json.loads(schema)["properties"]:
                lines = [{"text": t, "confidence": "certain", "unclear": False} for t in TEXTS[:2] + [blind_third]]
                return {"lines": lines}, {"model": model}
            return {"verdict": "pass2", "text": "Third line", "note": "n", "confidence": "certain", "unclear": False}, {"model": model}

        args = argparse.Namespace(output=root / "out", model="opus", codex="auto", band=None, workers=1, timeout=1)
        with mock.patch.object(gr.fv, "run_claude", fake_claude), mock.patch.object(
            gr.shutil, "which", lambda name: "/bin/codex" if codex else None
        ), mock.patch.object(gr, "run_codex", lambda image, timeout: ({"lines": TEXTS}, {})):
            self.assertTrue(gr.review_page(page, root / "out", args))
        return page, calls, json.loads((root / "out" / "pdf-9001.json").read_text())

    def test_blind_agreement_makes_gold_and_a_rerun_is_free(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            page, calls, review = self.review(root, blind_third="Third line")
            self.assertEqual(review["counts"], {"gold": 3, "contested": 0})
            self.assertEqual(len(calls), 1)
            args = argparse.Namespace(output=root / "out", model="opus", codex="off", band=None, workers=1, timeout=1)
            with mock.patch.object(gr.fv, "run_claude", side_effect=AssertionError("no model call expected")):
                gr.review_page(page, root / "out", args)

    def test_disagreement_needs_the_second_reader(self):
        with tempfile.TemporaryDirectory() as tmp:
            _, _, review = self.review(Path(tmp))
            self.assertEqual(review["counts"], {"gold": 3, "contested": 0})
            self.assertEqual(review["lines"][2]["triggers"], ["blind read differs from pass 2"])
        with tempfile.TemporaryDirectory() as tmp:
            _, _, review = self.review(Path(tmp), codex=False)
            self.assertEqual(review["counts"], {"gold": 2, "contested": 1})
            self.assertIsNone(review["lines"][2]["text_gold"])

    def test_assemble_excludes_contested_and_anchors_inside_raster(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            page, _, review = self.review(root, codex=False)
            record = json.load(open(page))
            fixture, excluded = ga.assemble(review, record)
            self.assertEqual([ln["text"] for ln in fixture["lines"]], TEXTS[:2])
            self.assertEqual(len(excluded), 1)
            self.assertEqual([ln["line_id"] for ln in fixture["lines"]], ["pdf9001-c1-001", "pdf9001-c1-002"])
            for ln in fixture["lines"]:
                b = ln["source"]["bounds"]
                self.assertTrue(b["x"] >= 0 and b["x"] + b["width"] <= 300 and b["y"] + b["height"] <= 200)
            self.assertIn("frontier-raster-400dpi-pdf9001-sha256-", fixture["source_image"]["coordinate_frame"])
            self.assertEqual(len(fixture["source_sha256"]), 64)

    def test_assemble_refusals(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            page, _, review = self.review(root)
            record = json.load(open(page))
            with self.assertRaises(SystemExit):
                ga.assemble({**review, "status": "incomplete"}, record)
            with self.assertRaises(SystemExit):
                ga.assemble({**review, "pass2_generated_at": "older"}, record)
            with self.assertRaises(SystemExit):
                ga.assemble(review, {**record, "status": "incomplete"})
            bad = json.loads(json.dumps(review))
            bad["lines"][0]["text_gold"] = "a‏b"
            with self.assertRaises(SystemExit):
                ga.assemble(bad, record)
            moved = json.loads(json.dumps(record))
            moved["raster_sha256"] = "0" * 64
            with self.assertRaises(SystemExit):
                ga.assemble(review, moved)


if __name__ == "__main__":
    unittest.main()
