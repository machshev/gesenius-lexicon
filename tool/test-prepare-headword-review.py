"""Run with python3 tool/test-prepare-headword-review.py."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import sys
sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location('prepare', Path(__file__).with_name('prepare-headword-review.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PreparationTests(unittest.TestCase):
    def test_invalid_and_fractional_crop_bounds(self):
        self.assertEqual(module.bounds([{'x': 10.8, 'y': 20.2}, {'x': 30.1, 'y': 40.4}]), (10, 20, 21, 21))
        for polygon in [[], [{'x': float('nan'), 'y': 0}], [{'x': 1, 'y': 1}]]:
            with self.assertRaises(ValueError):
                module.bounds(polygon)

    def test_reserved_and_unassigned_pages_are_never_read(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            splits = root / 'splits.toml'
            splits.write_text('edition="robinson-1854"\nsource_sha256="test"\ntraining=["11"]\ndevelopment=[]\nvalidation=["175"]\nfinal_test=["250"]\n')
            catalogue = root / 'sources.toml'
            catalogue.write_text('[[sources]]\nedition="robinson-1854"\nsha256="test"\nprinted_page_offset=-16\n')
            for page in [29, 191, 266]:
                directory = root / f'page-{page:04}'
                directory.mkdir()
                (directory / 'parsed.json').write_text('must not be parsed')
            # A propagated headword is not a fresh detection on fitting page 11.
            directory = root / 'page-0027'
            directory.mkdir()
            (directory / 'parsed.json').write_text(json.dumps({'entries': [{'headword': {
                'diplomatic': 'אָב', 'coordinates': [{'source_page': 17}]}}]}))
            self.assertEqual(module.prepare(root, splits, catalogue, root / 'drafts'), 0)
            self.assertEqual(list((root / 'drafts').iterdir()), [])
            splits.write_text(splits.read_text().replace('validation=["175"]', 'validation=["11"]'))
            with self.assertRaisesRegex(ValueError, 'Overlapping'):
                module.prepare(root, splits, catalogue, root / 'drafts')


if __name__ == '__main__':
    unittest.main()
