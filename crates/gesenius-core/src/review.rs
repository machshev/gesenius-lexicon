//! Append-only correction patches and a local optimistic-lock review service.

mod transcription;

use crate::corpus_io::load_entries;
use crate::metrics::normalized_disagreement;
use crate::model::{CorpusEntry, Point, ReviewState};
use crate::unicode::{aggregate_confidence, refresh_span};
use crate::validate::validate_entry;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

/// One canonical, append-only review change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPatch {
    /// Entry being replaced.
    pub entry_id: String,
    /// Revision observed by the reviewer.
    pub base_revision: u64,
    /// Newly assigned revision.
    pub revision: u64,
    /// Reviewer identity or local handle.
    pub reviewer: String,
    /// Optional audit note.
    pub comment: Option<String>,
    /// UTC review timestamp.
    pub reviewed_at: DateTime<Utc>,
    /// Complete replacement entry.
    pub replacement: CorpusEntry,
}

/// HTTP review update payload.
#[derive(Debug, Deserialize)]
struct ReviewRequest {
    base_revision: u64,
    reviewer: String,
    comment: Option<String>,
    review_state: ReviewState,
    entry: CorpusEntry,
}

/// Summary used by queue browsing.
#[derive(Debug, Serialize)]
struct EntrySummary {
    id: String,
    edition: String,
    printed_page: String,
    headword: Option<String>,
    confidence: f32,
    review_state: ReviewState,
    revision: u64,
    warnings: usize,
    disagreement: f64,
    queued: bool,
}

#[derive(Debug, Serialize)]
struct PageEntrySummary {
    id: String,
    headword: Option<String>,
    review_state: ReviewState,
    polygons: Vec<Vec<Point>>,
}

#[derive(Debug, Serialize)]
struct PageSummary {
    edition: String,
    source_page: u32,
    printed_page: Option<String>,
    page_image: String,
    entries: Vec<PageEntrySummary>,
}

/// On-disk base corpus plus append-only patches.
#[derive(Debug, Clone)]
pub struct ReviewStore {
    corpus_root: PathBuf,
    patch_path: PathBuf,
}

impl ReviewStore {
    /// Opens a corpus directory, creating only the patch directory.
    pub fn open(corpus_root: &Path, patch_path: &Path) -> Result<Self> {
        if !corpus_root.is_dir() {
            bail!("corpus root does not exist: {}", corpus_root.display());
        }
        if let Some(parent) = patch_path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            corpus_root: corpus_root.to_owned(),
            patch_path: patch_path.to_owned(),
        })
    }

    /// Loads base JSONL files and applies patches in append order.
    pub fn materialized_entries(&self) -> Result<Vec<CorpusEntry>> {
        let mut entries = load_base_entries(&self.corpus_root)?;
        let patches = load_patches(&self.patch_path)?;
        apply_patch_sequence(&mut entries, &patches)?;
        entries.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(entries)
    }

    /// Appends a validated optimistic revision while holding an OS file lock.
    pub fn apply(
        &self,
        base_revision: u64,
        reviewer: &str,
        comment: Option<String>,
        review_state: ReviewState,
        mut replacement: CorpusEntry,
    ) -> Result<ReviewPatch> {
        if reviewer.trim().is_empty() {
            bail!("reviewer must not be empty");
        }
        if review_state == ReviewState::Machine {
            bail!("a human review must be `corrected` or `verified`");
        }
        let mut patch_file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.patch_path)?;
        patch_file.lock_exclusive()?;

        let result = (|| {
            let mut entries = load_base_entries(&self.corpus_root)?;
            patch_file.seek(SeekFrom::Start(0))?;
            let patches = read_patches(&patch_file)?;
            apply_patch_sequence(&mut entries, &patches)?;
            let current = entries
                .iter()
                .find(|entry| entry.id == replacement.id)
                .with_context(|| format!("unknown entry `{}`", replacement.id))?;
            if current.revision != base_revision {
                bail!(
                    "revision conflict for `{}`: expected {}, current {}",
                    replacement.id,
                    base_revision,
                    current.revision
                );
            }
            if replacement.provenance != current.provenance
                || replacement.edition != current.edition
                || replacement.printed_page != current.printed_page
                || replacement.entry_ordinal != current.entry_ordinal
            {
                bail!("source identity and provenance cannot be changed during review");
            }
            replacement.revision = base_revision + 1;
            replacement.review_state = review_state;
            let has_explicit_span_review = replacement
                .spans()
                .any(|span| span.review_state != ReviewState::Machine);
            replacement.for_each_span_mut(|span| {
                refresh_span(span);
                if !has_explicit_span_review {
                    span.review_state = review_state;
                }
            });
            replacement.confidence = aggregate_confidence(replacement.spans());
            let issues = validate_entry(&replacement);
            let errors: Vec<_> = issues
                .iter()
                .filter(|issue| issue.severity.is_error())
                .collect();
            if !errors.is_empty() {
                bail!(
                    "review would create invalid entry: {}",
                    errors
                        .iter()
                        .map(|issue| issue.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
            let patch = ReviewPatch {
                entry_id: replacement.id.clone(),
                base_revision,
                revision: replacement.revision,
                reviewer: reviewer.trim().to_owned(),
                comment,
                reviewed_at: Utc::now(),
                replacement,
            };
            serde_json::to_writer(&mut patch_file, &patch)?;
            patch_file.write_all(b"\n")?;
            patch_file.sync_all()?;
            Ok(patch)
        })();
        let _ = FileExt::unlock(&patch_file);
        result
    }
}

/// Review web server configuration.
pub struct ReviewServerOptions<'a> {
    /// Bind address, normally loopback.
    pub bind: &'a str,
    /// Base machine corpus directory.
    pub corpus_root: &'a Path,
    /// Append-only patch JSONL path.
    pub patch_path: &'a Path,
    /// Source transcription draft directory.
    pub transcription_drafts: &'a Path,
    /// Roots from which page images may be served.
    pub asset_roots: &'a [PathBuf],
    /// Queue threshold.
    pub confidence_threshold: f32,
    /// OCR engine disagreement threshold.
    pub disagreement_threshold: f64,
}

