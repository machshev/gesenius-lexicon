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


class Pass2CompletenessTests(unittest.TestCase):
    def record(self, errored=0, **extra):
        bands = [{'reread': {'error': 'claude exited 1'}} for _ in range(errored)] + [{'reread': {'lines': []}}]
        return {'pass': 2, 'chunks': [{'bands': bands}], **extra}

    def test_clean_record_passes(self):
        self.assertIsNone(module.pass2_problem(self.record(status='complete', errors=0)))

    def test_errored_bands_are_refused_even_without_status(self):
        self.assertIn('2 errored', module.pass2_problem(self.record(errored=2)))

    def test_incomplete_status_is_refused(self):
        self.assertIn('incomplete', module.pass2_problem(self.record(status='incomplete')))

    def test_failed_adjudication_is_refused(self):
        self.assertIn('1 failed adjudication', module.pass2_problem(self.record(judge_errors=1)))

    def test_pass1_record_is_never_refused(self):
        self.assertIsNone(module.pass2_problem({'pass': 1, 'chunks': []}))


class VerifyIncompleteTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('frontier_verify', Path(__file__).with_name('frontier-verify.py'))
        cls.verify = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.verify)

    def test_first_error_finds_band_then_judgement_errors(self):
        chunks = [{'bands': [{'reread': {'lines': []}}, {'reread': {'error': 'boom'}}]}]
        self.assertEqual(self.verify.first_error(chunks), 'boom')
        chunks = [{'bands': [{'reread': {'lines': []}, 'lines': [{'judgement': {'error': 'judge boom'}}]}]}]
        self.assertEqual(self.verify.first_error(chunks), 'judge boom')
        self.assertIsNone(self.verify.first_error([{'bands': [{'reread': {'lines': []}}]}]))

    def test_quota_pattern_matches_limit_messages(self):
        self.assertTrue(self.verify.QUOTA_PATTERN.search('{"result":"You have hit your usage limit"}'))
        self.assertFalse(self.verify.QUOTA_PATTERN.search('no structured output'))


class TierOrderTests(unittest.TestCase):
    CHUNKS = [
        ("t1-c1-r00", 1, 0, 1), ("t1-c0-r01", 0, 1, 1), ("c1-r00", 1, 0, 0),
        ("section-1", -1, 0, 1), ("c0-r00", 0, 0, 0), ("header", -1, 0, 0),
        ("t1-c0-r00", 0, 0, 1),
    ]

    def chunk(self, cid, column, row, tier, with_tier=True):
        c = {"chunk_id": cid, "column": column, "row": row}
        if with_tier:
            c["tier"] = tier
        return c

    def check(self, with_tier):
        chunks = [self.chunk(*c, with_tier=with_tier) for c in self.CHUNKS]
        ordered = sorted(chunks, key=module.chunk_order)
        self.assertEqual([c["chunk_id"] for c in ordered], [
            "header", "c0-r00", "c1-r00", "section-1", "t1-c0-r00", "t1-c0-r01", "t1-c1-r00"])
        ids = [module.region_id_for(c) for c in ordered]
        self.assertEqual(ids, ["header", "column-1", "column-2", "section-1", "t1-column-1", "t1-column-1", "t1-column-2"])
        unique = list(dict.fromkeys(ids))
        self.assertEqual(sorted(unique, key=module.region_order), unique)

    def test_tiers_from_the_tier_field(self):
        self.check(True)

    def test_tiers_from_the_chunk_id_prefix(self):
        self.check(False)

    def test_section_heading_is_not_the_running_head(self):
        self.assertEqual(module.region_id_for(self.chunk("section-2", -1, 0, 2)), "section-2")
        self.assertEqual(module.region_id_for(self.chunk("header", -1, 0, 0)), "header")


class RunningHeadTests(unittest.TestCase):
    def test_points_dagesh_meteg_and_accents_go_but_shin_dots_stay(self):
        f = module.unpoint_running_head
        self.assertEqual(f("נָשָׁה 700 נָשַׁק"), "נשׁה 700 נשׁק")
        self.assertEqual(f("שָׂרַף"), "שׂרף")
        self.assertEqual(f("נָֽשֶׁךְ"), "נשׁך")
        self.assertEqual(f("בּ\u05c7\u0591"), "ב")

    def test_other_text_is_untouched(self):
        self.assertEqual(module.unpoint_running_head("LEXICON. 100 ā"), "LEXICON. 100 ā")

    def test_result_is_nfc_and_keeps_maqaf(self):
        self.assertEqual(module.unpoint_running_head("אֵל־שַׁדַּי"), "אל־שׁדי")


if __name__ == '__main__':
    unittest.main()
