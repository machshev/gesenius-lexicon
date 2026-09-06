// Run with: node --test tool/test-transcription-keyboard.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { edit, backspace, fromCodePoint, layouts, ethiopicRows } = require('../crates/gesenius-core/src/review/transcription-keyboard.js');

test('Ethiopic table preserves every key and vowel columns across incomplete series', () => {
    const keys = layouts.find(layout => layout[0] === 'ethiopic')[3].flatMap(group => group[1]);
    const { rows, extras } = ethiopicRows(keys);
    assert.deepEqual(rows[0].map(key => key?.[0]), [...'ሀሁሂሃሄህሆሇ']);
    const qw = rows.find(row => row[0][0] === 'ቈ');
    assert.deepEqual(qw.map(key => key?.[0]), ['ቈ', undefined, 'ቊ', 'ቋ', 'ቌ', 'ቍ', undefined, undefined]);
    assert.ok(extras.some(key => key[0] === 'ፘ'));
    assert.ok(extras.some(key => key[0] === '፡'));
    const actual = [...rows.flat().filter(Boolean), ...extras].map(key => key[0]);
    assert.deepEqual(actual.sort(), keys.map(key => key[0]).sort());
    assert.equal(new Set(actual).size, keys.length);
});

test('Hebrew points attach without replacing a selected letter or inserting dotted circles', () => {
    assert.deepEqual(edit('abc א xyz', 5, 5, '\u05b6'), {value: 'abc אֶ xyz', cursor: 6});
    assert.deepEqual(edit('א', 0, 1, '\u05b6'), {value: 'אֶ', cursor: 2});
    assert.deepEqual(edit('אֶ', 0, 2, '\u05bc'), {value: 'אֶּ', cursor: 3});
    assert.throws(() => edit('', 0, 0, '\u05b0'));
    assert.throws(() => edit('ab', 0, 2, '\u05b0'));
    assert.throws(() => edit('a ', 2, 2, '\u05b0'));
});

test('literal Greek, Arabic and Syriac marks are preserved without normalization', () => {
    assert.equal(edit('α', 1, 1, '\u0313').value, 'α\u0313');
    assert.equal(edit('α\u0313', 2, 2, '\u0301').value, 'α\u0313\u0301');
    assert.equal(edit('ب', 1, 1, '\u064e').value, 'بَ');
    assert.equal(edit('ܐ', 1, 1, '\u0730').value, 'ܐܰ');
});

test('selection replacement and backspace preserve astral characters and remove one mark', () => {
    assert.deepEqual(edit('hello', 1, 4, '𐤀'), {value: 'h𐤀o', cursor: 3});
    assert.deepEqual(backspace('x𐤀', 3, 3), {value: 'x', cursor: 1});
    assert.deepEqual(backspace('אֶּ', 3, 3), {value: 'אֶ', cursor: 2});
    assert.deepEqual(edit('𐤀', 1, 1, 'x'), {value: '𐤀x', cursor: 3});
    assert.deepEqual(backspace('𐤀', 0, 1), {value: '', cursor: 0});
});

test('code point entry allows rare glyphs but rejects controls, surrogates and invalid values', () => {
    assert.equal(fromCodePoint('U+10900'), '𐤀');
    assert.equal(fromCodePoint('05b0'), '\u05b0');
    for (const input of ['D800', '110000', 'U+202E', '0000', 'garbage', '05B0 05B1']) {
        assert.throws(() => fromCodePoint(input));
    }
});

test('seven palettes contain named scalar values and the required pointing characters', () => {
    assert.equal(layouts.length, 7);
    const all = layouts.flatMap(layout => layout[3].flatMap(group => group[1]));
    for (const [glyph, name] of all) {
        assert.equal([...glyph].length, 1);
        assert.ok(name.length);
        assert.equal(fromCodePoint(glyph.codePointAt(0).toString(16).padStart(4, '0')), glyph);
    }
    for (const glyph of ['\u05b0','\u05c1','\u05c2','\u05c7','\u064e','\u0730','\u0313','\u0345','ሀ','𐤀']) {
        assert.ok(all.some(key => key[0] === glyph));
    }
});
