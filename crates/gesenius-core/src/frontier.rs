//! Import of frontier-model page transcriptions into the machine corpus.
//!
//! `tool/frontier-to-alto.py` turns each frontier page record into the
//! pipeline's in-memory ALTO page, with line geometry measured from the page
//! raster. This module runs those pages through the same entry segmentation
//! as the OCR pipeline, writes a `parsed.json` assignment artifact per page
//! under a content-addressed run directory, and merges the entries into the
//! machine corpus, replacing every entry that touches an imported page.

use crate::alto::{parse_transcribed_entries_continuing, AltoPage, EngineIdentity, ParseContext};
use crate::corpus_io::{load_entries, write_entries};
use crate::pipeline::{content_hash, merge_parsed_pages, printed_page, write_page_parse};
use crate::source::{sha256_file, verify_source, SourceCatalogue};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Schema tag written by `tool/frontier-to-alto.py`.
pub const FRONTIER_ALTO_SCHEMA: &str = "gesenius-frontier-alto/1";

/// One converted frontier page.
#[derive(Debug, Clone, Deserialize)]
pub struct FrontierPage {
    /// Must equal [`FRONTIER_ALTO_SCHEMA`].
    pub schema: String,
    /// Edition identifier.
    pub edition: String,
    /// One-based PDF page.
    pub source_page: u32,
    /// Relative path of the raster the geometry refers to.
    pub raster: String,
    /// SHA-256 of that raster.
    pub raster_sha256: String,
    /// Frontier record the page was converted from.
    pub transcription: Option<String>,
    /// Frontier pass that produced the canonical text.
    pub pass: u32,
    /// Driver used to call the model.
    pub engine: String,
    /// Model identifier.
    pub model: String,
    /// SHA-256 over the prompts used.
    pub prompt_digest: String,
    /// Canonical reading.
    pub page: AltoPage,
    /// Earlier readings on the same geometry.
    #[serde(default)]
    pub hypotheses: Vec<FrontierHypothesis>,
}

/// An earlier frontier reading kept as an OCR hypothesis.
#[derive(Debug, Clone, Deserialize)]
pub struct FrontierHypothesis {
    /// Pass label such as `pass1`.
    pub label: String,
    /// The reading.
    pub page: AltoPage,
}

/// Options for [`import_frontier`].
pub struct FrontierImportOptions<'a> {
    /// Edition identifier.
    pub edition: &'a str,
    /// Converted page files.
    pub pages: &'a [PathBuf],
    /// Source catalogue.
    pub catalogue_path: &'a Path,
    /// Content-addressed cache root.
    pub cache_root: &'a Path,
    /// Machine corpus directory.
    pub corpus_root: &'a Path,
}

/// Result of an import.
#[derive(Debug, Clone, Serialize)]
pub struct FrontierImportResult {
    /// Content address of the import run.
    pub run_id: String,
    /// Imported PDF pages.
    pub pages: Vec<u32>,
    /// Entries emitted for those pages.
    pub entries: usize,
    /// Entries with a headword.
    pub entries_with_headword: usize,
    /// Lines left unparsed, such as running heads.
    pub unparsed_lines: usize,
    /// Updated machine corpus.
    pub corpus_path: PathBuf,
    /// Directory holding the `parsed.json` artifacts.
    pub run_path: PathBuf,
}

fn identity(engine: &str, pass: String, model: &str, prompt_digest: &str) -> EngineIdentity {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update([0]);
    hasher.update(prompt_digest.as_bytes());
    EngineIdentity {
        engine: format!("{engine}/{pass}"),
        version: pass,
        model: model.to_owned(),
        model_hash: hex::encode(hasher.finalize()),
    }
}

