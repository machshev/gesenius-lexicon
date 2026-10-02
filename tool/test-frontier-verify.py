"""Run with python3 tool/test-frontier-verify.py. Real pass 2 lines; no model calls."""
import importlib.util
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("frontier_verify", HERE / "frontier-verify.py")
fv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fv)


class Align(unittest.TestCase):
    def test_reordered_word_list_pairs_instead_of_duplicating(self):
        # pdf 66, c1-r02: the re-read lists the names in the opposite order.
        draft = ["אֵל, אֱלִי, which are usual in pr. names,", "comp. אֱלִימֶלֶךְ, אֶלְיָשִׁיב, אֶלְיָקִים, etc.—", "Among the Phenicians"]
        reread = ["אֵל, אֱלִי, which are usual in pr. names,", "comp. אֱלְיָקִים, אֱלְיָשִׁיב, אֱלִימֶלֶךְ, etc.—", "Among the Phenicians"]
        self.assertEqual(fv.align(draft, reread), [(0, 0), (1, 1), (2, 2)])

    def test_running_head_split_by_pass_1_is_joined_with_its_re_read(self):
        # pdf 116 header: pass 1 gave three lines, the band re-read gives one.
        draft = ["אשר", "100", "אשת", "first body line"]
        reread = ["אשׁר 100 אשׁת", "first body line"]
        joined = fv.join_split_lines(draft, reread)
        self.assertEqual(joined, ["אשר 100 אשת", "first body line"])
        self.assertEqual(fv.align(joined, reread), [(0, 0), (1, 1)])

    def test_separate_body_lines_are_not_joined(self):
        lines = ["the root of the word", "is found in Arabic"]
        self.assertEqual(fv.join_split_lines(lines, ["the root of the word", "is found in Arabic"]), lines)
        self.assertEqual(fv.join_split_lines(lines, ["something else entirely here"]), lines)


if __name__ == "__main__":
    unittest.main()
