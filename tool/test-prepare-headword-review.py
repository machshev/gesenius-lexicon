"""Run with python3 tool/test-prepare-headword-review.py."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import sys
import xml.etree.ElementTree as ET
sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location('prepare', Path(__file__).with_name('prepare-headword-review.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PreparationTests(unittest.TestCase):
    def test_missing_run_root_is_rejected(self):
        missing = Path('/definitely/missing/review-run')
        with self.assertRaisesRegex(ValueError, 'Run root does not exist'):
            module.prepare(missing, missing, missing, missing)
        with self.assertRaisesRegex(ValueError, 'Run root does not exist'):
            module.prepare_hebrew_words(missing, missing, missing, missing)

    def test_invalid_and_fractional_crop_bounds(self):
        self.assertEqual(module.bounds([{'x': 10.8, 'y': 20.2}, {'x': 30.1, 'y': 40.4}]), (10, 20, 21, 21))
        for polygon in [[], [{'x': float('nan'), 'y': 0}], [{'x': 1, 'y': 1}]]:
            with self.assertRaises(ValueError):
                module.bounds(polygon)

    def test_hebrew_word_filter_and_alto_bounds(self):
        self.assertEqual(module.hebrew_word('\u200fהַמֶּלֶךְ\u200e'), 'הַמֶּלֶךְ')
        self.assertEqual(module.hebrew_word('12 אָב'), '12 אָב')
        self.assertIsNone(module.hebrew_word('English'))
        self.assertIsNone(module.hebrew_word('a א'))
        word = ET.fromstring('<String HPOS="10.4" VPOS="20" WIDTH="30.2" HEIGHT="40.1"/>')
        self.assertEqual(module.alto_rectangle(word), (8, 18, 35, 45))
        candidates = [('first', 'אָב'), ('duplicate', 'אָב'), ('diverse', 'שָׁלוֹם')]
        self.assertEqual(module.representative_words(candidates, 2),
                         [('first', 'אָב'), ('diverse', 'שָׁלוֹם')])
        with self.assertRaises(ValueError):
            module.representative_words(candidates, 0)

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
            with self.assertRaisesRegex(ValueError, 'Unsupported review partition'):
                module.prepare(root, splits, catalogue, root / 'drafts', ('final_test',))
            splits.write_text(splits.read_text().replace('validation=["175"]', 'validation=["11"]'))
            with self.assertRaisesRegex(ValueError, 'Overlapping'):
                module.prepare(root, splits, catalogue, root / 'drafts')

    def test_general_hebrew_words_exclude_headword_lines(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run_root = root / 'run' / 'robinson-1854'
            page = run_root / 'page-0027'
            page.mkdir(parents=True)
            splits = root / 'splits.toml'
            splits.write_text('edition="robinson-1854"\nsource_sha256="test"\ntraining=["11"]\ndevelopment=[]\nvalidation=[]\nfinal_test=[]\n')
            catalogue = root / 'sources.toml'
            catalogue.write_text('[[sources]]\nedition="robinson-1854"\nsha256="test"\nprinted_page_offset=-16\n')
            (page / 'parsed.json').write_text(json.dumps({'entries': [{
                'provenance': {'source_sha256': 'test'},
                'headword': {'coordinates': [{'source_page': 27, 'line_id': 'headword-line'}]}
            }]}))
            (page / 'tesseract-fused.alto.xml').write_text('''<alto><Layout><Page><TextBlock>
              <TextLine ID="headword-line" HPOS="1" VPOS="2" WIDTH="50" HEIGHT="10">
                <String ID="headword" CONTENT="אָב" HPOS="5" VPOS="2" WIDTH="12" HEIGHT="10"/>
              </TextLine>
              <TextLine ID="body-line" HPOS="1" VPOS="20" WIDTH="80" HEIGHT="10">
                <String ID="body" CONTENT="מֶלֶךְ" WC="0.7" HPOS="7" VPOS="20" WIDTH="20" HEIGHT="10"/>
                <String ID="noise" CONTENT="א" HPOS="30" VPOS="20" WIDTH="5" HEIGHT="10"/>
              </TextLine>
            </TextBlock></Page></Layout></alto>''')
            (page / 'processed.png').write_bytes(b'processed')
            (page / 'original.png').write_bytes(b'original')
            original_crop = module.crop

            def fake_crop(source, output, rectangle):
                output.write_bytes(f'{source.name}:{rectangle}'.encode())
                return ['magick', str(source), '-crop', str(rectangle), '+repage', str(output)]

            module.crop = fake_crop
            try:
                output = root / 'drafts'
                self.assertEqual(module.prepare_hebrew_words(
                    run_root, splits, catalogue, output, ('training',)), 1)
            finally:
                module.crop = original_crop
            sample = output / 'robinson-1854-p011-hebrew-words'
            draft = json.loads((sample / 'draft.json').read_text())
            review = json.loads((sample / 'review.json').read_text())
            self.assertEqual(draft['lines'], [
                {'line_id': 'hebrew-word-0001', 'text': 'מֶלֶךְ'}])
            self.assertEqual(review['kind'], 'hebrew-word')
            self.assertEqual(review['lines'][0]['source_line_id'], 'body-line')


if __name__ == '__main__':
    unittest.main()
