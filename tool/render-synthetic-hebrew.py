#!/usr/bin/env python3
"""Render degraded synthetic pointed-Hebrew word crops for Kraken pretraining.

Produces image/text pairs in the same on-disk shape as
`cargo run -- export-headword-training`, so the result can be fed to `ketos`
directly. It is pretraining material, not reviewed gold: nothing here is
evidence about the Robinson 1854 pages, and the labels are correct only in the
sense that they are what was rendered.

Rendering goes through ImageMagick's PangoCairo delegate, which applies real
HarfBuzz shaping, so combining points land where the font's mark-positioning
tables put them rather than at a naive advance. ImageMagick is already a
dev-shell dependency and already used by `training.rs`.

Fonts and degradation defaults follow `docs/pointed-headword-plan.md`. The font
mixture matters: a single-font synthetic model overfits to that face. Scale
matches the real crops, whose ink height including points above and below is
median 53 px, p10-p90 42-68 px.

Train and validation vocabularies are disjoint, so the synthetic validation
score measures generalization to unseen words rather than memorization. That
score is only for early stopping during pretraining; the reportable number
remains `ketos test` against the real frozen validation manifest.

Determinism: every sample draws from a seed derived from (--seed, split, index),
so output does not depend on --count, ordering, or worker count.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import tempfile
import unicodedata
from xml.sax.saxutils import escape

import numpy as np
from PIL import Image, ImageFilter

# Measured shape similarity to the 1854 type; see the plan's font table.
DEFAULT_FONTS = [
    {'family': 'Frank Ruhl Libre', 'weight': 3, 'bold': True},
    {'family': 'Frank Ruehl CLM', 'weight': 3, 'bold': True},
    {'family': 'Ezra SIL', 'weight': 2, 'bold': False},
    {'family': 'Taamey Frank CLM', 'weight': 2, 'bold': True},
    {'family': 'Bona Nova', 'weight': 1, 'bold': True},
    {'family': 'Keter YG', 'weight': 1, 'bold': True},
    {'family': 'Noto Serif Hebrew', 'weight': 1, 'bold': True},
]

DEFAULTS = {
    # Ink height including points above and below, sampled per image.
    'height_min': 42, 'height_median': 53, 'height_max': 68,
    'rotation_degrees': 0.6,
    'blur_min': 0.6, 'blur_max': 1.6,
    'ink_spread_min': 1.0, 'ink_spread_max': 1.45,
    'contrast_min': 0.80, 'contrast_max': 0.95,
    'paper_min': 10, 'paper_max': 35,
    'noise_min': 3.0, 'noise_max': 9.0,
    'jpeg_probability': 0.5, 'jpeg_quality_min': 45, 'jpeg_quality_max': 85,
    'margin_min': 3, 'margin_max': 12,
}

RENDER_POINTSIZE = 96  # Rendered large, then resampled down to the target height.


def digest(payload):
    return hashlib.sha256(payload).hexdigest()


def font_description(font, pointsize=RENDER_POINTSIZE):
    """A Pango font description string, e.g. 'Frank Ruhl Libre Bold 96'.

    The weight has to travel inside the description: ImageMagick's -weight and
    -font options are silently ignored by the pango: coder, which quietly
    rendered the regular face where the mixture asked for bold.
    """
    style = ' Bold' if font.get('bold') else ''
    return f'{font["family"]}{style} {pointsize}'


def covered_families(codepoints, fontconfig):
    """Families whose charset covers every codepoint, via fontconfig."""
    query = ':charset=' + ' '.join('%04x' % ord(c) for c in sorted(codepoints))
    environment = dict(os.environ)
    if fontconfig:
        environment['FONTCONFIG_FILE'] = str(fontconfig)
    result = subprocess.run(['fc-list', '-f', '%{family[0]}\n', query],
                            capture_output=True, text=True, env=environment)
    if result.returncode:
        raise RuntimeError(f'fc-list failed: {result.stderr.strip()}')
    return {line.strip() for line in result.stdout.splitlines() if line.strip()}


def build_fontconfig(font_dirs, cache_dir):
    """A fontconfig file restricted to the requested directories.

    Restricting the search path is not cosmetic: scanning a large system font
    tree costs over a hundred times as much per render, and it keeps Pango from
    silently substituting some other face for a missing glyph.
    """
    directories = ''.join(f'<dir>{path}</dir>' for path in font_dirs)
    config = cache_dir / 'fonts.conf'
    config.write_text(
        '<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd"><fontconfig>'
        f'{directories}<cachedir>{cache_dir / "cache"}</cachedir></fontconfig>\n')
    (cache_dir / 'cache').mkdir(exist_ok=True)
    return config


def render_text(text, font, fontconfig, destination):
    environment = dict(os.environ)
    if fontconfig:
        environment['FONTCONFIG_FILE'] = str(fontconfig)
    markup = (f'<span font_desc="{escape(font_description(font), {chr(34): "&quot;"})}">'
              f'{escape(text)}</span>')
    command = [
        'magick', '-background', 'white', '-fill', 'black',
        f'pango:{markup}', str(destination),
    ]
    result = subprocess.run(command, capture_output=True, text=True, env=environment)
    if result.returncode or not destination.exists():
        raise RuntimeError(f'magick failed for {font["family"]}: {result.stderr.strip()}')


def ink_bounds(array, threshold=200):
    rows, columns = np.where(array < threshold)
    if not len(rows):
        return None
    return rows.min(), rows.max(), columns.min(), columns.max()


def degrade(image, rng, numpy_rng, settings):
    """Approximate the scan: impression, optics, paper, sensor, then compression."""
    parameters = {}

    angle = rng.uniform(-settings['rotation_degrees'], settings['rotation_degrees'])
    parameters['rotation'] = round(angle, 4)
    image = image.rotate(angle, resample=Image.BICUBIC, expand=True, fillcolor=255)

    blur = rng.uniform(settings['blur_min'], settings['blur_max'])
    parameters['blur'] = round(blur, 4)
    image = image.filter(ImageFilter.GaussianBlur(blur))

    array = np.asarray(image, dtype=float)
    spread = rng.uniform(settings['ink_spread_min'], settings['ink_spread_max'])
    parameters['ink_spread'] = round(spread, 4)
    array = 255.0 - (255.0 - array) * spread

    contrast = rng.uniform(settings['contrast_min'], settings['contrast_max'])
    paper = rng.uniform(settings['paper_min'], settings['paper_max'])
    parameters['contrast'] = round(contrast, 4)
    parameters['paper'] = round(paper, 4)
    array = array * contrast + paper

    noise = rng.uniform(settings['noise_min'], settings['noise_max'])
    parameters['noise'] = round(noise, 4)
    array = array + numpy_rng.normal(0.0, noise, array.shape)

    image = Image.fromarray(np.clip(array, 0, 255).astype(np.uint8), mode='L')

    if rng.random() < settings['jpeg_probability']:
        quality = rng.randint(settings['jpeg_quality_min'], settings['jpeg_quality_max'])
        parameters['jpeg_quality'] = quality
        with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as handle:
            path = Path(handle.name)
        try:
            image.save(path, 'JPEG', quality=quality)
            image = Image.open(path).convert('L')
            image.load()
        finally:
            path.unlink(missing_ok=True)
    return image, parameters


def sample_height(rng, settings):
    """Triangular around the measured median, bounded by the measured decile range."""
    return rng.triangular(settings['height_min'], settings['height_max'],
                          settings['height_median'])


def make_sample(job):
    (index, split, text, fonts, fontconfig, settings, seed, directory, prefix) = job
    rng = random.Random(f'{seed}:{split}:{index}')
    numpy_rng = np.random.default_rng(
        int.from_bytes(hashlib.sha256(f'{seed}:{split}:{index}'.encode()).digest()[:8], 'big'))

    population = [font for font in fonts for _ in range(font['weight'])]
    font = population[rng.randrange(len(population))]

    name = f'{prefix}-{index:06d}'
    raw = directory / f'{name}-raw.png'
    try:
        render_text(text, font, fontconfig, raw)
        image = Image.open(raw).convert('L')
        image.load()
    finally:
        raw.unlink(missing_ok=True)

    bounds = ink_bounds(np.asarray(image, dtype=float))
    if bounds is None:
        raise RuntimeError(f'{font["family"]} rendered {text!r} with no ink')
    top, bottom, left, right = bounds
    image = image.crop((left, top, right + 1, bottom + 1))

    target = sample_height(rng, settings)
    scale = target / image.height
    image = image.resize((max(1, round(image.width * scale)),
                          max(1, round(image.height * scale))), Image.LANCZOS)

    image, parameters = degrade(image, rng, numpy_rng, settings)

    margin = [rng.randint(settings['margin_min'], settings['margin_max']) for _ in range(4)]
    padded = Image.new('L', (image.width + margin[0] + margin[2],
                             image.height + margin[1] + margin[3]),
                       int(np.asarray(image, dtype=float).max()))
    padded.paste(image, (margin[0], margin[1]))

    image_path = directory / f'{name}.png'
    text_path = directory / f'{name}.gt.txt'
    padded.save(image_path, 'PNG', optimize=True)
    text_path.write_bytes(text.encode('utf-8') + b'\n')

    return {
        'id': name,
        'split': split,
        'nfc': text,
        'font': font['family'],
        'font_weight': 'bold' if font.get('bold') else 'normal',
        'font_description': font_description(font),
        'target_ink_height': round(target, 2),
        'margins': margin,
        'degradation': parameters,
        'image': str(image_path.resolve()),
        'ground_truth': str(text_path.resolve()),
        'crop_sha256': digest(image_path.read_bytes()),
        'ground_truth_sha256': digest(text_path.read_bytes()),
    }


def alphabet(records):
    counts = {}
    for record in records:
        for character in record['nfc']:
            key = 'U+%04X' % ord(character)
            counts[key] = counts.get(key, 0) + 1
    return dict(sorted(counts.items()))


def run(args):
    words = [unicodedata.normalize('NFC', line.strip())
             for line in args.words.read_text(encoding='utf-8').splitlines() if line.strip()]
    if len(words) < 2:
        raise SystemExit('word list must contain at least two words')
    wordlist_sha256 = digest(args.words.read_bytes())

    fonts = json.loads(args.font_config.read_text()) if args.font_config else DEFAULT_FONTS
    if args.output.exists():
        raise SystemExit(f'{args.output} already exists; choose a new directory')

    cache = Path(tempfile.mkdtemp(prefix='synthetic-fonts-'))
    try:
        fontconfig = build_fontconfig([str(Path(d).resolve()) for d in args.font_dir],
                                      cache) if args.font_dir else None

        needed = {c for word in words for c in word}
        available = covered_families(needed, fontconfig)
        missing = [font['family'] for font in fonts if font['family'] not in available]
        if missing:
            raise SystemExit(
                'these fonts do not cover every codepoint in the word list, so Pango '
                'would silently substitute another face: ' + ', '.join(missing) +
                '\navailable and covering: ' + ', '.join(sorted(available)))

        # Disjoint vocabularies: a synthetic validation score over shared words
        # would only measure memorization.
        shuffled = sorted(words)
        random.Random(f'{args.seed}:vocabulary').shuffle(shuffled)
        cut = max(1, round(len(shuffled) * args.validation_vocabulary))
        vocabulary = {'validation': shuffled[:cut], 'train': shuffled[cut:]}
        if not vocabulary['train']:
            raise SystemExit('validation vocabulary fraction leaves no training words')

        settings = dict(DEFAULTS)
        for key in settings:
            override = getattr(args, key, None)
            if override is not None:
                settings[key] = override

        args.output.mkdir(parents=True)
        jobs = []
        for split, count, prefix in (('train', args.count, 'synthetic'),
                                     ('validation', args.validation_count, 'synthetic-val')):
            directory = args.output / split
            directory.mkdir()
            pool = vocabulary[split]
            for index in range(count):
                text = pool[random.Random(f'{args.seed}:{split}:word:{index}').randrange(len(pool))]
                jobs.append((index, split, text, fonts, fontconfig, settings,
                             args.seed, directory, prefix))

        records = []
        workers = args.jobs or os.cpu_count() or 1
        with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
            for done, record in enumerate(executor.map(make_sample, jobs), start=1):
                records.append(record)
                if done % 500 == 0 or done == len(jobs):
                    print(f'rendered {done}/{len(jobs)}', file=sys.stderr)

        records.sort(key=lambda record: (record['split'], record['id']))
        manifest = args.output / 'manifest.jsonl'
        with manifest.open('w', encoding='utf-8') as handle:
            for record in records:
                handle.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + '\n')

        by_split = {'train': [], 'validation': []}
        for record in records:
            by_split[record['split']].append(record)
        # Names match what execute_kraken_training writes, so a ketos command
        # copied from the experiment reports works against either directory.
        listings = {'train': 'training-paths.txt', 'validation': 'validation-paths.txt'}
        for split, items in by_split.items():
            listing = args.output / listings[split]
            listing.write_text(''.join(record['image'] + '\n' for record in items))

        duplicates = len(records) - len({record['crop_sha256'] for record in records})
        summary = {
            'version': 1,
            'authority': 'Synthetic renderings. Pretraining material only; not reviewed '
                         'gold and not evidence about any Robinson 1854 page.',
            'seed': args.seed,
            'counts': {split: len(items) for split, items in by_split.items()},
            'duplicate_images': duplicates,
            'wordlist': str(args.words.resolve()),
            'wordlist_sha256': wordlist_sha256,
            'distinct_words': len(words),
            'vocabulary': {split: len(pool) for split, pool in vocabulary.items()},
            'vocabulary_overlap': sorted(set(vocabulary['train']) & set(vocabulary['validation'])),
            'fonts': fonts,
            'font_counts': {family: sum(1 for record in records if record['font'] == family)
                            for family in sorted({record['font'] for record in records})},
            'settings': settings,
            'render_pointsize': RENDER_POINTSIZE,
            'alphabet': {split: alphabet(items) for split, items in by_split.items()},
            'manifest_sha256': digest(manifest.read_bytes()),
        }
        uncovered = sorted(set(summary['alphabet']['validation']) - set(summary['alphabet']['train']))
        summary['validation_scalars_absent_from_train'] = uncovered
        (args.output / 'summary.json').write_text(
            json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + '\n',
            encoding='utf-8')

        print(f'Rendered {len(records)} samples into {args.output}')
        if uncovered:
            print(f'warning: validation uses scalars absent from train: {uncovered}',
                  file=sys.stderr)
        if duplicates:
            print(f'warning: {duplicates} byte-identical images', file=sys.stderr)
    finally:
        shutil.rmtree(cache, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--words', type=Path, default=Path('artifacts/wordlists/hebrew.txt'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--count', type=int, default=50000)
    parser.add_argument('--validation-count', type=int, default=2000)
    parser.add_argument('--validation-vocabulary', type=float, default=0.1,
                        help='fraction of distinct words reserved for validation')
    parser.add_argument('--seed', type=int, default=0)
    parser.add_argument('--jobs', type=int, default=None)
    parser.add_argument('--font-dir', action='append', default=[],
                        help='repeatable; restricts fontconfig to these directories')
    parser.add_argument('--font-config', type=Path,
                        help='JSON list of {family, weight, bold} overriding the default mixture')
    for key, value in DEFAULTS.items():
        parser.add_argument(f'--{key.replace("_", "-")}', type=type(value), default=None)
    run(parser.parse_args())


if __name__ == '__main__':
    main()
