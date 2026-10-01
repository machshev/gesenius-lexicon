"""Run with python3 tool/test-frontier-to-alto.py."""
import importlib.util
from pathlib import Path
import sys
import unittest
sys.dont_write_bytecode = True

import numpy as np

spec = importlib.util.spec_from_file_location('frontier_to_alto', Path(__file__).with_name('frontier-to-alto.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def row(width, blocks, height=10, specks=()):
    """Ink array with solid full-height blocks, given as (x0, x1) column spans.

    Specks are (x0, x1) spans only two pixels tall, so each holds a few
    columns' worth of ink rather than a letter's.
    """
    ink = np.zeros((height, width), dtype=bool)
    for x0, x1 in blocks:
        ink[:, x0:x1] = True
    for x0, x1 in specks:
        ink[4:6, x0:x1] = True
    return ink


class RowExtentTests(unittest.TestCase):
    def test_plain_row_spans_its_ink(self):
        ink = row(400, [(50, 120), (130, 300)])
        self.assertEqual(module.row_extent(ink, 0, 10), (50, 300))

    def test_blank_row_has_no_extent(self):
        self.assertIsNone(module.row_extent(row(100, []), 0, 10))

    def test_single_ink_pixels_are_below_the_column_threshold(self):
        ink = np.zeros((10, 100), dtype=bool)
        ink[3, 5] = True
        self.assertIsNone(module.row_extent(ink, 0, 10))

    def test_row_window_limits_the_rows_measured(self):
        ink = np.zeros((20, 100), dtype=bool)
        ink[0:10, 5:20] = True
        ink[10:20, 60:90] = True
        self.assertEqual(module.row_extent(ink, 0, 10), (5, 20))
        self.assertEqual(module.row_extent(ink, 10, 20), (60, 90))

    def test_specks_before_a_wide_asterisk_do_not_hide_the_indent(self):
        # pdf 1156, first line: 6 to 8 pixel specks left of a 289 pixel asterisk.
        ink = row(1000, [(200, 489), (520, 900)], specks=[(20, 26), (40, 48), (60, 66)])
        self.assertEqual(module.row_extent(ink, 0, 10), (200, 900))

    def test_specks_at_the_right_end_are_ignored_too(self):
        ink = row(1000, [(200, 489), (520, 900)], specks=[(930, 936), (960, 968)])
        self.assertEqual(module.row_extent(ink, 0, 10), (200, 900))

    def test_speck_close_to_the_text_is_kept(self):
        # A gap of 6 columns or fewer joins the speck to the cluster beside it.
        ink = row(400, [(110, 300)], specks=[(100, 106)])
        self.assertEqual(module.row_extent(ink, 0, 10), (100, 300))

    def test_small_mark_with_enough_ink_is_kept(self):
        # 4 columns by 10 rows is 40 ink pixels: the smallest cluster kept.
        self.assertEqual(module.row_extent(row(400, [(50, 54), (200, 300)]), 0, 10), (50, 300))
        # 3 columns by 10 rows is 30: dropped.
        self.assertEqual(module.row_extent(row(400, [(50, 53), (200, 300)]), 0, 10), (200, 300))

    def test_row_of_only_specks_keeps_one_cluster(self):
        ink = row(400, [], specks=[(20, 26), (100, 108)])
        self.assertEqual(module.row_extent(ink, 0, 10), (100, 108))


class AlignTests(unittest.TestCase):
    def test_equal_counts_map_one_to_one(self):
        self.assertEqual(module.align([1.0, 0.9, 0.5], [0.3, 0.3, 0.3]), [0, 1, 2])

    def test_no_lines(self):
        self.assertEqual(module.align([], [0.5, 0.5]), [])

    def test_stray_row_is_skipped(self):
        # A narrow mark (row 1) between two full lines is not given a line.
        self.assertEqual(module.align([1.0, 1.0], [1.0, 0.05, 1.0]), [0, 2])

    def test_leading_and_trailing_stray_rows_are_skipped(self):
        self.assertEqual(module.align([1.0, 1.0], [0.05, 1.0, 1.0, 0.05]), [1, 2])

    def test_short_line_shares_the_row_of_the_previous_line(self):
        # Two short pieces of text sit on one printed row.
        self.assertEqual(module.align([1.0, 0.2, 0.1, 1.0], [1.0, 1.0, 1.0]), [0, 1, 1, 2])

    def test_result_is_monotonic_and_in_range(self):
        lengths = [1.0, 0.4, 0.9, 0.1, 1.0, 0.7]
        widths = [1.0, 0.45, 0.95, 0.9, 1.0, 0.7, 0.05]
        out = module.align(lengths, widths)
        self.assertEqual(len(out), len(lengths))
        self.assertEqual(out, sorted(out))
        self.assertTrue(all(0 <= j < len(widths) for j in out))


if __name__ == '__main__':
    unittest.main()
