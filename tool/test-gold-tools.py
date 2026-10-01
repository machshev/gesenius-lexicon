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


def make_page(root: Path, status_error: bool = False, texts=None, head: bool = False) -> Path:
    """A 300x200 raster with three printed rows and a matching pass 2 record."""
    texts = texts or TEXTS
    chunk_id = "header" if head else "c0-r00"
    (root / "raster").mkdir(exist_ok=True)
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
        "reread": {"lines": texts, "model": "m2", **({"error": "boom"} if status_error else {})},
        "lines": [{"draft": t, "reread": t, "status": "agreed", "text": t} for t in texts],
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
        "chunks": [{"chunk_id": chunk_id, "column": 0, "row": 0, "bounds": {"x": 10, "y": 10, "width": 280, "height": 180}, "bands": [band]}],
        "lines": [{"chunk_id": chunk_id, "column": 0, "text": t, "status": "agreed"} for t in texts],
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
        line = {"triggers": ["blind read differs from pass 2"], "candidate": "abc", "text": "abc", "blind": "abd"}
        self.assertEqual(gr.decide(line, {"lines": []}, "abc")[0], "gold")
        self.assertEqual(gr.decide(line, {"lines": []}, "abd")[0], "contested")  # codex backs the other reader only
        self.assertEqual(gr.decide(line, {"lines": []}, None)[0], "contested")
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


class ConventionTests(unittest.TestCase):
    def test_spacing_fold_is_comparison_only(self):
        for a, b in (("e. g. x", "e.g. x"), ("‘ a tree", "‘a tree"), ("violence, oppression,", "violence,oppression,"), ("אל ,", "אל,"), ("a  b", "a b")):
            self.assertEqual(gq.cmpkey(a), gq.cmpkey(b))
        self.assertNotEqual(gq.cmpkey("ab cd"), gq.cmpkey("abcd"))  # words never merge
        self.assertNotEqual(gq.cmpkey("אָ בּ"), gq.cmpkey("אָבּ"))  # a space after a combining mark is a word break
        self.assertEqual(gq.cmpkey("χϑές"), gq.cmpkey("χθές"))

    def test_gold_text_follows_the_print(self):
        self.assertEqual(gq.canon("e.g. and i.e. x"), "e. g. and i. e. x")
        self.assertEqual(gq.canon("word , next ; end"), "word, next; end")
        self.assertEqual(gq.canon("violence,oppression,wrong,"), "violence,oppression,wrong,")  # tight print stays tight
        self.assertEqual(gq.canon("a well.  Hence"), "a well. Hence")

    def test_pthaha_and_yhwh_have_one_form(self):
        self.assertEqual(gq.canon_pthaha("ܐ▽ܒ"), "ܐܰܒ")
        self.assertEqual(gq.canon_pthaha("ܐܰܒ"), "ܐܰܒ")
        pass2, blind = "יְהֹוָה", "יְהוָֹה"
        self.assertEqual(gq.canon_yhwh(blind), pass2)
        self.assertEqual(gq.canon_yhwh(pass2), pass2)
        self.assertEqual(gq.cmpkey(blind), gq.cmpkey(pass2))
        self.assertEqual(gq.canon_yhwh("הוֹ"), "הוֹ")  # other words are untouched

    def test_running_head_pointing_is_stripped(self):
        self.assertEqual(gq.strip_head_points("נָשָׁה 700 נָשַׁק"), "נשׁה 700 נשׁק")
        self.assertEqual(gq.strip_head_points("Plain ā"), "Plain ā")

    def test_reversed_hebrew_order_flag(self):
        self.assertEqual(gq.hebrew_order_flag("אב גד הו", "הו גד אב"), "Hebrew word order reversed")
        self.assertEqual(gq.hebrew_order_flag("אב גד הו", "גד אב הו"), "Hebrew word order differs")
        self.assertIsNone(gq.hebrew_order_flag("אב גד", "אב גה"))
        self.assertIsNone(gq.hebrew_order_flag("אבּ x", "x אב"))

    def test_token_diff_names_the_differing_tokens(self):
        self.assertEqual(gq.token_diff("אֶתְנָן for x", "אֶתְנַן for x"), [{"at": 0, "a": ["אֶתְנָן"], "b": ["אֶתְנַן"]}])
        self.assertEqual(gq.token_diff("e. g. x", "e.g. x"), [])


