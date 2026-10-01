"""Run with python3 tool/test-frontier-transcribe.py. Synthetic arrays only; no model calls."""
import importlib.util
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True

import numpy as np

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("frontier_transcribe", HERE / "frontier-transcribe.py")
ft = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ft)

COLUMNS = [(20, 180), (220, 380)]


def page(glyph: bool, rule: bool = True) -> np.ndarray:
    ink = np.zeros((1000, 400), dtype=bool)
    ink[:, 20:180:7] = True
    ink[:, 220:380:7] = True
    if rule:
        # A tall column rule in the gutter, broken where the heading sits.
        ink[0:430, 195:205] = True
        ink[510:1000, 195:205] = True
    if glyph:
        # A single centred letter, 20 px wide and 40 rows tall, over the rule.
        ink[450:490, 190:210] = True
    return ink


class SectionBreaks(unittest.TestCase):
    def test_centred_glyph_over_rule_is_a_break(self):
        breaks = ft.detect_section_breaks(page(glyph=True), COLUMNS, 100)
        self.assertEqual(len(breaks), 1)
        self.assertTrue(440 <= breaks[0][0] <= 455 and 485 <= breaks[0][1] <= 495)

    def test_rule_alone_is_not_a_break(self):
        self.assertEqual(ft.detect_section_breaks(page(glyph=False), COLUMNS, 100), [])

    def test_rule_without_gap_is_not_a_break(self):
        ink = page(glyph=False)
        ink[:, 195:205] = True
        self.assertEqual(ft.detect_section_breaks(ink, COLUMNS, 100), [])

    def test_speck_is_not_a_break(self):
        ink = page(glyph=False, rule=False)
        ink[450:455, 195:200] = True
        self.assertEqual(ft.detect_section_breaks(ink, COLUMNS, 100), [])

    def test_glyph_above_body_top_is_ignored(self):
        self.assertEqual(ft.detect_section_breaks(page(glyph=True), COLUMNS, 400), [])


if __name__ == "__main__":
    unittest.main()
