#!/usr/bin/env python3
"""Re-encode reviewed Hebrew points entered as visually identical wrong scalars.

The review keyboard used to offer U+05BA, U+05C4, U+05C5 and U+05C7 in the same
"Vowel points" row as the points the Robinson 1854 edition actually prints. At
review size they are indistinguishable from holam, the shin dot, hiriq and
qamats, so reviewers selected them by mistake. U+05C4 was never actually used, so no
substitution is asserted for it; it is removed from the palette only.

The edition prints no glyph that distinguishes qamats qatan from qamats, or
holam haser for vav from holam, and U+05C5 is Masoretic punctuation rather than
a vowel at all. A diplomatic transcription of these pages therefore cannot
license any of them: the project's own policy forbids inferring a reading the
pixels do not support. This tool only changes which scalar encodes the mark the
reviewer already saw; it does not re-read a crop or change any decision.

Each affected review gains a new revision that carries the reviewer, state and
timestamp of the decision it re-encodes. Prior revisions stay in the journal.
Repeated execution is byte-for-byte stable.
"""
import argparse
import copy
import json
from pathlib import Path
import unicodedata

# Wrong scalar -> the point the edition actually prints.
SUBSTITUTIONS = {
    'ֺ': 'ֹ',  # HOLAM HASER FOR VAV -> HOLAM
    'ׅ': 'ִ',  # MARK LOWER DOT      -> HIRIQ
    'ׇ': 'ָ',  # QAMATS QATAN        -> QAMATS
}

METHOD = 'pointing_normalization'
COMMENT = ('Pointing normalized: points entered as visually identical scalars the '
           '1854 edition does not print were re-encoded. Reading unchanged.')


def normalize(text):
    return ''.join(SUBSTITUTIONS.get(character, character) for character in text)


def describe(text):
    counts = {}
    for character in text:
        if character in SUBSTITUTIONS:
            counts[character] = counts.get(character, 0) + 1
    return {
        'U+%04X %s' % (ord(character), unicodedata.name(character)): count
        for character, count in sorted(counts.items())
    }


def latest_records(records):
    latest = {}
    for record in records:
        latest[(record['sample'], record['line_id'])] = record
    return latest


def apply(journal, report_path=None):
    records = [json.loads(line) for line in journal.read_text().splitlines() if line.strip()]
    # Regenerate from the decisions this tool did not write so retries are stable.
    records = [record for record in records if record.get('review_method') != METHOD]
    latest = latest_records(records)

    appended = []
    report = []
    for key, previous in sorted(latest.items()):
        text = previous['text']
        corrected = normalize(text)
        if corrected == text:
            continue
        if unicodedata.normalize('NFD', corrected) == unicodedata.normalize('NFD', text):
            raise ValueError(f'Substitution changed nothing under NFD for {key}')
        record = copy.deepcopy(previous)
        record['revision'] = previous['revision'] + 1
        record['review_method'] = METHOD
        record['text'] = corrected
        if record.get('runs'):
            for run in record['runs']:
                run['text'] = normalize(run['text'])
            if ''.join(run['text'] for run in record['runs']) != corrected:
                raise ValueError(f'Runs stopped concatenating to the transcription for {key}')
        if record.get('independent_reading'):
            record['independent_reading'] = normalize(record['independent_reading'])
        record['comment'] = (previous['comment'] + ' ' if previous['comment'] else '') + COMMENT
        appended.append(record)
        report.append({
            'sample': key[0],
            'line_id': key[1],
            'revision': record['revision'],
            'before': text,
            'after': corrected,
            'substituted': describe(text),
        })

    with journal.open('w') as output:
        for record in records + appended:
            output.write(json.dumps(record, ensure_ascii=False, separators=(',', ':')) + '\n')

    total = sum(sum(item['substituted'].values()) for item in report)
    if report_path is not None:
        report_path.write_text(json.dumps({
            'version': 1,
            'substitutions': {
                'U+%04X %s' % (ord(source), unicodedata.name(source)):
                'U+%04X %s' % (ord(target), unicodedata.name(target))
                for source, target in sorted(SUBSTITUTIONS.items())
            },
            'records_corrected': len(report),
            'marks_corrected': total,
            'records': report,
        }, ensure_ascii=False, indent=2) + '\n')
    print(f'Re-encoded {total} points across {len(report)} reviews.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--journal', type=Path, default=Path('corpus/review/transcription-reviews.jsonl'))
    parser.add_argument('--report', type=Path, default=Path('corpus/review/pointing-normalization.json'))
    args = parser.parse_args()
    apply(args.journal, args.report)