class PolicyFlowTests(unittest.TestCase):
    """Blind unclear, header, spacing and 2-of-3 rules with fake readers; no model calls."""

    def run_review(self, root, texts, blind, codex=None, recon=None, head=False, codex_on=True):
        page = make_page(root, texts=texts, head=head)
        calls = {"blind": 0, "reconcile": 0, "codex": 0, "prompts": []}

        def fake_claude(prompt, schema, model, timeout):
            if "lines" in json.loads(schema)["properties"]:
                calls["blind"] += 1
                return {"lines": [{"text": t, "confidence": "certain", "unclear": u} for t, u in blind]}, {"model": model}
            calls["reconcile"] += 1
            calls["prompts"].append(prompt)
            return recon, {"model": model}

        def fake_codex(image, timeout):
            calls["codex"] += 1
            return {"lines": codex if codex is not None else texts}, {}

        args = argparse.Namespace(output=root / "out", model="opus", codex="auto", band=None, workers=1, timeout=1)
        with mock.patch.object(gr.fv, "run_claude", fake_claude), mock.patch.object(
            gr.shutil, "which", lambda name: "/bin/codex" if codex_on else None
        ), mock.patch.object(gr, "run_codex", fake_codex):
            gr.review_page(page, root / "out", args)
        return page, calls, json.loads((root / "out" / "pdf-9001.json").read_text())

    def verdict(self, which, text="Third line", **kw):
        return {"verdict": which, "text": text, "note": "n", "confidence": "certain", "unclear": False, **kw}

    def test_unclear_with_blind_equal_to_pass2_goes_to_codex(self):
        blind = [(t, t == "אל") for t in TEXTS]
        with tempfile.TemporaryDirectory() as tmp:
            _, calls, review = self.run_review(Path(tmp), TEXTS, blind)
            self.assertEqual((calls["reconcile"], calls["codex"]), (0, 1))
            self.assertEqual(review["counts"], {"gold": 3, "contested": 0})
            self.assertIn("blind reader unclear", review["lines"][1]["triggers"])
        with tempfile.TemporaryDirectory() as tmp:
            _, _, review = self.run_review(Path(tmp), TEXTS, blind, codex=["Hello world", "אלף", "Third line"])
            self.assertEqual(review["lines"][1]["decision"], "contested")
            self.assertIn("token_diff_codex", review["lines"][1])
            self.assertEqual(review["counts"], {"gold": 2, "contested": 1})
        with tempfile.TemporaryDirectory() as tmp:
            _, _, review = self.run_review(Path(tmp), TEXTS, blind, codex_on=False)
            self.assertEqual(review["lines"][1]["decision"], "contested")

    def test_unclear_with_a_different_blind_read_is_reconciled_not_excluded(self):
        blind = [(TEXTS[0], False), ("אלף", True), (TEXTS[2], False)]
        with tempfile.TemporaryDirectory() as tmp:
            _, calls, review = self.run_review(Path(tmp), TEXTS, blind, recon=self.verdict("pass2", "אל", unclear=True))
            self.assertEqual((calls["reconcile"], calls["codex"]), (1, 1))
            line = review["lines"][1]
            self.assertEqual((line["decision"], line["text_gold"]), ("gold", "אל"))
            self.assertIn("reviewer unclear", line["triggers"])
            self.assertEqual(line["token_diff_blind"], [{"at": 0, "a": ["אל"], "b": ["אלף"]}])

    def test_codex_alone_never_decides(self):
        blind = [(TEXTS[0], False), ("אלף", False), (TEXTS[2], False)]
        with tempfile.TemporaryDirectory() as tmp:  # reviewer edits to a third form that only codex shares
            _, _, review = self.run_review(Path(tmp), TEXTS, blind, codex=[TEXTS[0], "אלם", TEXTS[2]], recon=self.verdict("edited", "אלם"))
            self.assertEqual(review["lines"][1]["decision"], "contested")
            self.assertIn("only the second reader", review["lines"][1]["decision_reason"])
        with tempfile.TemporaryDirectory() as tmp:  # codex backs blind but the reviewer chose pass 2
            _, _, review = self.run_review(Path(tmp), TEXTS, blind, codex=[TEXTS[0], "אלף", TEXTS[2]], recon=self.verdict("pass2", "אל"))
            self.assertEqual(review["lines"][1]["decision"], "contested")
        with tempfile.TemporaryDirectory() as tmp:  # blind and codex agree with the reviewer
            _, _, review = self.run_review(Path(tmp), TEXTS, blind, codex=[TEXTS[0], "אלף", TEXTS[2]], recon=self.verdict("blind", "אלף"))
            self.assertEqual(review["lines"][1]["text_gold"], "אלף")
            self.assertIn("blind + codex", review["lines"][1]["decision_reason"])
        with tempfile.TemporaryDirectory() as tmp:  # contested reviewer verdict excludes
            _, _, review = self.run_review(Path(tmp), TEXTS, blind, recon=self.verdict("contested", "אל"))
            self.assertEqual(review["lines"][1]["decision"], "contested")

    def test_reconcile_prompt_demands_the_print(self):
        for needle in ("not the expected lexical or dictionary form", "a missing point as missing" if False else "leave a point out when none is printed", "logical order", "print defect", "no space where none is"):
            self.assertIn(needle, gr.RECONCILE_PROMPT)
        self.assertEqual(gr.PROMPT_VERSION, 1)
        self.assertGreater(gr.RECONCILE_VERSION, 1)

    def test_reversed_order_is_flagged_and_passed_to_reconcile(self):
        texts = ["Hello world", "see אבגד הוזח טיכל and", "Third line"]
        blind = [(texts[0], False), ("see טיכל הוזח אבגד and", False), (texts[2], False)]
        with tempfile.TemporaryDirectory() as tmp:
            _, calls, review = self.run_review(Path(tmp), texts, blind, recon=self.verdict("pass2", texts[1]))
            self.assertIn("blind vs pass 2: Hebrew word order reversed", review["lines"][1]["flags"])
            self.assertIn("reversed", calls["prompts"][0])

    def test_spacing_difference_is_not_a_disagreement_and_the_print_spacing_wins(self):
        texts = ["Hello world", "violence,oppression,wrong,", "Third line"]
        blind = [(texts[0], False), ("violence, oppression, wrong,", False), (texts[2], False)]
        with tempfile.TemporaryDirectory() as tmp:  # reviewer sees the tight print
            _, calls, review = self.run_review(Path(tmp), texts, blind, recon=self.verdict("pass2", "violence,oppression,wrong,"))
            line = review["lines"][1]
            self.assertEqual(calls["codex"], 0)  # a spacing variant is no letter disagreement
            self.assertEqual(line["triggers"], [])
            self.assertEqual(line["text_gold"], "violence,oppression,wrong,")
        with tempfile.TemporaryDirectory() as tmp:  # reviewer fails to answer: pass 2 spacing stands
            _, _, review = self.run_review(Path(tmp), texts, blind, recon={"verdict": "contested", "text": "x", "note": "n", "confidence": "certain", "unclear": False})
            self.assertEqual(review["lines"][1]["text_gold"], "violence,oppression,wrong,")
        with tempfile.TemporaryDirectory() as tmp:  # abbreviation spacing is folded and normalised
            t2 = ["Hello world", "words, e. g. x", "Third line"]
            _, calls, review = self.run_review(Path(tmp), t2, [(t2[0], False), ("words, e.g. x", False), (t2[2], False)], recon=self.verdict("pass2", "words, e. g. x"))
            self.assertEqual(review["lines"][1]["text_gold"], "words, e. g. x")

    def test_running_head_is_unpointed_and_not_sent_pointed_to_gold(self):
        texts = ["נָשָׁה 700 נָשַׁק"]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            page = make_page(root, texts=texts, head=True)
            args = argparse.Namespace(output=root / "out", model="opus", codex="off", band=None, workers=1, timeout=1)
            blind = {"lines": [{"text": "נשׁה 700 נשׁק", "confidence": "certain", "unclear": True}]}
            with mock.patch.object(gr.fv, "run_claude", lambda *a: (blind, {})):
                gr.review_page(page, root / "out", args)
            line = json.loads((root / "out" / "pdf-9001.json").read_text())["lines"][0]
            self.assertEqual(line["triggers"], ["blind reader unclear"])  # pointing was no disagreement
            with mock.patch.object(gr.fv, "run_claude", lambda *a: (blind, {})), mock.patch.object(gr.shutil, "which", lambda n: "/bin/codex"), mock.patch.object(
                gr, "run_codex", lambda image, timeout: ({"lines": ["נָשָׁה 700 נָשַׁק"]}, {})
            ):
                args.codex = "auto"
                gr.review_page(page, root / "out", args)
            line = json.loads((root / "out" / "pdf-9001.json").read_text())["lines"][0]
            self.assertEqual((line["decision"], line["text_gold"]), ("gold", "נשׁה 700 נשׁק"))

    def test_cached_blind_reads_survive_the_reconcile_version_bump(self):
        blind = [(TEXTS[0], False), ("אלף", False), (TEXTS[2], False)]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            page, calls, _ = self.run_review(root, TEXTS, blind, recon=self.verdict("pass2", "אל"))
            side = root / "out" / "pdf-9001.json"
            data = json.loads(side.read_text())
            self.assertTrue(any(k.startswith(f"reconcile:v{gr.RECONCILE_VERSION}:") for k in data["cache"]))
            data["cache"] = {k.replace(f"v{gr.RECONCILE_VERSION}:", ""): v for k, v in data["cache"].items()}  # a version 1 sidecar
            side.write_text(json.dumps(data, ensure_ascii=False))
            _, calls2, _ = self.run_review(root, TEXTS, blind, recon=self.verdict("pass2", "אל"))
            self.assertEqual(calls2["blind"], 0)  # blind reads reused
            self.assertEqual(calls2["reconcile"], 1)  # old reconcile verdicts are not
            self.assertEqual(calls2["codex"], 0)  # the codex reading was reused too

    def test_dry_run_counts_without_calling(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            page = make_page(root)
            args = argparse.Namespace(output=root / "out", model="opus", codex="auto", band=None, workers=1, timeout=1, dry_run=True)
            with mock.patch.object(gr.fv, "run_claude", side_effect=AssertionError("no call")):
                self.assertTrue(gr.review_page(page, root / "out", args))
            self.assertFalse((root / "out" / "pdf-9001.json").exists())


if __name__ == "__main__":
    unittest.main()
