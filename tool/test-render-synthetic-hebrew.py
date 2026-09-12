#!/usr/bin/env python3
"""Tests for the synthetic pointed-Hebrew renderer."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    'render_synthetic_hebrew', ROOT / 'render-synthetic-hebrew.py')
renderer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(renderer)

WORDS = ['אָב', 'נְשָׁמָה', 'כִּנּוֹר', 'שְׁחִיטָה', 'מַעֲצֵבָה', 'בֹּקֶר', 'אֱגוֹז', 'דָּבָר']


def font_dirs():
    """Directories holding the mixture, discovered through the running system."""
    found = []
    for family in ('Ezra SIL', 'Frank Ruehl CLM'):
        result = subprocess.run(['fc-match', '-f', '%{file}', family],
                                capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            found.append(str(Path(result.stdout.strip()).parent))
    return found


class FontDescription(unittest.TestCase):
    def test_weight_travels_inside_the_description(self):
        # ImageMagick's -weight is ignored by the pango: coder, so a bold face
        # can only be requested through the font description itself.
        bold = renderer.font_description({'family': 'Frank Ruhl Libre', 'bold': True}, 96)
        plain = renderer.font_description({'family': 'Frank Ruhl Libre', 'bold': False}, 96)
        self.assertEqual(bold, 'Frank Ruhl Libre Bold 96')
        self.assertEqual(plain, 'Frank Ruhl Libre 96')

    def test_default_mixture_is_not_a_single_face(self):
        families = {font['family'] for font in renderer.DEFAULT_FONTS}
        self.assertGreater(len(families), 3)
        self.assertTrue(all(font['weight'] >= 1 for font in renderer.DEFAULT_FONTS))


class Balancing(unittest.TestCase):
    def test_clusters_keep_a_letter_with_its_points(self):
        # Kaf and kaf-with-dagesh are separate discriminations to learn, so the
        # unit has to be the letter plus the marks attached to it.
        self.assertEqual(renderer.clusters('\u05db\u05b4\u05bc\u05e0'),
                         ['\u05db\u05b4\u05bc', '\u05e0'])
        self.assertEqual(renderer.clusters(''), [])

    def test_alpha_zero_keeps_sampling_uniform(self):
        # The default must reproduce corpora rendered before balancing existed.
        self.assertIsNone(renderer.balance_weights(WORDS, 0.0))
        import random
        rng = lambda i: random.Random(f'seed:{i}')
        self.assertEqual([renderer.pick(WORDS, None, rng(i)) for i in range(20)],
                         [WORDS[rng(i).randrange(len(WORDS))] for i in range(20)])

    def test_balancing_raises_the_share_of_rare_clusters(self):
        import collections, random
        words = ['\u05d1\u05bc\u05d0'] * 40 + ['\u05db\u05bc\u05d0']
        weights = renderer.balance_weights(words, 1.0)
        counts = collections.Counter(
            renderer.pick(words, weights, random.Random(f's{i}')) for i in range(400))
        rare = counts['\u05db\u05bc\u05d0'] / 400
        self.assertGreater(rare, 1 / 41)

    def test_weights_are_a_normalized_distribution(self):
        weights = renderer.balance_weights(WORDS, 1.0)
        self.assertAlmostEqual(sum(weights), 1.0, places=9)
        self.assertTrue(all(w > 0 for w in weights))


class Degradation(unittest.TestCase):
    def test_sampled_height_stays_within_the_measured_range(self):
        import random
        settings = dict(renderer.DEFAULTS)
        rng = random.Random(0)
        heights = [renderer.sample_height(rng, settings) for _ in range(2000)]
        self.assertGreaterEqual(min(heights), settings['height_min'])
        self.assertLessEqual(max(heights), settings['height_max'])
        mean = sum(heights) / len(heights)
        self.assertAlmostEqual(mean, 54.33, delta=1.5)

    def test_ink_bounds_reports_none_for_blank_input(self):
        import numpy as np
        self.assertIsNone(renderer.ink_bounds(np.full((10, 10), 255.0)))
        self.assertIsNotNone(renderer.ink_bounds(np.zeros((10, 10))))


class EndToEnd(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which('magick') or not shutil.which('fc-list'):
            raise unittest.SkipTest('magick and fc-list are required; enter `nix develop`')
        cls.directories = font_dirs()
        if not cls.directories:
            raise unittest.SkipTest('no Hebrew fonts from the mixture are installed')
        cls.temporary = Path(tempfile.mkdtemp(prefix='synthetic-test-'))
        cls.words = cls.temporary / 'words.txt'
        cls.words.write_text('\n'.join(WORDS) + '\n', encoding='utf-8')

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.temporary, ignore_errors=True)

    def render(self, name, *extra, count=6, validation=3, seed=11):
        output = self.temporary / name
        command = [sys.executable, str(ROOT / 'render-synthetic-hebrew.py'),
                   '--words', str(self.words), '--output', str(output),
                   '--count', str(count), '--validation-count', str(validation),
                   '--validation-vocabulary', '0.25', '--seed', str(seed), *extra]
        for directory in self.directories:
            command += ['--font-dir', directory]
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return output

    def test_pairs_labels_and_manifest_agree(self):
        output = self.render('pairs')
        records = [json.loads(line) for line in (output / 'manifest.jsonl').read_text().splitlines()]
        self.assertEqual(len(records), 9)
        for record in records:
            image = Path(record['image'])
            text = Path(record['ground_truth'])
            self.assertTrue(image.exists() and text.exists())
            # The label on disk is exactly the rendered string plus one newline.
            self.assertEqual(text.read_bytes(), record['nfc'].encode('utf-8') + b'\n')
            self.assertEqual(renderer.digest(image.read_bytes()), record['crop_sha256'])
            self.assertEqual(renderer.digest(text.read_bytes()), record['ground_truth_sha256'])
            self.assertIn(record['nfc'], WORDS)

    def test_train_and_validation_vocabularies_are_disjoint(self):
        output = self.render('disjoint')
        summary = json.loads((output / 'summary.json').read_text())
        self.assertEqual(summary['vocabulary_overlap'], [])
        records = [json.loads(line) for line in (output / 'manifest.jsonl').read_text().splitlines()]
        train = {r['nfc'] for r in records if r['split'] == 'train'}
        validation = {r['nfc'] for r in records if r['split'] == 'validation'}
        self.assertEqual(train & validation, set())

    def test_same_seed_reproduces_identical_images(self):
        first = self.render('seed-a')
        second = self.render('seed-b')
        def key(root):
            return [(r['id'], r['nfc'], r['font'], r['crop_sha256'])
                    for r in map(json.loads, (root / 'manifest.jsonl').read_text().splitlines())]
        self.assertEqual(key(first), key(second))

    def test_different_seed_changes_the_images(self):
        first = self.render('vary-a', seed=11)
        second = self.render('vary-b', seed=12)
        def digests(root):
            return {r['crop_sha256'] for r in
                    map(json.loads, (root / 'manifest.jsonl').read_text().splitlines())}
        self.assertEqual(digests(first) & digests(second), set())

    def test_path_lists_match_the_manifest(self):
        output = self.render('lists')
        records = [json.loads(line) for line in (output / 'manifest.jsonl').read_text().splitlines()]
        for split, name in (('train', 'training-paths.txt'),
                            ('validation', 'validation-paths.txt')):
            listed = (output / name).read_text().splitlines()
            expected = [r['image'] for r in records if r['split'] == split]
            self.assertEqual(listed, expected)
            self.assertTrue(all(Path(path).exists() for path in listed))

    def test_existing_output_directory_is_refused(self):
        output = self.render('refuse')
        command = [sys.executable, str(ROOT / 'render-synthetic-hebrew.py'),
                   '--words', str(self.words), '--output', str(output), '--count', '2',
                   '--validation-count', '1']
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('already exists', result.stdout + result.stderr)

    def test_font_without_coverage_is_refused_rather_than_substituted(self):
        # Pango silently falls back to another face for a missing glyph, which
        # would put an unrequested typeface into the corpus under the wrong label.
        config = self.temporary / 'impossible.json'
        config.write_text(json.dumps([{'family': 'No Such Font Family', 'weight': 1}]))
        output = self.temporary / 'nocover'
        command = [sys.executable, str(ROOT / 'render-synthetic-hebrew.py'),
                   '--words', str(self.words), '--output', str(output),
                   '--count', '2', '--validation-count', '1',
                   '--font-config', str(config)]
        for directory in self.directories:
            command += ['--font-dir', directory]
        result = subprocess.run(command, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('do not cover every codepoint', result.stdout + result.stderr)
        self.assertFalse(output.exists())

    def test_rendered_images_carry_ink_at_the_requested_scale(self):
        import numpy as np
        from PIL import Image
        output = self.render('scale', count=8, validation=2)
        records = [json.loads(line) for line in (output / 'manifest.jsonl').read_text().splitlines()]
        for record in records:
            array = np.asarray(Image.open(record['image']).convert('L'), dtype=float)
            bounds = renderer.ink_bounds(array, threshold=array.mean())
            self.assertIsNotNone(bounds, record['id'])
            height = bounds[1] - bounds[0] + 1
            # Degradation and rotation move this a little either way.
            self.assertLess(abs(height - record['target_ink_height']), 12, record['id'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
