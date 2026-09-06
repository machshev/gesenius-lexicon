const { test } = require('node:test');
const assert = require('node:assert/strict');
const { initialRuns, splitRun } = require('../crates/gesenius-core/src/review/transcription-runs.js');
test('page 50 line 4 separates English and pointed Hebrew without changing text', () => {
    const text='Plur. אֵלִם 1. mighty ones, heroes;';
    const runs=initialRuns(text);
    assert.deepEqual(runs,[{language:'en',direction:'ltr',text:'Plur. '},{language:'und',direction:'rtl',text:'אֵלִם'},{language:'en',direction:'ltr',text:' 1. mighty ones, heroes;'}]);
    assert.equal(runs.map(r=>r.text).join(''),text);
});
test('mixed scripts preserve exact scalars and do not infer semantic language', () => {
    for(const text of ['English بَ ܐܰ ἄ 𐤀.', 'a אֶ b', '', 'ሀ and Greek α']) {
        const runs=initialRuns(text);
        assert.equal(runs.map(r=>r.text).join(''),text);
        assert.ok(runs.every(r=>['en','und'].includes(r.language)));
    }
});
test('selection makes a language run including its marks and preserves surrounding text', () => {
    const run={language:'en',direction:'ltr',text:'word אֶ next'};
    const result=splitRun(run,5,6,'he');
    assert.deepEqual(result.runs.map(r=>r.text),['word ','אֶ',' next']);
    assert.equal(result.runs[1].direction,'rtl');
    assert.equal(result.runs[1].language,'he');
    const inserted=splitRun(run,5,5,'ar');
    assert.equal(inserted.runs[1].text,'');
    assert.equal(inserted.runs.map(r=>r.text).join(''),run.text);
});
test('splits do not break surrogate pairs or detach pointing at an interior cursor', () => {
    const result=splitRun({language:'und',direction:'rtl',text:'𐤀אֶ'},1,1,'en');
    assert.equal(result.runs[0].text,'𐤀');
    assert.equal(result.runs[1].text,'');
    const pointed=splitRun({language:'he',direction:'rtl',text:'אֶ'},1,1,'en');
    assert.equal(pointed.runs[0].text,'אֶ');
});
