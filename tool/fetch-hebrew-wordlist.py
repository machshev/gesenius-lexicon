#!/usr/bin/env python3
"""Build a pointed-Hebrew word list for synthetic rendering.

Two open sources, both recording their exact download digest:

  strongs  Strong's Hebrew lexicon headwords from openscriptures/HebrewLexicon.
           About 8,674 pointed lemmas. This is the distribution the recognizer
           actually has to read, so it is the default.
  wlc      The Westminster Leningrad Codex from openscriptures/morphhb. Running
           text, roughly 305,000 tokens, for breadth of letter/point context.

Both are normalized the way the Robinson 1854 headwords are transcribed:
cantillation removed, meteg removed, the WLC morpheme separator dropped, maqaf
and verse punctuation split, NFC, and nothing kept that is not a Hebrew letter
or a point the edition prints.
The reviewed corpus contains no meteg and no cantillation, so keeping either
would train on a distribution the pages do not show.

This downloads from the network. Output is written under artifacts/, which is
git-ignored: word lists are external inputs, not repository content.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
import unicodedata
import urllib.request

STRONGS_URL = 'https://raw.githubusercontent.com/openscriptures/HebrewLexicon/master/HebrewStrong.xml'
WLC_URL = 'https://raw.githubusercontent.com/openscriptures/morphhb/master/wlc/{book}.xml'

WLC_BOOKS = [
    'Gen', 'Exod', 'Lev', 'Num', 'Deut', 'Josh', 'Judg', 'Ruth', '1Sam', '2Sam',
    '1Kgs', '2Kgs', '1Chr', '2Chr', 'Ezra', 'Neh', 'Esth', 'Job', 'Ps', 'Prov',
    'Eccl', 'Song', 'Isa', 'Jer', 'Lam', 'Ezek', 'Dan', 'Hos', 'Joel', 'Amos',
    'Obad', 'Jonah', 'Mic', 'Nah', 'Hab', 'Zeph', 'Hag', 'Zech', 'Mal',
]

# Points the 1854 edition prints, matching the review keyboard after the
# U+05BA/U+05C4/U+05C5/U+05C7 keys were removed. Meteg and rafe are offered to
# reviewers but do not occur in the reviewed headwords, so they are dropped here.
KEPT_POINTS = set('ְֱֲֳִֵֶַָֹ'
                  'ֻּׁׂ')
LETTERS = {chr(c) for c in range(0x05d0, 0x05eb)}
# Maqaf and the verse punctuation are printed and do join separate words, so
# they are split points. The WLC morpheme separator '/' is an analytical
# annotation that is never printed: splitting on it would manufacture bare
# prefixes like he-ha or bet-shewa that no page shows, so it is simply dropped.
SPLIT_ON = '־׀׃׆-'
DISCARD = '/'


def clean(token):
    """Reduce one source token to the scalars a diplomatic headword would use."""
    token = unicodedata.normalize('NFD', token)
    kept = ''.join(c for c in token
                   if (c in LETTERS or c in KEPT_POINTS or c in SPLIT_ON) and c not in DISCARD)
    return [unicodedata.normalize('NFC', part) for part in re.split(f'[{re.escape(SPLIT_ON)}]', kept)]


def usable(word):
    if not word:
        return False
    if not any(c in LETTERS for c in word):
        return False
    # A point may never lead a word: that is a segmentation artefact, not a reading.
    return unicodedata.normalize('NFD', word)[0] in LETTERS


def fetch(url):
    with urllib.request.urlopen(url, timeout=120) as response:
        return response.read()


def strongs_words(payload):
    text = payload.decode('utf-8')
    return [match for entry in re.findall(r'<w\b[^>]*\bxlit=[^>]*>([^<]*)</w>', text)
            for match in clean(entry)]


def wlc_words(payload):
    text = payload.decode('utf-8')
    return [match for entry in re.findall(r'<w\b[^>]*>(.*?)</w>', text, re.S)
            for match in clean(re.sub(r'<[^>]+>', '', entry))]


def build(sources, output, minimum_length):
    provenance = []
    words = {}
    for name in sources:
        urls = [STRONGS_URL] if name == 'strongs' else [
            WLC_URL.format(book=book) for book in WLC_BOOKS]
        extract = strongs_words if name == 'strongs' else wlc_words
        kept = 0
        for url in urls:
            payload = fetch(url)
            provenance.append({
                'source': name,
                'url': url,
                'bytes': len(payload),
                'sha256': hashlib.sha256(payload).hexdigest(),
            })
            for word in extract(payload):
                if usable(word) and len(unicodedata.normalize('NFD', word)) >= minimum_length:
                    words.setdefault(word, name)
                    kept += 1
            print(f'{name}: {url.rsplit("/", 1)[-1]} -> {len(words)} distinct', file=sys.stderr)
        print(f'{name}: {kept} tokens kept', file=sys.stderr)

    ordered = sorted(words)
    output.parent.mkdir(parents=True, exist_ok=True)
    body = ''.join(word + '\n' for word in ordered)
    output.write_text(body, encoding='utf-8')

    scalars = {}
    for word in ordered:
        for character in unicodedata.normalize('NFC', word):
            scalars['U+%04X' % ord(character)] = scalars.get('U+%04X' % ord(character), 0) + 1
    meta = output.with_suffix('.provenance.json')
    meta.write_text(json.dumps({
        'version': 1,
        'sources': sources,
        'minimum_length': minimum_length,
        'distinct_words': len(ordered),
        'wordlist_sha256': hashlib.sha256(body.encode('utf-8')).hexdigest(),
        'normalization': 'NFC; cantillation, meteg and rafe removed; morpheme separator dropped; split on maqaf and verse punctuation',
        'scalar_counts': dict(sorted(scalars.items())),
        'downloads': provenance,
    }, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(f'Wrote {len(ordered)} distinct words to {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--source', action='append', choices=['strongs', 'wlc'],
                        help='repeatable; defaults to strongs')
    parser.add_argument('--output', type=Path, default=Path('artifacts/wordlists/hebrew.txt'))
    parser.add_argument('--minimum-length', type=int, default=2,
                        help='minimum decomposed scalar count (default 2)')
    args = parser.parse_args()
    build(args.source or ['strongs'], args.output, args.minimum_length)