/// Serves the local review UI until interrupted.
pub fn serve(options: &ReviewServerOptions<'_>) -> Result<()> {
    let store = ReviewStore::open(options.corpus_root, options.patch_path)?;
    let transcriptions = transcription::TranscriptionStore {
        root: options.transcription_drafts.to_owned(),
        journal: options
            .patch_path
            .with_file_name("transcription-reviews.jsonl"),
    };
    let mut asset_roots = options.asset_roots.to_vec();
    asset_roots.push(options.transcription_drafts.to_owned());
    let server = Server::http(options.bind)
        .map_err(|error| anyhow::anyhow!("failed to bind {}: {error}", options.bind))?;
    eprintln!("Gesenius review UI: http://{}", options.bind);
    for request in server.incoming_requests() {
        if matches!(
            request.url().split('?').next(),
            Some(
                "/transcriptions"
                    | "/api/transcriptions"
                    | "/transcription-keyboard.js"
                    | "/transcription-runs.js"
            )
        ) {
            if let Err(error) = transcription::handle(request, &transcriptions) {
                eprintln!("transcription request failed: {error:#}");
            }
            continue;
        }
        if let Err(error) = handle_request(
            request,
            &store,
            &asset_roots,
            options.confidence_threshold,
            options.disagreement_threshold,
        ) {
            eprintln!("review request failed: {error:#}");
        }
    }
    Ok(())
}

