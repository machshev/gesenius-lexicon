#!/usr/bin/env python3
"""Create unreviewed headword crops from cached parsed pages on assigned splits.

This is a candidate queue, never a complete headword inventory or verified gold.
Requires Python 3.11+ and ImageMagick. Existing review samples are not overwritten.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess
import tempfile
import tomllib
import xml.etree.ElementTree as ET


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bounds(polygon, padding=0):
    if not polygon or any(not math.isfinite(p[k]) for p in polygon for k in ('x', 'y')):
        raise ValueError('Invalid crop polygon')
    x = max(0, math.floor(min(p['x'] for p in polygon)) - padding)
    y = max(0, math.floor(min(p['y'] for p in polygon)) - padding)
    right = math.ceil(max(p['x'] for p in polygon)) + padding
    bottom = math.ceil(max(p['y'] for p in polygon)) + padding
    if right <= x or bottom <= y:
        raise ValueError('Empty crop')
    return x, y, right - x, bottom - y


def crop(source, output, rectangle):
    x, y, width, height = rectangle
    command = ['magick', str(source), '-crop', f'{width}x{height}+{x}+{y}', '+repage', str(output)]
    subprocess.run(command, check=True, capture_output=True)
    return command


def prepare(run_root, splits_path, catalogue_path, output,
            selected_partitions=('training', 'development')):
    splits = tomllib.loads(splits_path.read_text())
    source = next(s for s in tomllib.loads(catalogue_path.read_text())['sources'] if s['edition'] == splits['edition'])
    if source['sha256'] != splits['source_sha256']:
        raise ValueError('Split and catalogue source hashes disagree')
    partitions = {}
    for partition in ['training', 'development', 'validation', 'final_test']:
        for page in splits.get(partition, []):
            if page in partitions:
                raise ValueError(f'Overlapping split page {page}')
            partitions[page] = partition
    selected_partitions = tuple(selected_partitions)
    allowed_partitions = {'training', 'development', 'validation'}
    invalid_partitions = set(selected_partitions) - allowed_partitions
    if invalid_partitions:
        raise ValueError(
            'Unsupported review partition(s): ' + ', '.join(sorted(invalid_partitions)))
    # Validation collection must be explicitly requested so it remains separate
    # from ordinary fitting work. Final-test pages are never opened here.
    output.mkdir(parents=True, exist_ok=True)
    count = 0
    for parsed in sorted(run_root.glob('page-*/parsed.json')):
        source_page = int(parsed.parent.name.removeprefix('page-'))
        printed_page = str(source_page + source['printed_page_offset'])
        partition = partitions.get(printed_page)
        if partition not in selected_partitions:
            continue
        data = json.loads(parsed.read_text())
        candidates = []
        seen = set()
        for entry in data['entries']:
            span = entry.get('headword')
            if not span or not span['diplomatic'].strip() or len(span['coordinates']) != 1:
                continue
            coordinate = span['coordinates'][0]
            if coordinate['source_page'] != source_page:
                continue  # propagated entry from an earlier PDF page
            if entry['provenance']['source_sha256'] != source['sha256']:
                raise ValueError('Cached source hash mismatch')
            key = tuple(bounds(coordinate['polygon']))
            if key in seen:
                continue
            seen.add(key)
            candidates.append((entry, span, coordinate))
        # The existing page-1 regression already has its own source-reviewed draft.
        if printed_page == '1' and (output / 'robinson-1854-p001-headword').exists():
            continue
        if not candidates:
            continue
        destination = output / f"{source['edition']}-p{int(printed_page):03}-headwords"
        if destination.exists():
            print(f'Skipping existing sample {destination.name}')
            continue
        image = parsed.parent / 'processed.png'
        original = parsed.parent / 'original.png'
        alto = parsed.parent / 'tesseract-fused.alto.xml'
        line_bounds = {}
        for element in ET.parse(alto).iter():
            if element.tag.rsplit('}', 1)[-1] == 'TextLine':
                a = element.attrib
                line_bounds[a['ID']] = tuple(round(float(a[k])) for k in ['HPOS', 'VPOS', 'WIDTH', 'HEIGHT'])
        with tempfile.TemporaryDirectory(dir=output) as temporary:
            directory = Path(temporary)
            (directory / 'crops').mkdir()
            gold_lines, review_lines = [], []
            for index, (entry, span, coordinate) in enumerate(candidates, 1):
                line_id = f'headword-{index:04}'
                rectangle = bounds(coordinate['polygon'])
                context_rectangle = line_bounds.get(coordinate['line_id'])
                if context_rectangle is None:
                    raise ValueError(f'Missing full-line context for {coordinate["line_id"]}')
                crop_path = f'crops/{line_id}.png'
                context_path = f'crops/{line_id}-context.png'
                original_path = f'crops/{line_id}-original.png'
                commands = [crop(image, directory / crop_path, rectangle),
                            crop(image, directory / context_path, context_rectangle),
                            crop(original, directory / original_path, rectangle)]
                # Temporary paths are implementation details; record replayable final paths.
                for command in commands:
                    command[-1] = str(destination / Path(command[-1]).relative_to(directory))
                gold_lines.append({'line_id': line_id, 'text': span['diplomatic']})
                review_lines.append({'line_id': line_id, 'crop': crop_path,
                    'crop_sha256': digest(directory / crop_path), 'context': context_path,
                    'context_sha256': digest(directory / context_path), 'original_crop': original_path,
                    'original_crop_sha256': digest(directory / original_path),
                    'rectangle': rectangle, 'context_rectangle': context_rectangle,
                    'source_span': span['id'], 'entry_id': entry['id'],
                    'source_line_id': coordinate['line_id'], 'hypotheses': span['hypotheses'],
                    'crop_commands': commands})
            draft = {'id': destination.name, 'edition': source['edition'], 'source_page': source_page,
                     'source_sha256': source['sha256'],
                     'authority': 'Machine headword candidates awaiting human source review; not gold.',
                     'lines': gold_lines}
            review = {'status': 'draft-not-gold', 'kind': 'headword', 'partition': partition,
                      'printed_page': printed_page, 'source_page': source_page,
                      'source_raster_sha256': digest(image), 'original_raster_sha256': digest(original),
                      'parsed_sha256': digest(parsed), 'split_sha256': digest(splits_path),
                      'pipeline_run': run_root.parent.name, 'complete_page_inventory': False,
                      'lines': review_lines,
                      'unresolved': [{'line_id': line['line_id'], 'detail':
                        'Machine suggestion: check every letter and point against the crop and full line. '
                        'If this is not a true headword or the crop is damaged, leave it unresolved with a note.'}
                        for line in gold_lines]}
            for name, value in [('draft.json', draft), ('review.json', review)]:
                (directory / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
            directory.rename(destination)
        count += len(gold_lines)
        print(f'{destination.name}: {len(gold_lines)} unreviewed candidates ({partition})')
    return count


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-root', type=Path, required=True, help='Cached per-edition run directory')
    parser.add_argument('--splits', type=Path, required=True)
    parser.add_argument('--catalogue', type=Path, default=Path('sources.toml'))
    parser.add_argument('--output', type=Path, default=Path('benchmarks/transcription-drafts'))
    parser.add_argument('--partition', action='append', choices=['training', 'development', 'validation'],
                        dest='partitions', help='Partition to seed (repeatable; default: training and development)')
    args = parser.parse_args()
    selected = args.partitions or ('training', 'development')
    print(f'Added {prepare(args.run_root, args.splits, args.catalogue, args.output, selected)} headword candidates; no reviews created.')
