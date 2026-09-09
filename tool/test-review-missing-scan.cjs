// Run with: node --test tool/test-review-missing-scan.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

function reviewContext() {
    const source = fs.readFileSync('crates/gesenius-core/src/review.rs', 'utf8');
    const script = source.split('<script>')[1].split('</script>')[0];
    const elements = new Map();
    const context = vm.createContext({
        location: { pathname: '/pages', search: '', hash: '', href: '' },
        URLSearchParams,
        document: {
            body: null,
            querySelector(selector) {
                if (!elements.has(selector)) elements.set(selector, { innerHTML: '' });
                return elements.get(selector);
            },
            querySelectorAll: () => [],
        },
        htmx: { process() {} },
        Image: class { set src(value) { this.onerror(new Error('missing')); } },
    });
    // Skip startup, which fetches the queue; exercise the real rendering functions.
    vm.runInContext(script.slice(0, script.indexOf("$('#reload').onclick=")), context);
    return { context, elements };
}

test('missing entry scans produce an escaped message without rejecting rendering', async () => {
    const { context } = reviewContext();
    const html = await vm.runInContext("scanForPage([], {image:'cache/<missing>.png'}, null)", context);
    assert.match(html, /Scan unavailable/);
    assert.match(html, /cache\/&lt;missing&gt;.png/);
    assert.doesNotMatch(html, /<svg/);
});

test('page review retains navigation while streaming page detail', async () => {
    const { context, elements } = reviewContext();
    await vm.runInContext(`pages=[{edition:'test',source_page:17,printed_page_offset:-16}];renderPage(0)`, context);
    const html = elements.get('#detail').innerHTML;
    assert.match(html, /id="pageSelect"/);
    assert.match(html, /hx-get="\/fragments\/page\?edition=test&amp;source_page=17"/);
    assert.match(html, /Loading page/);
    assert.equal(typeof elements.get('#pageSelect').onchange, 'function');
});

test('entry text and save controls render when the image cannot load', async () => {
    const { context, elements } = reviewContext();
    const entry = {
        id: 'entry-1', revision: 0, headword: null,
        blocks: [{ kind: 'paragraph', spans: [{
            id: 'span-1', normalized: 'Visible entry text', diplomatic: 'Visible entry text',
            confidence: 0.8, hypotheses: [], warnings: [],
            coordinates: [{ page_image: 'missing.png', source_page: 17, printed_page: '1' }],
        }] }],
    };
    await vm.runInContext(`current=${JSON.stringify(entry)};render()`, context);
    const html = elements.get('#detail').innerHTML;
    assert.match(html, /Scan unavailable/);
    assert.match(html, /Visible/);
    assert.match(html, /id="save"/);
    assert.equal(typeof elements.get('#save').onclick, 'function');
});