fn handle_request(
    mut request: Request,
    store: &ReviewStore,
    asset_roots: &[PathBuf],
    confidence_threshold: f32,
    disagreement_threshold: f64,
) -> Result<()> {
    let url = request.url().to_owned();
    let path = url.split('?').next().unwrap_or("/");
    match (request.method(), path) {
        (&Method::Get, "/") => respond_html(request, REVIEW_UI),
        (&Method::Get, "/api/entries") => {
            let state_filter = query_parameter(&url, "state");
            let queue_only = query_parameter(&url, "queue").as_deref() == Some("true");
            let summaries: Vec<_> = store
                .materialized_entries()?
                .iter()
                .map(|entry| summarize(entry, confidence_threshold, disagreement_threshold))
                .filter(|summary| {
                    state_filter
                        .as_ref()
                        .is_none_or(|state| summary.review_state.as_str() == state)
                        && (!queue_only || summary.queued)
                })
                .collect();
            respond_json(request, StatusCode(200), &summaries)
        }
        (&Method::Get, "/api/pages") => {
            let pages = summarize_pages(&store.materialized_entries()?);
            respond_json(request, StatusCode(200), &pages)
        }
        (&Method::Get, _) if path.starts_with("/api/entries/") => {
            let id = percent_decode(&path["/api/entries/".len()..])?;
            let entry = store
                .materialized_entries()?
                .into_iter()
                .find(|entry| entry.id == id)
                .with_context(|| format!("unknown entry `{id}`"))?;
            respond_json(request, StatusCode(200), &entry)
        }
        (&Method::Patch, _) if path.starts_with("/api/entries/") => {
            let id = percent_decode(&path["/api/entries/".len()..])?;
            let mut body = String::new();
            request
                .as_reader()
                .take(2 * 1024 * 1024)
                .read_to_string(&mut body)?;
            let update: ReviewRequest =
                serde_json::from_str(&body).context("invalid review request JSON")?;
            if update.entry.id != id {
                return respond_error(request, StatusCode(400), "entry ID does not match URL");
            }
            match store.apply(
                update.base_revision,
                &update.reviewer,
                update.comment,
                update.review_state,
                update.entry,
            ) {
                Ok(patch) => respond_json(request, StatusCode(200), &patch),
                Err(error) => {
                    let status = if error.to_string().contains("revision conflict") {
                        StatusCode(409)
                    } else {
                        StatusCode(422)
                    };
                    respond_error(request, status, &format!("{error:#}"))
                }
            }
        }
        (&Method::Get, "/api/image") => {
            let requested = query_parameter(&url, "path").context("missing image path")?;
            let path = resolve_asset(&requested, asset_roots)?;
            let data = fs::read(&path)?;
            let content_type = match path.extension().and_then(|value| value.to_str()) {
                Some("png") => "image/png",
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("tif" | "tiff") => "image/tiff",
                _ => "application/octet-stream",
            };
            request.respond(
                Response::from_data(data)
                    .with_header(content_type_header(content_type))
                    .with_header(security_header()),
            )?;
            Ok(())
        }
        _ => respond_error(request, StatusCode(404), "not found"),
    }
}

fn summarize_pages(entries: &[CorpusEntry]) -> Vec<PageSummary> {
    type PageKey = (String, u32, Option<String>, String);
    let mut pages: BTreeMap<PageKey, BTreeMap<String, PageEntrySummary>> = BTreeMap::new();
    for entry in entries {
        for span in entry.spans() {
            for coordinate in &span.coordinates {
                let key = (
                    entry.edition.clone(),
                    coordinate.source_page,
                    coordinate.printed_page.clone(),
                    coordinate.page_image.clone(),
                );
                let page_entry = pages
                    .entry(key)
                    .or_default()
                    .entry(entry.id.clone())
                    .or_insert_with(|| PageEntrySummary {
                        id: entry.id.clone(),
                        headword: entry
                            .headword
                            .as_ref()
                            .map(|headword| headword.normalized.clone()),
                        review_state: entry.review_state,
                        polygons: Vec::new(),
                    });
                page_entry.polygons.push(coordinate.polygon.clone());
            }
        }
    }
    pages
        .into_iter()
        .map(
            |((edition, source_page, printed_page, page_image), entries)| PageSummary {
                edition,
                source_page,
                printed_page,
                page_image,
                entries: entries.into_values().collect(),
            },
        )
        .collect()
}

fn summarize(
    entry: &CorpusEntry,
    confidence_threshold: f32,
    disagreement_threshold: f64,
) -> EntrySummary {
    let warnings = entry.spans().map(|span| span.warnings.len()).sum::<usize>();
    let disagreement = entry
        .spans()
        .filter_map(|span| {
            let first = span.hypotheses.first()?;
            let second = span.hypotheses.get(1)?;
            Some(normalized_disagreement(&first.text, &second.text))
        })
        .fold(0.0, f64::max);
    EntrySummary {
        id: entry.id.clone(),
        edition: entry.edition.clone(),
        printed_page: entry.printed_page.clone(),
        headword: entry.headword.as_ref().map(|span| span.normalized.clone()),
        confidence: entry.confidence,
        review_state: entry.review_state,
        revision: entry.revision,
        warnings,
        disagreement,
        queued: entry.review_state == ReviewState::Machine
            && (entry.confidence < confidence_threshold
                || warnings > 0
                || disagreement > disagreement_threshold),
    }
}

fn load_base_entries(root: &Path) -> Result<Vec<CorpusEntry>> {
    let mut paths: Vec<_> = fs::read_dir(root)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    paths.sort();
    let mut entries = Vec::new();
    for path in paths {
        entries.extend(load_entries(&path)?);
    }
    Ok(entries)
}

fn load_patches(path: &Path) -> Result<Vec<ReviewPatch>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    read_patches(File::open(path)?)
}

