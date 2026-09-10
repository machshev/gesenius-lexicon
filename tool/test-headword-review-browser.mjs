// Start Chromium with --headless --remote-debugging-port=9223, then run with Node 22+.
// Uses only local CDP and never submits a review to the live journal.
import assert from 'node:assert/strict';
const reviewBase = process.env.REVIEW_BASE_URL || 'http://127.0.0.1:8787';
const debuggerBase = process.env.CHROME_DEBUG_URL || 'http://127.0.0.1:9223';
const tab = await (await fetch(debuggerBase + '/json/new?about:blank', {method:'PUT'})).json();
const socket = new WebSocket(tab.webSocketDebuggerUrl);
await new Promise(resolve => socket.addEventListener('open', resolve, {once:true}));
let id = 0;
const pending = new Map();
const errors = [];
socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails);
    if (!message.id) return;
    const callback = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) callback.reject(Error(message.error.message));
    else callback.resolve(message.result);
});
function call(method, params = {}) {
    return new Promise((resolve, reject) => {
        const current = ++id;
        pending.set(current, {resolve, reject});
        socket.send(JSON.stringify({id:current, method, params}));
    });
}
async function evaluate(expression) {
    const result = await call('Runtime.evaluate', {expression, returnByValue:true, awaitPromise:true});
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
}
try {
    await call('Runtime.enable');
    await call('Page.enable');
    await call('Emulation.setDeviceMetricsOverride', {width:1280,height:900,deviceScaleFactor:1,mobile:false});
    await call('Page.navigate', {url:reviewBase + '/transcriptions'});
    for (let attempt = 0; attempt < 100; attempt++) {
        if (await evaluate('!!document.querySelector("[data-run-text]") && !!window.UnicodeKeyboard')) break;
        if (attempt === 99 && !await evaluate('!!document.querySelector("[data-run-text]")')) throw Error('Editor did not load');
        await new Promise(resolve => setTimeout(resolve, 100));
        if (await evaluate('!!document.querySelector("[data-run-text]") && document.querySelector("#crop").complete')) break;
    }
    const state = await evaluate(`(() => {
        const field = document.querySelector('[data-run-text]');
        const rect = field.getBoundingClientRect();
        return {samples:document.querySelector('#line').options.length,
            top:rect.top,bottom:rect.bottom,height:innerHeight,
            direction:field.dir,language:field.lang,errors:document.querySelector('#message').textContent};
    })()`);
    assert.ok(state.samples > 1, 'Headword queue needs multiple candidates');
    assert.ok(state.top >= 0 && state.bottom <= state.height, `Editor must be visible without scrolling: ${JSON.stringify(state)}`);
    assert.equal(state.direction, 'rtl');
    assert.equal(state.language, 'he');
    await evaluate(`document.querySelector('[data-run-text]').focus();document.querySelector('[data-run-text]').select()`);
    await call('Input.insertText', {text:'אָב'});
    assert.equal(await evaluate(`document.querySelector('#text').value`), 'אָב');
    assert.deepEqual(await evaluate('window.TranscriptionRuns.values()'), [{language:'he',direction:'rtl',text:'אָב'}]);
    assert.equal(await evaluate(`document.querySelector('#scalarText').textContent`), 'U+05D0 U+05B8 U+05D1');
    assert.equal(await evaluate(`document.querySelector('#notHeadword').textContent`), 'Not a headword');
    assert.equal(await evaluate(`document.querySelector('#excluded').textContent`), 'Exclude unusable crop');
    const unreviewedFilter = await evaluate(`(() => {
        const expected=allLines.filter(line=>line.kind==='headword'&&!line.review).length;
        const checkbox=document.querySelector('#unreviewedOnly');checkbox.click();
        const actual=document.querySelector('#line').options.length;
        const allVisible=lines.every(line=>line.kind==='headword'&&!line.review);
        checkbox.click();return {expected,actual,allVisible};
    })()`);
    assert.equal(unreviewedFilter.actual, unreviewedFilter.expected);
    assert.equal(unreviewedFilter.allVisible, true);
    const reviewedOutcomes = await evaluate(`allLines.filter(line => line.kind === 'headword').map(line => ({
        key:line.sample + '/' + line.line_id, state:line.review?.state,
        crop:line.crop, width:line.sample.includes('p075')&&line.line_id==='headword-0007'?180:
            line.sample.includes('p200')&&line.line_id==='headword-0003'?105:null
    })).filter(item => item.state === 'not_headword' || item.width)`);
    assert.equal(reviewedOutcomes.filter(item => item.state === 'not_headword').length, 3);
    for (const item of reviewedOutcomes.filter(item => item.width)) {
        const dimensions = await evaluate(`new Promise((resolve,reject)=>{const image=new Image();
            image.onload=()=>resolve([image.naturalWidth,image.naturalHeight]);image.onerror=reject;
            image.src='/api/image?path='+encodeURIComponent(${JSON.stringify(item.crop)});})`);
        assert.equal(dimensions[0], item.width);
    }
    await call('Emulation.setDeviceMetricsOverride', {width:390,height:844,deviceScaleFactor:1,mobile:true});
    await evaluate(`document.querySelector('[data-run-text]').scrollIntoView({block:'center'})`);
    assert.equal(await evaluate(`(() => {const f=document.querySelector('[data-run-text]');const r=f.getBoundingClientRect();return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)===f;})()`), true);
    assert.deepEqual(errors, []);
    console.log(`Passed: ${state.samples} headword candidates; visible desktop editor; Hebrew typing, scalars and mobile hit testing. No review submitted.`);
} finally {
    await call('Target.closeTarget', {targetId:tab.id});
    socket.close();
}
