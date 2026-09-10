#!/usr/bin/env python3
"""Apply source-reviewed false-candidate and crop-correction decisions once."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess


def digest_bytes(value):
    return hashlib.sha256(value).hexdigest()


def digest_file(path):
    return digest_bytes(path.read_bytes())


def latest_records(records):
    latest = {}
    for record in records:
        latest[(record['sample'], record['line_id'])] = record
    return latest


def assert_review(record, correction):
    crop_migration = record.get('review_method') == 'crop_corrected_from_reviewed_context'
    already_excluded = record['state'] == 'not_headword'
    if already_excluded or record['state'] == 'resolved':
        return
    if record['state'] != 'unresolved' and not crop_migration and not already_excluded:
        raise ValueError(f"Expected unresolved decision for {correction['sample']} / {correction['line_id']}")
    for field in ['text', 'comment']:
        expected = correction[f'review_{field}']
        if field == 'comment' and crop_migration:
            matches = record[field].startswith(expected)
        else:
            matches = record[field] == expected
        if not matches:
            raise ValueError(f"Review {field} changed for {correction['sample']} / {correction['line_id']}")


def apply(root, journal, corrections_path):
    corrections = json.loads(corrections_path.read_text())
    if corrections['version'] != 1:
        raise ValueError('Unsupported correction manifest')
    records = [json.loads(line) for line in journal.read_text().splitlines() if line.strip()]
    excluded = {(item['sample'], item['line_id']): item
                for item in corrections.get('excluded', [])}
    correction_keys = {
        (item['sample'], item['line_id'])
        for item in corrections['not_headwords'] + corrections['crop_corrections']
        + corrections.get('excluded', [])
    }
    # Rebuild generated migrations from the immutable human decisions. This also
    # makes retries byte-for-byte stable if a crop needs refinement before commit.
    human_records = [record for record in records if record.get('review_method') not in {
        'crop_corrected_from_reviewed_context', 'source_manifest_migration'
    }]
    unresolved_keys = {
        (record['sample'], record['line_id'], record['source_digest'])
        for record in human_records if record['state'] == 'unresolved'
    }
    records = [record for record in human_records if not (
        record['state'] == 'not_headword'
        and (record['sample'], record['line_id']) in correction_keys
        and (record['sample'], record['line_id'], record['source_digest']) in unresolved_keys
    )]
    latest = latest_records(records)
    not_headwords = {(item['sample'], item['line_id']): item for item in corrections['not_headwords']}
    crop_corrections = {(item['sample'], item['line_id']): item for item in corrections['crop_corrections']}
    for key, correction in {**not_headwords, **excluded, **crop_corrections}.items():
        assert_review(latest[key], correction)

    changed_samples = set()
    for key, correction in crop_corrections.items():
        sample, line_id = key
        directory = root / sample
        review_path = directory / 'review.json'
        review = json.loads(review_path.read_text())
        item = next(line for line in review['lines'] if line['line_id'] == line_id)
        if item['rectangle'] == correction['new_rectangle']:
            if digest_file(directory / item['crop']) != item['crop_sha256']:
                raise ValueError(f'Corrected crop changed for {sample} / {line_id}')
        history = item.get('crop_revision_history', [])
        originated_at_expected_rectangle = any(
            revision['previous']['rectangle'] == correction['old_rectangle'] for revision in history
        )
        if item['rectangle'] != correction['old_rectangle'] and not originated_at_expected_rectangle:
            raise ValueError(f'Crop rectangle changed for {sample} / {line_id}')
        if digest_file(directory / item['context']) != item['context_sha256']:
            raise ValueError(f'Reviewed context changed for {sample} / {line_id}')
        old = (history[0]['previous'] if history else {name: copy.deepcopy(item[name]) for name in [
            'rectangle', 'crop_sha256', 'original_crop_sha256', 'crop_commands'
        ]})
        if item['rectangle'] != correction['new_rectangle']:
            rectangle = correction['new_rectangle']
            geometry = f'{rectangle[2]}x{rectangle[3]}+{rectangle[0]}+{rectangle[1]}'
            for index, field in [(0, 'crop'), (2, 'original_crop')]:
                command = item['crop_commands'][index]
                command[3] = geometry
                subprocess.run(command, check=True, capture_output=True)
                item[f'{field}_sha256'] = digest_file(directory / item[field])
            item['rectangle'] = rectangle
        item['crop_revision_history'] = [{
            'reason': latest[key]['comment'],
            'reviewed_context_sha256': item['context_sha256'],
            'review_source_digest': latest[key]['source_digest'],
            'review_revision': latest[key]['revision'],
            'previous': old
        }]
        review_path.write_text(json.dumps(review, ensure_ascii=False, indent=2) + '\n')
        changed_samples.add(sample)
    appended = []
    # Changing one crop changes the whole sample identity. Rebind each decision
    # to the new manifest, retaining an explicit link to its prior source digest.
    for sample in sorted(changed_samples):
        directory = root / sample
        new_digest = digest_bytes((directory / 'draft.json').read_bytes() + (directory / 'review.json').read_bytes())
        context_hashes = {line['line_id']: line.get('context_sha256')
                          for line in json.loads((directory / 'review.json').read_text())['lines']}
        for key, previous in sorted(latest.items()):
            if key[0] != sample:
                continue
            record = copy.deepcopy(previous)
            record['source_digest'] = new_digest
            record['revision'] = 1
            record['supersedes_source_digest'] = previous['source_digest']
            record['reviewed_context_sha256'] = context_hashes[key[1]]
            if key in crop_corrections:
                record['state'] = 'resolved'
                record['review_method'] = 'crop_corrected_from_reviewed_context'
                record['comment'] = crop_corrections[key]['review_comment'] + ' Corrected crop is a strict subset of the reviewed full-line context.'
            else:
                record['review_method'] = 'source_manifest_migration'
            if key in not_headwords:
                record['state'] = 'not_headword'
            if key in excluded:
                record['state'] = 'excluded'
            appended.append(record)

    # Samples without crop edits retain their identity and receive a normal new
    # revision that structures the reviewer's explicit false-candidate decision.
    for key, correction in sorted(not_headwords.items()):
        if key[0] in changed_samples:
            continue
        previous = latest[key]
        if previous['state'] == 'not_headword':
            continue
        record = copy.deepcopy(previous)
        record['revision'] = previous['revision'] + 1
        record['state'] = 'not_headword'
        appended.append(record)
    for key, correction in sorted(excluded.items()):
        if key[0] in changed_samples:
            continue
        previous = latest[key]
        if previous['state'] == 'excluded':
            continue
        record = copy.deepcopy(previous)
        record['revision'] = previous['revision'] + 1
        record['state'] = 'excluded'
        appended.append(record)
    with journal.open('w') as output:
        for record in records + appended:
            output.write(json.dumps(record, ensure_ascii=False, separators=(',', ':')) + '\n')
    print(f'Applied {len(not_headwords)} false-candidate decisions, {len(excluded)} unusable-crop exclusions, and {len(crop_corrections)} crop corrections; recorded {len(appended)} canonical audit records.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path('benchmarks/transcription-drafts'))
    parser.add_argument('--journal', type=Path, default=Path('corpus/review/transcription-reviews.jsonl'))
    parser.add_argument('--corrections', type=Path, default=Path('corpus/review/headword-candidate-corrections.json'))
    args = parser.parse_args()
    apply(args.root, args.journal, args.corrections)
