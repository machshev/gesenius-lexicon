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
use std::sync::{Arc, Mutex};
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

#[derive(Debug, Serialize)]
struct PageRange {
    source_start: u32,
    source_end: u32,
    printed_page_offset: Option<i32>,
}

/// On-disk base corpus plus append-only patches.
#[derive(Debug, Clone)]
pub struct ReviewStore {
    /// Base roots from least to most authoritative. Later entries replace
    /// earlier entries with the same stable ID.
    corpus_roots: Vec<PathBuf>,
    patch_path: PathBuf,
    cache: Arc<Mutex<Option<CachedCorpus>>>,
}

#[derive(Debug)]
struct CachedCorpus {
    edition: Option<String>,
    fingerprint: Vec<(PathBuf, u64, Option<std::time::SystemTime>)>,
    entries: Arc<Vec<CorpusEntry>>,
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
            corpus_roots: vec![corpus_root.to_owned()],
            patch_path: patch_path.to_owned(),
            cache: Arc::new(Mutex::new(None)),
        })
    }

    /// Adds a lower-priority corpus whose entries fill gaps in the base corpus.
    pub fn with_fallback_corpus(mut self, corpus_root: &Path) -> Result<Self> {
        if !corpus_root.is_dir() {
            bail!(
                "fallback corpus root does not exist: {}",
                corpus_root.display()
            );
        }
        self.corpus_roots.insert(0, corpus_root.to_owned());
        Ok(self)
    }

    /// Loads base JSONL files and applies patches in append order.
    pub fn materialized_entries(&self) -> Result<Vec<CorpusEntry>> {
        Ok(self.cached_materialized_entries(None)?.as_ref().clone())
    }

    fn cached_materialized_entries(&self, edition: Option<&str>) -> Result<Arc<Vec<CorpusEntry>>> {
        let fingerprint = self.fingerprint(edition)?;
        if let Some(cached) = self.cache.lock().expect("review cache poisoned").as_ref() {
            if cached.edition.as_deref() == edition && cached.fingerprint == fingerprint {
                return Ok(Arc::clone(&cached.entries));
            }
        }
        let mut entries = match edition {
            Some(edition) => load_layered_edition_entries(&self.corpus_roots, edition)?,
            None => load_layered_entries(&self.corpus_roots)?,
        };
        let patches = load_patches(&self.patch_path)?;
        let patches: Vec<_> = patches
            .into_iter()
            .filter(|patch| edition.is_none_or(|edition| patch.replacement.edition == edition))
            .collect();
        apply_patch_sequence(&mut entries, &patches)?;
        entries.sort_by(|left, right| left.id.cmp(&right.id));
        let entries = Arc::new(entries);
        *self.cache.lock().expect("review cache poisoned") = Some(CachedCorpus {
            edition: edition.map(str::to_owned),
            fingerprint,
            entries: Arc::clone(&entries),
        });
        Ok(entries)
    }

    fn fingerprint(
        &self,
        edition: Option<&str>,
    ) -> Result<Vec<(PathBuf, u64, Option<std::time::SystemTime>)>> {
        let mut paths = Vec::new();
        for root in &self.corpus_roots {
            if let Some(edition) = edition {
                paths.push(root.join(format!("{edition}.jsonl")));
            } else {
                paths.extend(
                    fs::read_dir(root)?
                        .filter_map(std::result::Result::ok)
                        .map(|entry| entry.path())
                        .filter(|path| {
                            path.extension()
                                .is_some_and(|extension| extension == "jsonl")
                        }),
                );
            }
        }
        paths.push(self.patch_path.clone());
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let metadata = fs::metadata(&path).ok();
                Ok((
                    path,
                    metadata.as_ref().map_or(0, fs::Metadata::len),
                    metadata.and_then(|value| value.modified().ok()),
                ))
            })
            .collect()
    }

    fn editions(&self) -> Result<Vec<String>> {
        let mut editions = std::collections::BTreeSet::new();
        for root in &self.corpus_roots {
            for path in fs::read_dir(root)?
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "jsonl")
                })
            {
                if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                    editions.insert(stem.to_owned());
                }
            }
        }
        Ok(editions.into_iter().collect())
    }

    fn selected_edition(&self, url: &str) -> Result<String> {
        let edition = query_parameter(url, "edition").context("missing edition")?;
        if !self.editions()?.contains(&edition) {
            bail!("unknown edition `{edition}`");
        }
        Ok(edition)
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
            let mut entries = load_layered_entries(&self.corpus_roots)?;
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
            *self.cache.lock().expect("review cache poisoned") = None;
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
    /// Optional lower-priority index-candidate corpus directory.
    pub index_candidate_root: Option<&'a Path>,
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
    let mut store = ReviewStore::open(options.corpus_root, options.patch_path)?;
    if let Some(root) = options.index_candidate_root.filter(|root| root.is_dir()) {
        store = store.with_fallback_corpus(root)?;
    }
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
                    | "/transcription-fonts/estrangela.ttf"
                    | "/transcription-fonts/serto.ttf"
                    | "/transcription-fonts/eastern.ttf"
                    | "/transcription-fonts/OFL.txt"
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
        (&Method::Get, "/api/editions") => {
            respond_json(request, StatusCode(200), &store.editions()?)
        }
        (&Method::Get, "/api/entries") => {
            let edition = store.selected_edition(&url)?;
            let state_filter = query_parameter(&url, "state");
            let queue_only = query_parameter(&url, "queue").as_deref() == Some("true");
            let summaries: Vec<_> = store
                .cached_materialized_entries(Some(&edition))?
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
            let edition = store.selected_edition(&url)?;
            let catalog =
                summarize_page_catalog(&store.cached_materialized_entries(Some(&edition))?);
            respond_json(request, StatusCode(200), &catalog)
        }
        (&Method::Get, "/fragments/page") => {
            let edition = store.selected_edition(&url)?;
            let source_page = query_parameter(&url, "source_page")
                .context("missing source page")?
                .parse::<u32>()
                .context("invalid source page")?;
            let page = summarize_page(
                &store.cached_materialized_entries(Some(&edition))?,
                &edition,
                source_page,
            )
            .with_context(|| format!("unknown page `{edition}` PDF {source_page}"))?;
            respond_html(request, &render_page_fragment(&page))
        }
        (&Method::Get, _) if path.starts_with("/api/entries/") => {
            let id = percent_decode(&path["/api/entries/".len()..])?;
            let edition = store.selected_edition(&url)?;
            let entry = store
                .cached_materialized_entries(Some(&edition))?
                .iter()
                .find(|entry| entry.id == id)
                .cloned()
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
            let Some(requested) = query_parameter(&url, "path") else {
                return respond_error(request, StatusCode(400), "missing image path");
            };
            let path = match resolve_asset(&requested, asset_roots) {
                Ok(path) => path,
                Err(error) => return respond_error(request, StatusCode(404), &error.to_string()),
            };
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

fn summarize_page_catalog(entries: &[CorpusEntry]) -> Vec<PageRange> {
    let mut pages = BTreeMap::new();
    for entry in entries {
        for coordinate in entry.spans().flat_map(|span| &span.coordinates) {
            pages.entry(coordinate.source_page).or_insert_with(|| {
                coordinate
                    .printed_page
                    .as_deref()
                    .and_then(|printed| printed.parse::<i32>().ok())
                    .map(|printed| printed - coordinate.source_page as i32)
            });
        }
    }
    let mut ranges: Vec<PageRange> = Vec::new();
    for (source_page, printed_page_offset) in pages {
        if let Some(range) = ranges.last_mut() {
            if source_page == range.source_end + 1
                && printed_page_offset == range.printed_page_offset
            {
                range.source_end = source_page;
                continue;
            }
        }
        ranges.push(PageRange {
            source_start: source_page,
            source_end: source_page,
            printed_page_offset,
        });
    }
    ranges
}

fn summarize_page(entries: &[CorpusEntry], edition: &str, source_page: u32) -> Option<PageSummary> {
    let mut page: Option<PageSummary> = None;
    for entry in entries.iter().filter(|entry| entry.edition == edition) {
        for coordinate in entry
            .spans()
            .flat_map(|span| &span.coordinates)
            .filter(|coordinate| coordinate.source_page == source_page)
        {
            let page = page.get_or_insert_with(|| PageSummary {
                edition: edition.to_owned(),
                source_page,
                printed_page: coordinate.printed_page.clone(),
                page_image: coordinate.page_image.clone(),
                entries: Vec::new(),
            });
            let entry_index = page
                .entries
                .iter()
                .position(|summary| summary.id == entry.id)
                .unwrap_or_else(|| {
                    page.entries.push(PageEntrySummary {
                        id: entry.id.clone(),
                        headword: entry
                            .headword
                            .as_ref()
                            .map(|headword| headword.normalized.clone()),
                        review_state: entry.review_state,
                        polygons: Vec::new(),
                    });
                    page.entries.len() - 1
                });
            let page_entry = &mut page.entries[entry_index];
            if !page_entry.polygons.contains(&coordinate.polygon) {
                page_entry.polygons.push(coordinate.polygon.clone());
            }
        }
    }
    page
}

fn render_page_fragment(page: &PageSummary) -> String {
    let image_url = format!("/api/image?path={}", percent_encode(&page.page_image));
    let mut polygons = String::new();
    let mut legend = String::new();
    for (entry_index, entry) in page.entries.iter().enumerate() {
        let hue = entry_index * 360 / page.entries.len().max(1);
        let color = format!("hsl({hue} 70% 38%)");
        let label = html_escape(entry.headword.as_deref().unwrap_or(&entry.id));
        for points in &entry.polygons {
            let points = points
                .iter()
                .map(|point| format!("{},{}", point.x, point.y))
                .collect::<Vec<_>>()
                .join(" ");
            polygons.push_str(&format!(
                r#"<polygon class="page-overlay" data-id="{}" style="fill:{color};fill-opacity:.18;stroke:{color}" points="{points}"><title>{label}</title></polygon>"#,
                html_escape(&entry.id),
            ));
        }
        legend.push_str(&format!(
            r#"<button data-entry="{}" style="--entry-color:{color}">{label} · {}</button>"#,
            html_escape(&entry.id),
            entry.review_state.as_str(),
        ));
    }
    format!(
        r#"<section class="page-canvas"><svg data-page-image="{}"><image href="{}"/>{polygons}</svg><div class="legend"><strong>Entry key · {} entries</strong>{legend}</div></section>"#,
        html_escape(&image_url),
        html_escape(&image_url),
        page.entries.len(),
    )
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

fn load_layered_entries(roots: &[PathBuf]) -> Result<Vec<CorpusEntry>> {
    let mut entries = BTreeMap::new();
    for root in roots {
        let root_entries = load_base_entries(root)?;
        let mut root_ids = std::collections::BTreeSet::new();
        for entry in root_entries {
            if !root_ids.insert(entry.id.clone()) {
                bail!(
                    "duplicate base entry ID `{}` in {}",
                    entry.id,
                    root.display()
                );
            }
            entries.insert(entry.id.clone(), entry);
        }
    }
    Ok(entries.into_values().collect())
}

fn load_layered_edition_entries(roots: &[PathBuf], edition: &str) -> Result<Vec<CorpusEntry>> {
    let mut entries = BTreeMap::new();
    for root in roots {
        let path = root.join(format!("{edition}.jsonl"));
        if !path.is_file() {
            continue;
        }
        let mut root_ids = std::collections::BTreeSet::new();
        for entry in load_entries(&path)? {
            if !root_ids.insert(entry.id.clone()) {
                bail!(
                    "duplicate base entry ID `{}` in {}",
                    entry.id,
                    root.display()
                );
            }
            entries.insert(entry.id.clone(), entry);
        }
    }
    Ok(entries.into_values().collect())
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

fn percent_encode(value: &str) -> String {
    value.bytes().fold(String::new(), |mut encoded, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
        encoded
    })
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
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
#detail.page-detail{display:block;overflow:auto}
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
.page-toolbar{display:flex;gap:.5rem;align-items:center;margin-bottom:.7rem;position:sticky;top:0;z-index:2;background:#eee9df;padding-bottom:.35rem}.page-toolbar select{min-width:0;flex:1}.page-canvas{display:grid;grid-template-columns:minmax(0,1fr) minmax(14rem,20rem);gap:.8rem;align-items:start}
#pageContent{min-height:0}.page-canvas svg{display:block;width:100%;height:auto;background:#ddd}
.page-overlay{cursor:pointer;stroke-width:4}.page-overlay:hover{fill-opacity:.42}
.legend{display:flex;flex-direction:column;gap:.35rem;position:sticky;top:3rem;max-height:calc(100vh - 7rem);overflow:auto}.legend button{border-left:.65rem solid var(--entry-color);text-align:left}
@media(max-width:850px){main{display:block;height:auto}.grid,.page-canvas{grid-template-columns:1fr}#list{max-height:35vh}#detail.page-detail{box-sizing:border-box}.legend{position:static;max-height:none}}
</style><script defer src="https://cdn.jsdelivr.net/npm/htmx.org@2.0.10/dist/htmx.min.js" integrity="sha384-H5SrcfygHmAuTDZphMHqBJLc3FhssKjG7w/CeCpFReSfwBWDTKpkzPP8c+cLsK+V" crossorigin="anonymous"></script></head>
<body><header><strong>Gesenius review</strong>
<a href="/transcriptions" style="color:white">Transcription review</a>
<label>Edition <select id="edition"><option value="">Choose edition…</option></select></label>
<button id="entryMode">Entries</button><button id="pageMode">Pages</button>
<span id="entryFilters"><label>State <select id="state"><option value="">all</option><option>machine</option><option>corrected</option><option>verified</option></select></label>
<label><input id="queue" type="checkbox" checked> review queue</label></span><button id="reload">Reload</button></header>
<main><div id="list"></div><div id="detail"><p>Select an entry.</p></div></main>
<script>
const $=s=>document.querySelector(s), esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
let current=null,mode='entries',pages=[],pageRanges=[];
const entryColor=(index,total)=>`hsl(${Math.round(index*360/Math.max(1,total))} 70% 38%)`;
function setMode(next){mode=next;$('#detail').classList.toggle('page-detail',mode==='pages');$('#entryFilters').classList.toggle('hidden',mode==='pages');$('#entryMode').disabled=mode==='entries';$('#pageMode').disabled=mode==='pages';}
async function loadEditions(){let editions=await (await fetch('/api/editions')).json();$('#edition').innerHTML='<option value="">Choose edition…</option>'+editions.map(edition=>`<option>${esc(edition)}</option>`).join('');}
async function loadList(){let edition=$('#edition').value;if(!edition){$('#list').innerHTML='<p class="item muted">Choose an edition.</p>';return;}let q=new URLSearchParams({edition,state:$('#state').value,queue:$('#queue').checked});let rows=await (await fetch('/api/entries?'+q)).json();
$('#list').innerHTML=rows.map(r=>`<div class="item" data-id="${esc(r.id)}"><span class="hebrew">${esc(r.headword||'—')}</span><br><b>${esc(r.edition)}</b> p. ${esc(r.printed_page)}
<div class="muted">${Math.round(r.confidence*100)}% · ${r.review_state} · ${r.warnings} warnings · Δ ${r.disagreement.toFixed(2)}</div></div>`).join('');
document.querySelectorAll('.item').forEach(x=>x.onclick=()=>loadEntry(x.dataset.id));}
async function openEntry(id){setMode('entries');await loadList();await loadEntry(id);}
const printedPage=page=>page.printed_page_offset===null?'—':String(page.source_page+page.printed_page_offset);
async function loadPages(selectedSource){let edition=$('#edition').value;if(!edition){$('#list').innerHTML='<p class="item muted">Choose an edition.</p>';$('#detail').innerHTML='<p>Choose an edition to browse its pages.</p>';return;}pageRanges=await (await fetch('/api/pages?edition='+encodeURIComponent(edition))).json();pages=pageRanges.flatMap(range=>Array.from({length:range.source_end-range.source_start+1},(_,offset)=>({edition,source_page:range.source_start+offset,printed_page_offset:range.printed_page_offset})));
$('#list').innerHTML=pageRanges.map((range,index)=>`<div class="item" data-range="${index}"><b>printed ${range.printed_page_offset===null?'—':range.source_start+range.printed_page_offset}${range.source_end===range.source_start?'':'–'+(range.printed_page_offset===null?'—':range.source_end+range.printed_page_offset)}</b><br><span class="muted">PDF ${range.source_start}${range.source_end===range.source_start?'':'–'+range.source_end}</span></div>`).join('');document.querySelectorAll('[data-range]').forEach(x=>x.onclick=()=>renderPage(pages.findIndex(page=>page.source_page===pageRanges[Number(x.dataset.range)].source_start)));let index=Math.max(0,pages.findIndex(page=>page.source_page===selectedSource));if(pages.length)await renderPage(index);else $('#detail').innerHTML='<p>No pages available.</p>';}
async function renderPage(index){let page=pages[index],url='/fragments/page?edition='+encodeURIComponent(page.edition)+'&source_page='+page.source_page;
$('#detail').innerHTML=`<div class="page-toolbar"><button id="previousPage" ${index===0?'disabled':''}>← Previous</button><select id="pageSelect">${pages.map((candidate,i)=>`<option value="${i}" ${i===index?'selected':''}>printed ${printedPage(candidate)} · PDF ${candidate.source_page}</option>`).join('')}</select><button id="nextPage" ${index===pages.length-1?'disabled':''}>Next →</button></div><div id="pageContent" hx-get="${esc(url)}" hx-trigger="load" hx-swap="innerHTML"><p class="muted">Loading page…</p></div>`;
$('#previousPage').onclick=()=>renderPage(index-1);$('#nextPage').onclick=()=>renderPage(index+1);$('#pageSelect').onchange=event=>renderPage(Number(event.target.value));htmx.process($('#pageContent'));}
document.body?.addEventListener('htmx:afterSwap',async event=>{if(event.detail.target.id!=='pageContent')return;let svg=event.detail.target.querySelector('svg[data-page-image]');if(svg){let dimensions=await imageSize(svg.dataset.pageImage).catch(()=>null);if(dimensions){svg.setAttribute('viewBox',`0 0 ${dimensions.width} ${dimensions.height}`);let image=svg.querySelector('image');image.setAttribute('width',dimensions.width);image.setAttribute('height',dimensions.height);}else event.detail.target.innerHTML=missingScan(svg.dataset.pageImage);}document.querySelectorAll('.page-overlay').forEach(x=>x.onclick=()=>openEntry(x.dataset.id));document.querySelectorAll('[data-entry]').forEach(x=>x.onclick=()=>openEntry(x.dataset.entry));});
async function loadEntry(id){current=await (await fetch('/api/entries/'+encodeURIComponent(id)+'?edition='+encodeURIComponent($('#edition').value))).json();await render();}
function cps(text){return [...text].map(c=>`${c} U+${c.codePointAt(0).toString(16).toUpperCase().padStart(4,'0')}`).join(' · ')}
function imageSize(src){return new Promise((resolve,reject)=>{let image=new Image();image.onload=()=>resolve({width:image.naturalWidth,height:image.naturalHeight});image.onerror=reject;image.src=src;});}
function renderTextSpan(span){let word=0,content=span.normalized.split(/(\s+)/).map(part=>/^\s+$/.test(part)?esc(part):`<span class="text-word" dir="auto" data-word="${word++}">${esc(part)}</span>`).join('');return `<span class="text-line" data-span="${esc(span.id)}">${content}</span>`;}
function renderStructuredText(headword,blocks){let html=headword?`<h2 class="entry-headword" dir="rtl">${renderTextSpan(headword)}</h2>`:`<h2 class="entry-headword" dir="ltr">${esc(current.id)}</h2>`,currentPage=null;
const renderPart=(kind,spans)=>{if(!spans.length)return '';let content=spans.map(renderTextSpan).join(' ');if(kind==='heading')return `<h3 dir="ltr">${content}</h3>`;if(kind==='paragraph')return `<p dir="ltr">${content}</p>`;return `<div class="structural-block"><div class="block-kind">${esc(kind.replaceAll('_',' '))}</div><p dir="ltr">${content}</p></div>`;};
for(let block of blocks){let partPage=null,partSpans=[];for(let span of block.spans){if(!span.normalized)continue;let spanPage=span.coordinates[0]?.printed_page||null;if(partPage!==null&&spanPage!==partPage){html+=renderPart(block.kind,partSpans);partSpans=[];}if(spanPage!==currentPage){if(spanPage)html+=`<div class="page-break">Page ${esc(spanPage)}</div>`;currentPage=spanPage;}partPage=spanPage;partSpans.push(span);}html+=renderPart(block.kind,partSpans);}return html;}
function missingScan(path){return `<p class="warn">Scan unavailable: <code>${esc(path)}</code></p><p>Restore the generated page image in the local cache, then reload. Entry text and review controls remain available.</p>`;}
async function scanForPage(spans,page,selectedSpan){let imageUrl='/api/image?path='+encodeURIComponent(page.image),dimensions=await imageSize(imageUrl).catch(()=>null);
if(!dimensions)return missingScan(page.image);
return `<svg viewBox="0 0 ${dimensions.width} ${dimensions.height}"><image href="${esc(imageUrl)}" width="${dimensions.width}" height="${dimensions.height}"/>
${spans.flatMap(s=>s.coordinates.filter(c=>c.source_page===page.source).map(c=>`<polygon class="overlay${s.id===selectedSpan?' selected':''}" data-span="${esc(s.id)}" points="${c.polygon.map(p=>p.x+','+p.y).join(' ')}"><title>${esc(s.normalized)}</title></polygon>`)).join('')}</svg>`;}
async function render(){let spans=[...(current.headword?[current.headword]:[]),...current.blocks.flatMap(b=>b.spans)];
let pages=[];for(let span of spans)for(let coordinate of span.coordinates)if(!pages.some(page=>page.source===coordinate.source_page))pages.push({image:coordinate.page_image,source:coordinate.source_page,printed:coordinate.printed_page});
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
const selectText=async(event,line)=>{selectedSpan=line.dataset.span;let span=spans.find(candidate=>candidate.id===selectedSpan),source=span?.coordinates[0]?.source_page,pageIndex=pages.findIndex(page=>page.source===source);if(pageIndex>=0&&pageIndex!==selectedPage){selectedPage=pageIndex;if($('#scanPage'))$('#scanPage').value=String(selectedPage);$('#scanCanvas').innerHTML=await scanForPage(spans,pages[selectedPage],selectedSpan);bindScan();}markSelection(selectedSpan,event.target.closest('.text-word'));};
textLines().forEach(line=>line.onclick=event=>selectText(event,line));bindScan();
if(pages.length>1)$('#scanPage').onchange=async event=>{selectedPage=Number(event.target.value);$('#scanCanvas').innerHTML=await scanForPage(spans,pages[selectedPage],selectedSpan);bindScan();};
$('#viewPage').onclick=async()=>{$('#edition').value=current.edition;setMode('pages');await loadPages(pages[selectedPage].source);};
$('#save').onclick=save;}
async function save(){let message=$('#message');try{let entry=JSON.parse($('#editor').value);let response=await fetch('/api/entries/'+encodeURIComponent(current.id),{method:'PATCH',headers:{'Content-Type':'application/json'},
body:JSON.stringify({base_revision:current.revision,reviewer:$('#reviewer').value,review_state:$('#reviewState').value,entry})});
let result=await response.json();if(!response.ok)throw Error(result.error);current=result.replacement;message.textContent='Saved.';await loadList();await render();}catch(e){message.className='warn';message.textContent=e.message;}}
$('#entryMode').onclick=async()=>{setMode('entries');await loadList();$('#detail').innerHTML='<p>Select an entry.</p>';};
$('#pageMode').onclick=async()=>{setMode('pages');await loadPages();};$('#reload').onclick=()=>mode==='entries'?loadList():loadPages();
$('#edition').onchange=()=>mode==='entries'?loadList():loadPages();$('#state').onchange=loadList;$('#queue').onchange=loadList;loadEditions().then(()=>{setMode(location.hash==='#page-view-smoke-test'?'pages':'entries');$('#list').innerHTML='<p class="item muted">Choose an edition.</p>';$('#detail').innerHTML='<p>Choose an edition to begin.</p>';});
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::{html_escape, percent_decode, percent_encode, query_parameter, REVIEW_UI};

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
    fn page_fragment_values_are_safe_for_urls_and_html() {
        assert_eq!(percent_encode("cache/a b&c.png"), "cache%2Fa%20b%26c.png");
        assert_eq!(
            html_escape("<Aleph & \"Beth\">"),
            "&lt;Aleph &amp; &quot;Beth&quot;&gt;"
        );
    }

    #[test]
    fn page_review_loads_selected_detail_with_htmx() {
        assert!(REVIEW_UI.contains("htmx.org@2.0.10"));
        assert!(REVIEW_UI.contains(r#"hx-get="${esc(url)}""#));
        assert!(REVIEW_UI.contains("/fragments/page?edition="));
        assert!(REVIEW_UI.contains("Choose edition…"));
        assert!(REVIEW_UI.contains("pageRanges.flatMap"));
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
        assert!(REVIEW_UI.contains("page.source===coordinate.source_page"));
        assert!(REVIEW_UI.contains("c.source_page===page.source"));
    }
}