fn read_patches(reader: impl Read) -> Result<Vec<ReviewPatch>> {
    BufReader::new(reader)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            line => Some((index, line)),
        })
        .map(|(index, line)| {
            serde_json::from_str(&line?)
                .with_context(|| format!("invalid review patch at JSONL line {}", index + 1))
        })
        .collect()
}

fn apply_patch_sequence(entries: &mut [CorpusEntry], patches: &[ReviewPatch]) -> Result<()> {
    let mut indices = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if indices.insert(entry.id.clone(), index).is_some() {
            bail!("duplicate base entry ID `{}`", entry.id);
        }
    }
    for patch in patches {
        let index = *indices
            .get(&patch.entry_id)
            .with_context(|| format!("patch refers to missing entry `{}`", patch.entry_id))?;
        let current = &entries[index];
        if current.revision != patch.base_revision
            || patch.revision != patch.base_revision + 1
            || patch.replacement.revision != patch.revision
            || patch.replacement.id != patch.entry_id
        {
            bail!(
                "invalid revision chain for `{}` at revision {}",
                patch.entry_id,
                patch.revision
            );
        }
        entries[index] = patch.replacement.clone();
    }
    Ok(())
}

fn respond_html(request: Request, body: &str) -> Result<()> {
    request.respond(
        Response::from_string(body)
            .with_header(content_type_header("text/html; charset=utf-8"))
            .with_header(security_header()),
    )?;
    Ok(())
}

fn respond_json(request: Request, status: StatusCode, value: &impl Serialize) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    request.respond(
        Response::from_data(body)
            .with_status_code(status)
            .with_header(content_type_header("application/json"))
            .with_header(security_header()),
    )?;
    Ok(())
}

fn respond_error(request: Request, status: StatusCode, message: &str) -> Result<()> {
    respond_json(request, status, &serde_json::json!({ "error": message }))
}

fn content_type_header(value: &str) -> Header {
    Header::from_bytes("Content-Type", value).expect("static header is valid")
}

fn security_header() -> Header {
    Header::from_bytes("X-Content-Type-Options", "nosniff").expect("static header is valid")
}

fn query_parameter(url: &str, name: &str) -> Option<String> {
    url.split_once('?')?.1.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name)
            .then(|| percent_decode(value).ok())
            .flatten()
            .filter(|value| !value.is_empty())
    })
}

fn percent_decode(value: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let input = value.as_bytes();
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'%' if index + 2 < input.len() => {
                let pair = std::str::from_utf8(&input[index + 1..index + 3])?;
                bytes.push(u8::from_str_radix(pair, 16).context("invalid percent encoding")?);
                index += 3;
            }
            b'+' => {
                bytes.push(b' ');
                index += 1;
            }
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(bytes).context("URL is not UTF-8")
}

fn resolve_asset(requested: &str, roots: &[PathBuf]) -> Result<PathBuf> {
    let requested_path = Path::new(requested);
    let candidate = if requested_path.is_absolute() {
        requested_path.to_owned()
    } else {
        std::env::current_dir()?.join(requested_path)
    };
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("missing asset {}", candidate.display()))?;
    let allowed = roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|allowed_root| canonical.starts_with(allowed_root))
    });
    if !allowed || !canonical.is_file() {
        bail!("asset is outside configured roots");
    }
    Ok(canonical)
}

