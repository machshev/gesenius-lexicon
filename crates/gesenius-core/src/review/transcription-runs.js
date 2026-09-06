/* Ordered language runs; direction is DOM metadata, never hidden Unicode controls. */
(function () {
'use strict';
const languages = [['en','English','ltr'],['he','Hebrew','rtl'],['arc','Aramaic','rtl'],['ar','Arabic','rtl'],['fa','Persian','rtl'],['syc','Syriac','rtl'],['grc','Greek','ltr'],['la','Latin','ltr'],['gez','Geʿez','ltr'],['phn','Phoenician','rtl'],['und','Unspecified','ltr']];
function initialRuns(text) {
    const runs=[];
    let lastScript='Latn';
    for (const char of text) {
        let script='Latn';
        if (/\p{M}/u.test(char)) script=lastScript;
        else for (const name of ['Hebrew','Arabic','Syriac','Phoenician','Greek','Ethiopic']) {
            if (new RegExp('\\p{Script='+name+'}', 'u').test(char)) {script=name;break;}
        }
        const direction=['Hebrew','Arabic','Syriac','Phoenician'].includes(script)?'rtl':'ltr';
        const language=script==='Latn'?'en':'und';
        if (runs.length && lastScript===script) runs.at(-1).text+=char;
        else runs.push({language,direction,text:char});
        lastScript=script;
    }
    return runs.length ? runs : [{language:'en',direction:'ltr',text:''}];
}
function splitRun(run,start,end,language) {
    const bounds=[0,...Array.from(new Intl.Segmenter(undefined,{granularity:'grapheme'}).segment(run.text), item=>item.index+item.segment.length)];
    if(start===end) start=end=bounds.find(bound=>bound>=start)??run.text.length;
    else {start=bounds.filter(bound=>bound<=start).at(-1);end=bounds.find(bound=>bound>=end)??run.text.length;}
    const result=[];
    if(start)result.push({...run,text:run.text.slice(0,start)});
    const index=result.length;
    result.push({language,direction:languages.find(item=>item[0]===language)[2],text:run.text.slice(start,end)});
    if(end<run.text.length)result.push({...run,text:run.text.slice(end)});
    return {runs:result,index};
}
if(typeof module!=='undefined')module.exports={initialRuns,splitRun};
if(typeof document==='undefined')return;
const $=id=>document.getElementById(id);
let runs=[];
function preview() {
    $('runPreview').replaceChildren(...runs.map(run=>{const span=document.createElement('bdi');span.dir=run.direction;span.lang=run.language;span.textContent=run.text;return span;}));
}
function sync() {
    $('text').value=runs.map(run=>run.text).join('');
    preview();
    $('text').dispatchEvent(new Event('input',{bubbles:true}));
}
function focusRun(index) {
    const field=document.querySelectorAll('[data-run-text]')[index];
    field?.focus({preventScroll:true});
}
function render() {
    $('runEditors').replaceChildren(...runs.map((run,index)=>{
        const box=document.createElement('div');box.className='text-run';
        const controls=document.createElement('div');controls.className='controls';
        const label=document.createElement('label');label.textContent=`Run ${index+1} language `;
        const language=document.createElement('select');language.setAttribute('aria-label',`Run ${index+1} language`);
        language.replaceChildren(...languages.map(([code,name])=>new Option(name,code)));language.value=run.language;
        label.append(language);
        const dirLabel=document.createElement('label');dirLabel.textContent='Direction ';
        const direction=document.createElement('select');direction.setAttribute('aria-label',`Run ${index+1} direction`);
        direction.append(new Option('Left to right','ltr'),new Option('Right to left','rtl'));direction.value=run.direction;dirLabel.append(direction);
        const field=document.createElement('textarea');field.dataset.runText=String(index);field.dataset.languageLabel=languages.find(item=>item[0]===run.language)?.[1]||run.language;
        field.setAttribute('aria-label',`Run ${index+1} text`);field.dir=run.direction;field.lang=run.language;field.value=run.text;field.spellcheck=false;field.rows=2;
        field.addEventListener('input',()=>{run.text=field.value;sync();});
        language.onchange=()=>{run.language=language.value;run.direction=languages.find(item=>item[0]===run.language)[2];direction.value=run.direction;field.dir=run.direction;field.lang=run.language;field.dataset.languageLabel=languages.find(item=>item[0]===run.language)[1];sync();focusRun(index);};
        direction.onchange=()=>{run.direction=direction.value;field.dir=run.direction;sync();focusRun(index);};
        const split=document.createElement('button');split.type='button';split.textContent='Create run at cursor / selection';split.onpointerdown=e=>e.preventDefault();
        split.onclick=()=>{if($('save').disabled)return;const parts=splitRun(run,field.selectionStart,field.selectionEnd,$('newRunLanguage').value);runs.splice(index,1,...parts.runs);render();sync();focusRun(index+parts.index);};
        const merge=document.createElement('button');merge.type='button';merge.textContent='Join next run';merge.title='Keeps this run’s language and direction';merge.disabled=index===runs.length-1;
        merge.onclick=()=>{if($('save').disabled)return;run.text+=runs[index+1].text;runs.splice(index+1,1);render();sync();focusRun(index);};
        const remove=document.createElement('button');remove.type='button';remove.textContent='Remove empty run';remove.disabled=run.text.length>0||runs.length===1;
        field.addEventListener('input',()=>{remove.disabled=run.text.length>0||runs.length===1;});
        remove.onclick=()=>{if($('save').disabled)return;runs.splice(index,1);render();sync();focusRun(Math.min(index,runs.length-1));};
        controls.append(label,dirLabel);const actions=document.createElement('div');actions.className='controls run-actions';actions.append(split,merge,remove);
        box.append(controls,field,actions);return box;
    }));
    preview();
    document.dispatchEvent(new Event('transcription-runs-rendered'));
}
$('newRunLanguage').replaceChildren(...languages.map(([code,name])=>new Option(name,code)));
$('newRunLanguage').value='he';
window.TranscriptionRuns={
    load(text,saved){runs=saved?.length?saved.map(run=>({...run})):initialRuns(text);render();},
    values(){return runs.filter(run=>run.text.length).map(run=>({...run}));}
};
})();