/// Parses converted frontier pages into entries and merges them into the corpus.
///
/// Consecutive imported pages carry an unfinished entry across the page
/// break. An entry that began on a page outside the import is not continued,
/// so the first entry of such a page may lack its headword.
pub fn import_frontier(options: &FrontierImportOptions<'_>) -> Result<FrontierImportResult> {
    if options.pages.is_empty() {
        bail!("no frontier pages given");
    }
    let catalogue = SourceCatalogue::load(options.catalogue_path)?;
    let source = catalogue.edition(options.edition)?;
    let verified = verify_source(source, options.cache_root)?;

    let mut pages = Vec::new();
    let mut digests = Vec::new();
    for path in options.pages {
        let page: FrontierPage = serde_json::from_slice(
            &fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
        )
        .with_context(|| format!("invalid frontier page {}", path.display()))?;
        if page.schema != FRONTIER_ALTO_SCHEMA {
            bail!("{} has schema `{}`", path.display(), page.schema);
        }
        if page.edition != options.edition {
            bail!("{} is edition `{}`", path.display(), page.edition);
        }
        digests.push(sha256_file(path)?);
        pages.push(page);
    }
    pages.sort_by_key(|page| page.source_page);
    if pages.windows(2).any(|pair| pair[0].source_page == pair[1].source_page) {
        bail!("a PDF page was given more than once");
    }
    digests.sort();
    let mut parts = vec!["frontier-import-v1", verified.sha256.as_str()];
    parts.extend(digests.iter().map(String::as_str));
    let run_id = content_hash(&parts);
    let run_path = options.cache_root.join("runs").join(&run_id).join(options.edition);

    let corpus_path = options.corpus_root.join(format!("{}.jsonl", options.edition));
    let base_entries = if corpus_path.exists() {
        load_entries(&corpus_path)?
    } else {
        Vec::new()
    };

    let mut parsed_pages = Vec::new();
    let mut continuation = None;
    let mut previous_page: Option<u32> = None;
    for page in &pages {
        let canonical = identity(
            &page.engine,
            format!("pass{}", page.pass),
            &page.model,
            &page.prompt_digest,
        );
        let earlier: Vec<_> = page
            .hypotheses
            .iter()
            .map(|h| (&h.page, identity(&page.engine, h.label.clone(), &page.model, &page.prompt_digest)))
            .collect();
        let mut hypotheses = vec![(&page.page, &canonical)];
        hypotheses.extend(earlier.iter().map(|(p, i)| (*p, i)));

        let (printed, front_matter) = printed_page(source, page.source_page);
        let transform_id = format!("frontier-raster-sha256-{}", page.raster_sha256);
        let context = ParseContext {
            edition: options.edition,
            printed_page: &printed,
            source_page: page.source_page,
            source_sha256: &verified.sha256,
            scan_id: &source.scan_id,
            pipeline_run: &run_id,
            page_image: &page.raster,
            transform_id: &transform_id,
            front_matter,
        };
        let continued = if previous_page.is_some_and(|p| p + 1 == page.source_page) {
            continuation.take()
        } else {
            None
        };
        let parsed =
            parse_transcribed_entries_continuing(&page.page, &hypotheses, &context, continued);
        let page_path = run_path.join(format!("page-{:04}", page.source_page));
        fs::create_dir_all(&page_path)?;
        write_page_parse(&page_path, &parsed)?;
        continuation = parsed.entries.last().cloned();
        previous_page = Some(page.source_page);
        parsed_pages.push(parsed);
    }

    let selected: BTreeSet<u32> = pages.iter().map(|page| page.source_page).collect();
    let entries = merge_parsed_pages(&base_entries, &selected, &parsed_pages);
    write_entries(&corpus_path, &entries)?;

    // A continued entry is emitted by both pages; count it once.
    let mut ids = BTreeSet::new();
    let mut with_headword = 0;
    for entry in parsed_pages.iter().flat_map(|p| p.entries.iter()) {
        if ids.insert(entry.id.clone()) && entry.headword.is_some() {
            with_headword += 1;
        }
    }
    let unparsed_lines = parsed_pages
        .iter()
        .flat_map(|p| p.assignments.iter())
        .filter(|(_, _, a)| matches!(a, crate::alto::LineAssignment::Unparsed))
        .count();
    Ok(FrontierImportResult {
        run_id,
        pages: selected.into_iter().collect(),
        entries: ids.len(),
        entries_with_headword: with_headword,
        unparsed_lines,
        corpus_path,
        run_path,
    })
}