const REVIEW_UI: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<title>Gesenius corpus review</title>
<style>
:root{font-family:"Noto Sans",sans-serif;color:#25231f;background:#eee9df}
body{margin:0} header{padding:.7rem 1rem;background:#313a35;color:white;display:flex;gap:1rem;align-items:center}
main{display:grid;grid-template-columns:minmax(17rem,25rem) 1fr;height:calc(100vh - 3.2rem)}
#list{overflow:auto;border-right:1px solid #aaa;background:#faf8f2}.item{padding:.7rem;border-bottom:1px solid #ddd;cursor:pointer}
.item:hover,.item.active{background:#e3eee8}.hebrew{font:1.35rem "Noto Sans Hebrew",sans-serif;direction:rtl}
#detail{overflow:auto;padding:1rem}.grid{display:grid;grid-template-columns:minmax(20rem,1fr) minmax(22rem,1fr);gap:1rem}
#detail.page-detail{display:grid;grid-template-rows:auto minmax(0,1fr);overflow:hidden}
section{background:white;border:1px solid #d0cbc0;border-radius:.4rem;padding:.8rem}textarea{width:100%;height:28rem;font:13px monospace}
#scan svg{width:100%;height:auto;background:#ddd}.overlay{fill:rgba(238,171,48,.18);stroke:#cf6a16;stroke-width:3}
pre{white-space:pre-wrap}.warn{color:#9a3412}.muted{color:#666;font-size:.85rem}button,select,input{font:inherit;padding:.35rem}
.tabs{display:flex;gap:.35rem;margin-bottom:.7rem}.tabs button[aria-selected="true"]{background:#313a35;color:white}
.entry-text{font:1rem/1.65 "Noto Sans",sans-serif}.entry-text p{margin:.5rem 0;direction:ltr;unicode-bidi:isolate}
.entry-headword{margin:.15rem 0 1rem;text-align:center;font:1.6rem/1.35 "Noto Sans Hebrew",sans-serif;direction:rtl;unicode-bidi:isolate}
.entry-text h3{margin:1.25rem 0 .6rem;text-align:center;font-size:1.18rem;line-height:1.35;letter-spacing:.025em;direction:ltr;unicode-bidi:isolate}
.text-line,.text-word{border-radius:.18rem;cursor:pointer}.text-word{unicode-bidi:isolate}.text-line.selected{background:#fde9a9;box-shadow:0 0 0 .12rem #d97706}
.text-word:hover{background:#f7d77a}.text-word.selected{background:#f59e0b;color:#231700}
.overlay{cursor:pointer}.overlay.selected{fill:rgba(245,158,11,.5);stroke:#9a3412;stroke-width:7}
.structural-block{margin:.65rem 0;padding-left:.65rem;border-left:.2rem solid #d7d0c2}.block-kind{color:#6d685e;font-size:.72rem;font-weight:700;letter-spacing:.06em;text-transform:uppercase}
.page-break{margin:1rem 0 .25rem;color:#6d685e;font-size:.82rem;font-weight:600}.hidden{display:none}
.page-toolbar{display:flex;gap:.5rem;align-items:center;margin-bottom:.7rem}.page-toolbar select{min-width:0;flex:1}.page-canvas{min-height:0}.page-detail .page-canvas{display:grid;grid-template-rows:minmax(0,1fr) auto}
.page-canvas svg{display:block;width:100%;height:100%;min-height:0;background:#ddd}
.page-overlay{cursor:pointer;stroke-width:4}.page-overlay:hover{fill-opacity:.42}
.legend{display:flex;flex-wrap:wrap;gap:.5rem;margin-top:.7rem;max-height:6rem;overflow:auto}.legend button{border-left:.65rem solid var(--entry-color)}
@media(max-width:850px){main{display:block;height:auto}.grid{grid-template-columns:1fr}#list{max-height:35vh}#detail.page-detail{box-sizing:border-box;height:100dvh}}
</style></head>
<body><header><strong>Gesenius review</strong>
<a href="/transcriptions" style="color:white">Transcription review</a>
<button id="entryMode">Entries</button><button id="pageMode">Pages</button>
<span id="entryFilters"><label>State <select id="state"><option value="">all</option><option>machine</option><option>corrected</option><option>verified</option></select></label>
<label><input id="queue" type="checkbox" checked> review queue</label></span><button id="reload">Reload</button></header>
<main><div id="list"></div><div id="detail"><p>Select an entry.</p></div></main>
<script>
const $=s=>document.querySelector(s), esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
let current=null,mode='entries',pages=[];
const entryColor=(index,total)=>`hsl(${Math.round(index*360/Math.max(1,total))} 70% 38%)`;
function setMode(next){mode=next;$('#detail').classList.toggle('page-detail',mode==='pages');$('#entryFilters').classList.toggle('hidden',mode==='pages');$('#entryMode').disabled=mode==='entries';$('#pageMode').disabled=mode==='pages';}
async function loadList(){let q=new URLSearchParams({state:$('#state').value,queue:$('#queue').checked});let rows=await (await fetch('/api/entries?'+q)).json();
$('#list').innerHTML=rows.map(r=>`<div class="item" data-id="${esc(r.id)}"><span class="hebrew">${esc(r.headword||'—')}</span><br><b>${esc(r.edition)}</b> p. ${esc(r.printed_page)}
<div class="muted">${Math.round(r.confidence*100)}% · ${r.review_state} · ${r.warnings} warnings · Δ ${r.disagreement.toFixed(2)}</div></div>`).join('');
document.querySelectorAll('.item').forEach(x=>x.onclick=()=>loadEntry(x.dataset.id));}
async function openEntry(id){setMode('entries');await loadList();await loadEntry(id);}
async function loadPages(selectedImage){pages=await (await fetch('/api/pages')).json();$('#list').innerHTML=pages.map((page,index)=>`<div class="item" data-page="${index}"><b>${esc(page.edition)}</b><br>printed ${esc(page.printed_page||'—')} · PDF ${page.source_page}<div class="muted">${page.entries.length} entries</div></div>`).join('');
document.querySelectorAll('[data-page]').forEach(x=>x.onclick=()=>renderPage(Number(x.dataset.page)));let index=Math.max(0,pages.findIndex(page=>page.page_image===selectedImage));if(pages.length)await renderPage(index);else $('#detail').innerHTML='<p>No pages available.</p>';}
async function renderPage(index){let page=pages[index],imageUrl='/api/image?path='+encodeURIComponent(page.page_image),dimensions=await imageSize(imageUrl);
let polygons=page.entries.flatMap((entry,entryIndex)=>entry.polygons.map(points=>`<polygon class="page-overlay" data-id="${esc(entry.id)}" style="fill:${entryColor(entryIndex,page.entries.length)};fill-opacity:.18;stroke:${entryColor(entryIndex,page.entries.length)}" points="${points.map(point=>point.x+','+point.y).join(' ')}"><title>${esc(entry.headword||entry.id)}</title></polygon>`)).join('');
$('#detail').innerHTML=`<div class="page-toolbar"><button id="previousPage" ${index===0?'disabled':''}>← Previous</button><select id="pageSelect">${pages.map((candidate,i)=>`<option value="${i}" ${i===index?'selected':''}>${esc(candidate.edition)} · printed ${esc(candidate.printed_page||'—')} · PDF ${candidate.source_page}</option>`).join('')}</select><button id="nextPage" ${index===pages.length-1?'disabled':''}>Next →</button></div>
<section class="page-canvas"><svg viewBox="0 0 ${dimensions.width} ${dimensions.height}"><image href="${esc(imageUrl)}" width="${dimensions.width}" height="${dimensions.height}"/>${polygons}</svg>
<div class="legend">${page.entries.map((entry,entryIndex)=>`<button data-entry="${esc(entry.id)}" style="--entry-color:${entryColor(entryIndex,page.entries.length)}">${esc(entry.headword||entry.id)} · ${entry.review_state}</button>`).join('')}</div></section>`;
$('#previousPage').onclick=()=>renderPage(index-1);$('#nextPage').onclick=()=>renderPage(index+1);$('#pageSelect').onchange=event=>renderPage(Number(event.target.value));
document.querySelectorAll('.page-overlay').forEach(x=>x.onclick=()=>openEntry(x.dataset.id));document.querySelectorAll('[data-entry]').forEach(x=>x.onclick=()=>openEntry(x.dataset.entry));}
async function loadEntry(id){current=await (await fetch('/api/entries/'+encodeURIComponent(id))).json();await render();}
function cps(text){return [...text].map(c=>`${c} U+${c.codePointAt(0).toString(16).toUpperCase().padStart(4,'0')}`).join(' · ')}
function imageSize(src){return new Promise((resolve,reject)=>{let image=new Image();image.onload=()=>resolve({width:image.naturalWidth,height:image.naturalHeight});image.onerror=reject;image.src=src;});}
function renderTextSpan(span){let word=0,content=span.normalized.split(/(\s+)/).map(part=>/^\s+$/.test(part)?esc(part):`<span class="text-word" dir="auto" data-word="${word++}">${esc(part)}</span>`).join('');return `<span class="text-line" data-span="${esc(span.id)}">${content}</span>`;}
function renderStructuredText(headword,blocks){let html=headword?`<h2 class="entry-headword" dir="rtl">${renderTextSpan(headword)}</h2>`:`<h2 class="entry-headword" dir="ltr">${esc(current.id)}</h2>`,currentPage=null;
const renderPart=(kind,spans)=>{if(!spans.length)return '';let content=spans.map(renderTextSpan).join(' ');if(kind==='heading')return `<h3 dir="ltr">${content}</h3>`;if(kind==='paragraph')return `<p dir="ltr">${content}</p>`;return `<div class="structural-block"><div class="block-kind">${esc(kind.replaceAll('_',' '))}</div><p dir="ltr">${content}</p></div>`;};
for(let block of blocks){let partPage=null,partSpans=[];for(let span of block.spans){if(!span.normalized)continue;let spanPage=span.coordinates[0]?.printed_page||null;if(partPage!==null&&spanPage!==partPage){html+=renderPart(block.kind,partSpans);partSpans=[];}if(spanPage!==currentPage){if(spanPage)html+=`<div class="page-break">Page ${esc(spanPage)}</div>`;currentPage=spanPage;}partPage=spanPage;partSpans.push(span);}html+=renderPart(block.kind,partSpans);}return html;}
async function scanForPage(spans,page,selectedSpan){let imageUrl='/api/image?path='+encodeURIComponent(page.image),dimensions=await imageSize(imageUrl);
return `<svg viewBox="0 0 ${dimensions.width} ${dimensions.height}"><image href="${esc(imageUrl)}" width="${dimensions.width}" height="${dimensions.height}"/>
${spans.flatMap(s=>s.coordinates.filter(c=>c.page_image===page.image).map(c=>`<polygon class="overlay${s.id===selectedSpan?' selected':''}" data-span="${esc(s.id)}" points="${c.polygon.map(p=>p.x+','+p.y).join(' ')}"><title>${esc(s.normalized)}</title></polygon>`)).join('')}</svg>`;}
async function render(){let spans=[...(current.headword?[current.headword]:[]),...current.blocks.flatMap(b=>b.spans)];
let pages=[];for(let span of spans)for(let coordinate of span.coordinates)if(!pages.some(page=>page.image===coordinate.page_image))pages.push({image:coordinate.page_image,source:coordinate.source_page,printed:coordinate.printed_page});
let selectedSpan=null,selectedPage=0,scan=pages.length?await scanForPage(spans,pages[0],selectedSpan):'No scan coordinate';
let hypotheses=spans.map(s=>`<p><b>${esc(s.id)}</b> <span class="muted">${esc(s.language||'und')} · ${esc(s.script)} ${esc(s.direction)} ${Math.round(s.confidence*100)}%${s.language_runs?.length?' · '+s.language_runs.map(run=>esc(run.language)+' '+esc(run.script)+' '+esc(run.evidence)).join(', '):''}</span><br>
${s.hypotheses.map(h=>`<code>${esc(h.engine)}:</code> ${esc(h.text)} (${Math.round(h.confidence*100)}%)`).join('<br>')}
<br><span class="muted">${esc(cps(s.diplomatic))}</span>${s.warnings.map(w=>`<br><span class="warn">${esc(w.code)}: ${esc(w.message)}</span>`).join('')}</p>`).join('');
$('#detail').innerHTML=`<div class="grid">
<div><section id="scan">${pages.length>1?`<label>Scan page <select id="scanPage">${pages.map((page,index)=>`<option value="${index}">printed ${esc(page.printed)} · PDF ${page.source}</option>`).join('')}</select></label>`:''}<div id="scanCanvas">${scan}</div></section><section><h3>Hypotheses and Unicode</h3>${hypotheses}</section></div>
<section><div class="tabs" role="tablist"><button id="textTab" role="tab" aria-selected="true">Text</button><button id="jsonTab" role="tab" aria-selected="false">Structured JSON</button></div>
<div id="textPanel" role="tabpanel"><div class="entry-text">${renderStructuredText(current.headword,current.blocks)}</div></div>
<div id="jsonPanel" class="hidden" role="tabpanel"><textarea id="editor" spellcheck="false">${esc(JSON.stringify(current,null,2))}</textarea></div>
<p><input id="reviewer" placeholder="Reviewer" autocomplete="name"> <select id="reviewState"><option>corrected</option><option>verified</option></select>
<button id="save">Save revision ${current.revision+1}</button> <button id="viewPage">View on page</button></p><p id="message"></p></section></div>`;
const showPanel=json=>{$('#textPanel').classList.toggle('hidden',json);$('#jsonPanel').classList.toggle('hidden',!json);$('#textTab').setAttribute('aria-selected',!json);$('#jsonTab').setAttribute('aria-selected',json);};
$('#textTab').onclick=()=>showPanel(false);$('#jsonTab').onclick=()=>showPanel(true);
const textLines=()=>[...document.querySelectorAll('.text-line[data-span]')];
const markSelection=(spanId,wordElement)=>{textLines().forEach(line=>line.classList.toggle('selected',line.dataset.span===spanId));document.querySelectorAll('.text-word.selected').forEach(word=>word.classList.remove('selected'));if(wordElement)wordElement.classList.add('selected');document.querySelectorAll('.overlay').forEach(polygon=>polygon.classList.toggle('selected',polygon.dataset.span===spanId));};
const bindScan=()=>document.querySelectorAll('.overlay[data-span]').forEach(polygon=>polygon.onclick=()=>{selectedSpan=polygon.dataset.span;markSelection(selectedSpan);let line=textLines().find(candidate=>candidate.dataset.span===selectedSpan);line?.scrollIntoView({block:'center',behavior:'smooth'});});
const selectText=async(event,line)=>{selectedSpan=line.dataset.span;let span=spans.find(candidate=>candidate.id===selectedSpan),image=span?.coordinates[0]?.page_image,pageIndex=pages.findIndex(page=>page.image===image);if(pageIndex>=0&&pageIndex!==selectedPage){selectedPage=pageIndex;if($('#scanPage'))$('#scanPage').value=String(selectedPage);$('#scanCanvas').innerHTML=await scanForPage(spans,pages[selectedPage],selectedSpan);bindScan();}markSelection(selectedSpan,event.target.closest('.text-word'));};
textLines().forEach(line=>line.onclick=event=>selectText(event,line));bindScan();
if(pages.length>1)$('#scanPage').onchange=async event=>{selectedPage=Number(event.target.value);$('#scanCanvas').innerHTML=await scanForPage(spans,pages[selectedPage],selectedSpan);bindScan();};
$('#viewPage').onclick=async()=>{setMode('pages');await loadPages(pages[selectedPage].image);};
$('#save').onclick=save;}
async function save(){let message=$('#message');try{let entry=JSON.parse($('#editor').value);let response=await fetch('/api/entries/'+encodeURIComponent(current.id),{method:'PATCH',headers:{'Content-Type':'application/json'},
body:JSON.stringify({base_revision:current.revision,reviewer:$('#reviewer').value,review_state:$('#reviewState').value,entry})});
let result=await response.json();if(!response.ok)throw Error(result.error);current=result.replacement;message.textContent='Saved.';await loadList();await render();}catch(e){message.className='warn';message.textContent=e.message;}}
$('#entryMode').onclick=async()=>{setMode('entries');await loadList();$('#detail').innerHTML='<p>Select an entry.</p>';};
$('#pageMode').onclick=async()=>{setMode('pages');await loadPages();};$('#reload').onclick=()=>mode==='entries'?loadList():loadPages();
$('#state').onchange=loadList;$('#queue').onchange=loadList;if(location.hash==='#page-view-smoke-test'){setMode('pages');loadPages().then(()=>{if(innerWidth<=850)$('#detail').scrollIntoView();});}else{setMode('entries');loadList();}
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::{percent_decode, query_parameter, REVIEW_UI};

    #[test]
    fn decodes_url_components() {
        assert_eq!(percent_decode("a%3Ab+c").unwrap(), "a:b c");
    }

    #[test]
    fn empty_query_parameter_can_represent_no_filter() {
        assert_eq!(
            query_parameter("/api/entries?state=&queue=false", "state"),
            None
        );
    }

    #[test]
    fn review_ui_renders_parsed_block_structure() {
        assert!(REVIEW_UI.contains("renderStructuredText(current.headword,current.blocks)"));
        assert!(REVIEW_UI.contains(r#"`<h2 class="entry-headword" dir="rtl">"#));
        assert!(REVIEW_UI.contains("kind==='heading'"));
        assert!(REVIEW_UI.contains("kind==='paragraph'"));
        assert!(REVIEW_UI.contains(r#"<p dir="ltr">"#));
        assert!(
            REVIEW_UI.contains(".entry-text p{margin:.5rem 0;direction:ltr;unicode-bidi:isolate}")
        );
        assert!(!REVIEW_UI.contains(r#"<p dir="auto">"#));
        assert!(!REVIEW_UI.contains("unicode-bidi:plaintext"));
        assert!(!REVIEW_UI.contains("renderParagraphs(spans)"));
    }

    #[test]
    fn review_ui_links_text_lines_and_scan_polygons() {
        assert!(REVIEW_UI.contains(r#"class="text-line" data-span=""#));
        assert!(REVIEW_UI.contains(r#"class="text-word" dir="auto" data-word=""#));
        assert!(REVIEW_UI.contains(".text-word{unicode-bidi:isolate}"));
        assert!(REVIEW_UI.contains(r#"data-span="${esc(s.id)}""#));
        assert!(REVIEW_UI.contains("const selectText=async(event,line)=>"));
        assert!(REVIEW_UI.contains("const bindScan=()=>"));
        assert!(REVIEW_UI.contains("line?.scrollIntoView"));
    }
}
