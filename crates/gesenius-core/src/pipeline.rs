//! Resumable content-addressed raster, preprocessing, OCR, and parsing pipeline.

use crate::alto::{
    announced_line_languages, classify_word_languages, detected_word_language,
    fuse_multilingual_words, join_words, parse_alto, parse_entries_with_hypotheses_continuing,
    printed_label_languages, select_script_trial, word_confidence, word_matches_language,
    write_alto, AltoLine, AltoPage, AltoRegion, AltoWord, EngineIdentity, LineAssignment,
    ParseContext, ParsedPage, ScriptTrial, WordScriptContext,
};
use crate::corpus_io::{load_entries, write_entries};
use crate::metrics::{normalized_disagreement, polygon_iou};
use crate::model::{CorpusEntry, Point};
use crate::source::{sha256_file, verify_source, SourceCatalogue, SourceRecord};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use unicode_normalization::char::canonical_combining_class;

/// Default pipeline configuration.
pub const DEFAULT_PIPELINE_CONFIG: &str = "pipeline.toml";

/// Reproducible pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineSettings {
    /// Lossless raster resolution.
    pub raster_dpi: u32,
    /// Preprocessing settings.
    pub preprocessing: PreprocessingSettings,
    /// Tesseract baseline settings.
    pub tesseract: TesseractSettings,
    /// Embedded PDF text-layer settings.
    #[serde(default)]
    pub pdf_text: PdfTextSettings,
    /// Kraken primary recognizer settings.
    pub kraken: KrakenSettings,
    /// Confidence below which spans enter the review queue.
    pub review_confidence_threshold: f32,
    /// Normalized character disagreement threshold for review.
    pub disagreement_threshold: f32,
}

impl PipelineSettings {
    /// Loads TOML configuration.
    pub fn load(path: &Path) -> Result<Self> {
        let input = fs::read_to_string(path)
            .with_context(|| format!("failed to read pipeline settings {}", path.display()))?;
        let settings: Self = toml::from_str(&input)
            .with_context(|| format!("invalid pipeline settings {}", path.display()))?;
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&self) -> Result<()> {
        if !(150..=1200).contains(&self.raster_dpi) {
            bail!("raster_dpi must be between 150 and 1200");
        }
        if !(0.0..=1.0).contains(&self.review_confidence_threshold)
            || !(0.0..=1.0).contains(&self.disagreement_threshold)
        {
            bail!("review thresholds must be between 0 and 1");
        }
        if self.tesseract.primary_languages.is_empty() {
            bail!("at least one primary Tesseract language is required");
        }
        if self.tesseract.multilingual_languages.is_empty() {
            bail!("at least one multilingual Tesseract language is required");
        }
        if !(1..=13).contains(&self.tesseract.word_page_segmentation_mode) {
            bail!("word_page_segmentation_mode must be between 1 and 13");
        }
        if !(1..=13).contains(&self.tesseract.word_fallback_page_segmentation_mode) {
            bail!("word_fallback_page_segmentation_mode must be between 1 and 13");
        }
        if self
            .tesseract
            .word_additional_page_segmentation_modes
            .iter()
            .any(|mode| !(1..=13).contains(mode))
        {
            bail!("word_additional_page_segmentation_modes must be between 1 and 13");
        }
        if !(1..=13).contains(&self.tesseract.block_page_segmentation_mode) {
            bail!("block_page_segmentation_mode must be between 1 and 13");
        }
        if self.tesseract.block_padding_pixels > 100 {
            bail!("block_padding_pixels must not exceed 100");
        }
        if !(100..=400).contains(&self.tesseract.word_scale_percent) {
            bail!("word_scale_percent must be between 100 and 400");
        }
        if self.tesseract.word_padding_pixels > 100 {
            bail!("word_padding_pixels must not exceed 100");
        }
        if !(0.0..=1.0).contains(&self.tesseract.roman_word_refinement_confidence) {
            bail!("roman_word_refinement_confidence must be between 0 and 1");
        }
        if !(100..=800).contains(&self.tesseract.roman_word_scale_percent) {
            bail!("roman_word_scale_percent must be between 100 and 800");
        }
        if !(1..=99).contains(&self.tesseract.roman_word_threshold_percent) {
            bail!("roman_word_threshold_percent must be between 1 and 99");
        }
        if self.tesseract.roman_word_page_segmentation_modes.is_empty()
            || self
                .tesseract
                .roman_word_page_segmentation_modes
                .iter()
                .any(|mode| !(1..=13).contains(mode))
        {
            bail!("roman_word_page_segmentation_modes must contain modes between 1 and 13");
        }
        if self.pdf_text.enabled
            && (self.preprocessing.crop.is_some() || self.preprocessing.deskew_degrees != 0.0)
        {
            bail!("pdf_text requires geometry-preserving preprocessing");
        }
        Ok(())
    }
}

/// Use an embedded, word-positioned PDF text layer as an OCR witness.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PdfTextSettings {
    /// Extract and align the source PDF's hidden text layer.
    pub enabled: bool,
}

/// Photometric and explicit geometric transform settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreprocessingSettings {
    /// Convert to grayscale.
    pub grayscale: bool,
    /// ImageMagick contrast stretch argument.
    pub contrast_stretch: String,
    /// Explicit clockwise deskew rotation. Zero keeps source geometry unchanged.
    pub deskew_degrees: f32,
    /// Optional crop `[width, height, x, y]`.
    #[serde(default)]
    pub crop: Option<[u32; 4]>,
}

/// Tesseract invocation and immutable model identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TesseractSettings {
    /// Dominant-language models used for layout and English recognition.
    pub primary_languages: Vec<String>,
    /// Models used in a separate pass to recover embedded foreign scripts.
    pub multilingual_languages: Vec<String>,
    /// Page segmentation mode.
    pub page_segmentation_mode: u8,
    /// Re-recognize each multi-line layout block independently.
    #[serde(default = "default_block_refinement_enabled")]
    pub block_refinement_enabled: bool,
    /// Segmentation mode used for one cropped layout block.
    #[serde(default = "default_block_page_segmentation_mode")]
    pub block_page_segmentation_mode: u8,
    /// Source-image padding retained around a layout block.
    #[serde(default = "default_block_padding_pixels")]
    pub block_padding_pixels: u32,
    /// Segmentation mode used after cropping one detected foreign word.
    #[serde(default = "default_word_page_segmentation_mode")]
    pub word_page_segmentation_mode: u8,
    /// Segmentation mode retried when the primary mode reads a crop as empty.
    #[serde(default = "default_word_fallback_page_segmentation_mode")]
    pub word_fallback_page_segmentation_mode: u8,
    /// Further segmentation modes compared for isolated word crops.
    #[serde(default)]
    pub word_additional_page_segmentation_modes: Vec<u8>,
    /// Percentage enlargement applied to isolated word crops.
    #[serde(default = "default_word_scale_percent")]
    pub word_scale_percent: u16,
    /// Source-image padding retained around each detected word box.
    #[serde(default = "default_word_padding_pixels")]
    pub word_padding_pixels: u32,
    /// Re-read Roman tokens below this confidence when the PDF layer differs.
    #[serde(default = "default_roman_word_refinement_confidence")]
    pub roman_word_refinement_confidence: f32,
    /// Percentage enlargement applied to isolated Roman word crops.
    #[serde(default = "default_roman_word_scale_percent")]
    pub roman_word_scale_percent: u16,
    /// Global threshold used as a second Roman crop view.
    #[serde(default = "default_roman_word_threshold_percent")]
    pub roman_word_threshold_percent: u8,
    /// Segmentation modes compared for isolated Roman word crops.
    #[serde(default = "default_roman_word_page_segmentation_modes")]
    pub roman_word_page_segmentation_modes: Vec<u8>,
    /// Immutable Nix/tessdata model identity recorded in provenance.
    pub model_identity: String,
}

const fn default_word_page_segmentation_mode() -> u8 {
    8
}

const fn default_word_fallback_page_segmentation_mode() -> u8 {
    8
}

const fn default_block_refinement_enabled() -> bool {
    true
}

const fn default_block_page_segmentation_mode() -> u8 {
    6
}

const fn default_block_padding_pixels() -> u32 {
    8
}

const fn default_word_scale_percent() -> u16 {
    200
}

const fn default_word_padding_pixels() -> u32 {
    4
}

const fn default_roman_word_refinement_confidence() -> f32 {
    0.85
}

const fn default_roman_word_scale_percent() -> u16 {
    400
}

const fn default_roman_word_threshold_percent() -> u8 {
    72
}

fn default_roman_word_page_segmentation_modes() -> Vec<u8> {
    vec![6, 8]
}

/// Kraken invocation and trainable model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KrakenSettings {
    /// Run Kraken and require a model.
    pub enabled: bool,
    /// Local model path.
    pub model_path: PathBuf,
    /// Exact model SHA-256.
    pub model_sha256: String,
    /// Segmentation model or `default`.
    pub segmentation_model: String,
}

/// CLI-level pipeline paths and selection.
pub struct RunOptions<'a> {
    /// Edition ID.
    pub edition: &'a str,
    /// Explicit one-based PDF pages.
    pub pages: &'a [u32],
    /// Source catalogue.
    pub catalogue_path: &'a Path,
    /// Pipeline configuration.
    pub settings_path: &'a Path,
    /// Ignored content-addressed cache.
    pub cache_root: &'a Path,
    /// Authoritative machine corpus directory.
    pub corpus_root: &'a Path,
    /// Pipeline commit identity.
    pub pipeline_commit: &'a str,
}

/// Result of a pipeline invocation.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// Content-addressed run ID.
    pub run_id: String,
    /// Pages requested.
    pub pages: Vec<u32>,
    /// Total entries emitted for those pages.
    pub entries: usize,
    /// Lines explicitly left unparsed.
    pub unparsed_lines: usize,
    /// Updated canonical machine JSONL path.
    pub corpus_path: PathBuf,
    /// Run artifact directory.
    pub run_path: PathBuf,
}

/// A human-readable status update emitted while a pipeline run is in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunProgress {
    /// One-based position of the current page, or zero during run-wide setup.
    pub page_index: usize,
    /// Number of pages requested by the run.
    pub page_count: usize,
    /// PDF page currently being processed, when the update is page-specific.
    pub page_number: Option<u32>,
    /// Short description of the work about to be performed or just completed.
    pub message: String,
}

/// Exact preprocessing operations used on a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformManifest {
    /// Input raster path.
    pub source_image: String,
    /// Input dimensions.
    pub source_dimensions: [u32; 2],
    /// Output dimensions.
    pub output_dimensions: [u32; 2],
    /// Operations in application order.
    pub operations: Vec<TransformOperation>,
    /// Full reproducible subprocess arguments.
    pub command: Vec<String>,
}

/// Recorded image operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum TransformOperation {
    /// No geometric transform.
    Identity,
    /// Grayscale conversion.
    Grayscale,
    /// Contrast stretch.
    ContrastStretch {
        /// ImageMagick geometry argument.
        geometry: String,
    },
    /// Explicit rotation with white background.
    Rotate {
        /// Clockwise rotation in degrees.
        degrees: f32,
    },
    /// Explicit crop rectangle.
    Crop {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Left coordinate.
        x: u32,
        /// Top coordinate.
        y: u32,
    },
}

/// Stage receipt used to resume identical work.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StageReceipt {
    stage: String,
    input_hash: String,
    command: Vec<String>,
    outputs: Vec<String>,
    completed_at: DateTime<Utc>,
}

/// Runs selected pages through every stage and atomically merges their entries.
pub fn run(options: &RunOptions<'_>) -> Result<RunResult> {
    run_with_progress(options, |_| {})
}

/// Runs selected pages and reports status without coupling the core to a logger.
pub fn run_with_progress(
    options: &RunOptions<'_>,
    mut report: impl FnMut(RunProgress),
) -> Result<RunResult> {
    if options.pages.is_empty() {
        bail!("page selection is empty");
    }
    let page_count = options.pages.len();
    report_progress(
        &mut report,
        0,
        page_count,
        None,
        "loading configuration and verifying source",
    );
    let settings = PipelineSettings::load(options.settings_path)?;
    let catalogue = SourceCatalogue::load(options.catalogue_path)?;
    let source = catalogue.edition(options.edition)?;
    let verified = verify_source(source, options.cache_root)?;
    verify_model(&settings.kraken)?;

    report_progress(
        &mut report,
        0,
        page_count,
        None,
        "checking OCR engines and model identities",
    );
    let tesseract_version = command_version("tesseract")?;
    let primary_tesseract_models_sha256 =
        tesseract_model_hash(&settings.tesseract.primary_languages)?;
    let multilingual_tesseract_models_sha256 =
        tesseract_model_hash(&settings.tesseract.multilingual_languages)?;
    let pdf_text_version = settings
        .pdf_text
        .enabled
        .then(|| command_version_with_flag("pdftotext", "-v"))
        .transpose()?;
    let kraken_version = if settings.kraken.enabled {
        Some(command_version("kraken")?)
    } else {
        None
    };
    let run_id = content_hash(&[
        &verified.sha256,
        &fs::read_to_string(options.settings_path)?,
        options.pipeline_commit,
        &tesseract_version,
        &primary_tesseract_models_sha256,
        &multilingual_tesseract_models_sha256,
        pdf_text_version.as_deref().unwrap_or("pdf-text-disabled"),
        kraken_version.as_deref().unwrap_or("kraken-disabled"),
    ]);
    let run_path = options
        .cache_root
        .join("runs")
        .join(&run_id)
        .join(options.edition);
    fs::create_dir_all(&run_path)?;

    let primary_tesseract_identity = EngineIdentity {
        engine: "tesseract".to_owned(),
        version: tesseract_version.clone(),
        model: format!(
            "{}:{}:page-psm{}",
            settings.tesseract.model_identity,
            settings.tesseract.primary_languages.join("+"),
            settings.tesseract.page_segmentation_mode,
        ),
        model_hash: primary_tesseract_models_sha256.clone(),
    };
    let block_tesseract_identity = EngineIdentity {
        engine: "tesseract".to_owned(),
        version: tesseract_version.clone(),
        model: format!(
            "{}:{}:block-psm{}",
            settings.tesseract.model_identity,
            settings.tesseract.primary_languages.join("+"),
            settings.tesseract.block_page_segmentation_mode,
        ),
        model_hash: primary_tesseract_models_sha256,
    };
    let multilingual_tesseract_identity = EngineIdentity {
        engine: "tesseract".to_owned(),
        version: tesseract_version,
        model: format!(
            "{}:{}",
            settings.tesseract.model_identity,
            settings.tesseract.multilingual_languages.join("+")
        ),
        model_hash: multilingual_tesseract_models_sha256,
    };
    let pdf_text_identity = pdf_text_version.map(|version| EngineIdentity {
        engine: "pdftotext".to_owned(),
        version,
        model: "embedded-pdf-text-layer".to_owned(),
        model_hash: verified.sha256.clone(),
    });
    let kraken_identity = if settings.kraken.enabled {
        Some(EngineIdentity {
            engine: "kraken".to_owned(),
            version: kraken_version.context("missing Kraken version")?,
            model: settings.kraken.model_path.display().to_string(),
            model_hash: settings.kraken.model_sha256.clone(),
        })
    } else {
        None
    };

    let corpus_path = options
        .corpus_root
        .join(format!("{}.jsonl", options.edition));
    let base_entries = if corpus_path.exists() {
        load_entries(&corpus_path)?
    } else {
        Vec::new()
    };
    let mut parsed_pages = Vec::new();
    let mut previous_page_number = None;
    let mut continuation = None;
    for (page_offset, page_number) in options.pages.iter().enumerate() {
        let page_index = page_offset + 1;
        let mut report_page = |message: &str| {
            report_progress(
                &mut report,
                page_index,
                page_count,
                Some(*page_number),
                message,
            );
        };
        if *page_number == 0 {
            bail!("PDF page numbers are one-based");
        }
        if source
            .page_count
            .is_some_and(|page_count| *page_number > page_count)
        {
            bail!(
                "page {page_number} exceeds registered page count {}",
                source.page_count.unwrap_or_default()
            );
        }
        let page_path = run_path.join(format!("page-{page_number:04}"));
        fs::create_dir_all(&page_path)?;
        report_page("rasterizing PDF page");
        let original = rasterize(
            &verified.path,
            *page_number,
            settings.raster_dpi,
            &page_path,
            &run_id,
        )?;
        report_page("preprocessing page image");
        let (processed, transform_id) =
            preprocess(&original, &page_path, &settings.preprocessing, &run_id)?;
        report_page("running primary Tesseract OCR");
        let primary_tesseract_alto = recognize_tesseract(
            &processed,
            &page_path,
            &settings.tesseract,
            &settings.tesseract.primary_languages,
            "primary",
            settings.raster_dpi,
            &run_id,
        )?;
        report_page("running multilingual Tesseract OCR");
        let multilingual_tesseract_alto = recognize_tesseract(
            &processed,
            &page_path,
            &settings.tesseract,
            &settings.tesseract.multilingual_languages,
            "multilingual",
            settings.raster_dpi,
            &run_id,
        )?;
        let primary_layout_page = parse_alto(&fs::read_to_string(&primary_tesseract_alto)?)?;
        let pdf_text_page = if settings.pdf_text.enabled {
            report_page("extracting embedded PDF text layer");
            Some(extract_pdf_text_layer(
                &verified.path,
                *page_number,
                &page_path,
                primary_layout_page.width,
                primary_layout_page.height,
                &run_id,
            )?)
        } else {
            None
        };
        let primary_tesseract_page = if settings.tesseract.block_refinement_enabled {
            report_page("re-reading primary OCR layout blocks");
            recognize_tesseract_blocks(
                &processed,
                &page_path,
                &settings.tesseract,
                &primary_layout_page,
                settings.raster_dpi,
                &run_id,
            )?
        } else {
            primary_layout_page.clone()
        };
        let multilingual_tesseract_page =
            parse_alto(&fs::read_to_string(&multilingual_tesseract_alto)?)?;
        // Fusion first: the English-primary pass owns the layout and supplies
        // the geometry of every word, including the ones it could only read as
        // Latin rubbish. Arbitrating scripts on the fused page therefore also
        // reaches words the multilingual pass never recognized as foreign.
        let fused_tesseract_page =
            fuse_multilingual_words(&primary_tesseract_page, &multilingual_tesseract_page);
        fs::write(
            page_path.join("tesseract-fused.alto.xml"),
            write_alto(&fused_tesseract_page, &relative_or_absolute(&processed)),
        )?;
        let classified_tesseract_page = classify_word_languages(
            &fused_tesseract_page,
            &settings.tesseract.multilingual_languages,
        );
        report_page("refining detected word crops");
        let word_tesseract_page = recognize_tesseract_words(
            &processed,
            &page_path,
            &settings.tesseract,
            &classified_tesseract_page,
            settings.raster_dpi,
            &run_id,
            pdf_text_page.as_ref(),
        )?;
        let kraken_page = if kraken_identity.is_some() {
            report_page("running Kraken OCR");
            let kraken_alto = recognize_kraken(&processed, &page_path, &settings.kraken, &run_id)?;
            Some(parse_alto(&fs::read_to_string(&kraken_alto)?)?)
        } else {
            None
        };
        report_page("parsing OCR into lexicon entries");
        let canonical_page = kraken_page.as_ref().unwrap_or(&word_tesseract_page);
        let mut hypotheses = Vec::new();
        if let (Some(page), Some(identity)) = (kraken_page.as_ref(), kraken_identity.as_ref()) {
            hypotheses.push((page, identity));
        }
        hypotheses.extend([
            (&primary_tesseract_page, &block_tesseract_identity),
            (&primary_layout_page, &primary_tesseract_identity),
            (
                &multilingual_tesseract_page,
                &multilingual_tesseract_identity,
            ),
        ]);
        if let (Some(page), Some(identity)) = (pdf_text_page.as_ref(), pdf_text_identity.as_ref()) {
            hypotheses.push((page, identity));
        }
        let (printed_page, front_matter) = printed_page(source, *page_number);
        let page_image = relative_or_absolute(&processed);
        let context = ParseContext {
            edition: options.edition,
            printed_page: &printed_page,
            source_page: *page_number,
            source_sha256: &verified.sha256,
            scan_id: &source.scan_id,
            pipeline_run: &run_id,
            page_image: &page_image,
            transform_id: &transform_id,
            front_matter,
        };
        let continued_entry =
            if previous_page_number.is_some_and(|previous| previous + 1 == *page_number) {
                continuation.take()
            } else {
                None
            };
        let parsed = parse_entries_with_hypotheses_continuing(
            canonical_page,
            &hypotheses,
            &context,
            continued_entry,
        );
        write_page_parse(&page_path, &parsed)?;
        continuation = parsed.entries.last().cloned();
        previous_page_number = Some(*page_number);
        parsed_pages.push(parsed);
        let completed_pages = options.pages[..page_index].iter().copied().collect();
        let entries = merge_parsed_pages(&base_entries, &completed_pages, &parsed_pages);
        write_entries(&corpus_path, &entries)?;
        report_page("page complete");
    }

    report_progress(
        &mut report,
        0,
        page_count,
        None,
        "merging parsed entries into the machine corpus",
    );
    let selected_pages: BTreeSet<u32> = options.pages.iter().copied().collect();
    let entries = merge_parsed_pages(&base_entries, &selected_pages, &parsed_pages);
    write_entries(&corpus_path, &entries)?;

    let unparsed_lines = parsed_pages
        .iter()
        .flat_map(|page| page.assignments.iter())
        .filter(|(_, _, assignment)| matches!(assignment, LineAssignment::Unparsed))
        .count();
    report_progress(&mut report, 0, page_count, None, "run complete");
    Ok(RunResult {
        run_id,
        pages: options.pages.to_vec(),
        entries: parsed_pages.iter().map(|page| page.entries.len()).sum(),
        unparsed_lines,
        corpus_path,
        run_path,
    })
}

fn merge_parsed_pages(
    base_entries: &[CorpusEntry],
    selected_pages: &BTreeSet<u32>,
    parsed_pages: &[ParsedPage],
) -> Vec<CorpusEntry> {
    let mut entries = base_entries.to_vec();
    entries.retain(|entry| !should_replace_entry(entry, selected_pages));
    for parsed_entry in parsed_pages
        .iter()
        .flat_map(|page| page.entries.iter().cloned())
    {
        entries.retain(|entry| entry.id != parsed_entry.id);
        entries.push(parsed_entry);
    }
    entries
}

fn should_replace_entry(entry: &CorpusEntry, selected_pages: &BTreeSet<u32>) -> bool {
    entry_source_page(entry).is_some_and(|source_page| selected_pages.contains(&source_page))
}

fn entry_source_page(entry: &CorpusEntry) -> Option<u32> {
    entry
        .spans()
        .flat_map(|span| span.coordinates.iter())
        .map(|coordinate| coordinate.source_page)
        .min()
}

fn report_progress(
    report: &mut impl FnMut(RunProgress),
    page_index: usize,
    page_count: usize,
    page_number: Option<u32>,
    message: &str,
) {
    report(RunProgress {
        page_index,
        page_count,
        page_number,
        message: message.to_owned(),
    });
}

/// Parses `1,3-5,9` into a sorted, de-duplicated one-based page list.
pub fn parse_page_spec(specification: &str) -> Result<Vec<u32>> {
    let mut pages = BTreeSet::new();
    for component in specification.split(',').map(str::trim) {
        if component.is_empty() {
            bail!("empty component in page selection");
        }
        if let Some((start, end)) = component.split_once('-') {
            let start = start.parse::<u32>().context("invalid page range start")?;
            let end = end.parse::<u32>().context("invalid page range end")?;
            if start == 0 || start > end {
                bail!("invalid page range `{component}`");
            }
            pages.extend(start..=end);
        } else {
            let page = component.parse::<u32>().context("invalid page number")?;
            if page == 0 {
                bail!("PDF page numbers are one-based");
            }
            pages.insert(page);
        }
    }
    Ok(pages.into_iter().collect())
}

fn rasterize(
    source: &Path,
    page: u32,
    dpi: u32,
    page_path: &Path,
    run_id: &str,
) -> Result<PathBuf> {
    let output = page_path.join("original.png");
    let stem = page_path.join("original");
    let arguments = vec![
        "-f".to_owned(),
        page.to_string(),
        "-l".to_owned(),
        page.to_string(),
        "-r".to_owned(),
        dpi.to_string(),
        "-png".to_owned(),
        "-singlefile".to_owned(),
        source.display().to_string(),
        stem.display().to_string(),
    ];
    run_resumable_command(
        "rasterize",
        &content_hash(&[run_id, &page.to_string(), &dpi.to_string()]),
        "pdftoppm",
        &arguments,
        std::slice::from_ref(&output),
        &page_path.join("rasterize.stage.json"),
    )?;
    Ok(output)
}

fn extract_pdf_text_layer(
    pdf: &Path,
    pdf_page: u32,
    page_path: &Path,
    width: u32,
    height: u32,
    run_id: &str,
) -> Result<AltoPage> {
    let xhtml_path = page_path.join("pdf-text-layer.xhtml");
    let alto_path = page_path.join("pdf-text-layer.alto.xml");
    let receipt_path = page_path.join("pdf-text-layer.stage.json");
    let input_hash = content_hash(&[
        run_id,
        &pdf_page.to_string(),
        &width.to_string(),
        &height.to_string(),
        "pdf-text-layer-v1",
    ]);
    let outputs = [xhtml_path.clone(), alto_path.clone()];
    if stage_is_current(&receipt_path, &input_hash, &outputs)? {
        return parse_alto(&fs::read_to_string(alto_path)?);
    }
    let arguments = vec![
        "-f".to_owned(),
        pdf_page.to_string(),
        "-l".to_owned(),
        pdf_page.to_string(),
        "-bbox-layout".to_owned(),
        pdf.display().to_string(),
        xhtml_path.display().to_string(),
    ];
    execute("pdftotext", &arguments)?;
    if !xhtml_path.is_file() {
        bail!("pdftotext succeeded without producing positioned XHTML");
    }
    let page = parse_pdf_text_layer(&fs::read_to_string(&xhtml_path)?, width, height)?;
    fs::write(&alto_path, write_alto(&page, &relative_or_absolute(pdf)))?;
    write_receipt(
        &receipt_path,
        "pdf-text-layer",
        &input_hash,
        "pdftotext",
        &arguments,
        &outputs,
    )?;
    Ok(page)
}

fn parse_pdf_text_layer(xhtml: &str, width: u32, height: u32) -> Result<AltoPage> {
    let document = roxmltree::Document::parse_with_options(
        xhtml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        },
    )
    .context("invalid pdftotext XHTML")?;
    let page = document
        .descendants()
        .find(|node| node.has_tag_name("page"))
        .context("pdftotext XHTML has no page")?;
    let source_width = pdf_text_coordinate(&page, "width")?;
    let source_height = pdf_text_coordinate(&page, "height")?;
    if source_width <= 0.0 || source_height <= 0.0 {
        bail!("pdftotext XHTML has invalid page dimensions");
    }
    let scale_x = width as f32 / source_width;
    let scale_y = height as f32 / source_height;
    let rectangle = |node: &roxmltree::Node<'_, '_>| -> Result<Vec<Point>> {
        let x_min = pdf_text_coordinate(node, "xMin")? * scale_x;
        let y_min = pdf_text_coordinate(node, "yMin")? * scale_y;
        let x_max = pdf_text_coordinate(node, "xMax")? * scale_x;
        let y_max = pdf_text_coordinate(node, "yMax")? * scale_y;
        Ok(vec![
            Point { x: x_min, y: y_min },
            Point { x: x_max, y: y_min },
            Point { x: x_max, y: y_max },
            Point { x: x_min, y: y_max },
        ])
    };
    let mut regions = Vec::new();
    for (region_index, block) in page
        .descendants()
        .filter(|node| node.has_tag_name("block"))
        .enumerate()
    {
        let mut lines = Vec::new();
        for (line_index, line) in block
            .children()
            .filter(|node| node.has_tag_name("line"))
            .enumerate()
        {
            let mut words = Vec::new();
            for (word_index, word) in line
                .children()
                .filter(|node| node.has_tag_name("word"))
                .enumerate()
            {
                let text = word.text().unwrap_or_default().trim();
                if text.is_empty() {
                    continue;
                }
                words.push(AltoWord {
                    id: format!("pdf-{region_index:04}-{line_index:04}-{word_index:04}"),
                    polygon: rectangle(&word)?,
                    text: text.to_owned(),
                    confidence: 0.0,
                    language: None,
                    structural_language: false,
                });
            }
            if words.is_empty() {
                continue;
            }
            lines.push(AltoLine {
                id: format!("pdf-{region_index:04}-{line_index:04}"),
                polygon: rectangle(&line)?,
                text: join_words(&words),
                confidence: 0.0,
                words,
            });
        }
        if lines.is_empty() {
            continue;
        }
        regions.push(AltoRegion {
            id: format!("pdf-{region_index:04}"),
            polygon: rectangle(&block)?,
            lines,
        });
    }
    Ok(AltoPage {
        width,
        height,
        regions,
    })
}

fn pdf_text_coordinate(node: &roxmltree::Node<'_, '_>, name: &str) -> Result<f32> {
    node.attribute(name)
        .with_context(|| format!("pdftotext XHTML node has no {name}"))?
        .parse()
        .with_context(|| format!("invalid pdftotext XHTML coordinate {name}"))
}

fn preprocess(
    input: &Path,
    page_path: &Path,
    settings: &PreprocessingSettings,
    run_id: &str,
) -> Result<(PathBuf, String)> {
    let output = page_path.join("processed.png");
    let transform_path = page_path.join("transform.json");
    let mut arguments = vec![input.display().to_string()];
    let mut operations = Vec::new();
    if settings.grayscale {
        arguments.extend(["-colorspace".to_owned(), "Gray".to_owned()]);
        operations.push(TransformOperation::Grayscale);
    }
    if settings.deskew_degrees != 0.0 {
        arguments.extend([
            "-background".to_owned(),
            "white".to_owned(),
            "-rotate".to_owned(),
            settings.deskew_degrees.to_string(),
        ]);
        operations.push(TransformOperation::Rotate {
            degrees: settings.deskew_degrees,
        });
    }
    if let Some([width, height, x, y]) = settings.crop {
        arguments.extend([
            "-crop".to_owned(),
            format!("{width}x{height}+{x}+{y}"),
            "+repage".to_owned(),
        ]);
        operations.push(TransformOperation::Crop {
            width,
            height,
            x,
            y,
        });
    }
    if !settings.contrast_stretch.is_empty() {
        arguments.extend([
            "-contrast-stretch".to_owned(),
            settings.contrast_stretch.clone(),
        ]);
        operations.push(TransformOperation::ContrastStretch {
            geometry: settings.contrast_stretch.clone(),
        });
    }
    if operations.is_empty() {
        operations.push(TransformOperation::Identity);
    }
    arguments.push(output.display().to_string());
    let settings_json = serde_json::to_string(settings)?;
    let input_hash = content_hash(&[run_id, &settings_json]);
    run_resumable_command(
        "preprocess",
        &input_hash,
        "magick",
        &arguments,
        std::slice::from_ref(&output),
        &page_path.join("preprocess.stage.json"),
    )?;
    let source_dimensions = image_dimensions(input)?;
    let output_dimensions = image_dimensions(&output)?;
    let transform = TransformManifest {
        source_image: relative_or_absolute(input),
        source_dimensions,
        output_dimensions,
        operations,
        command: std::iter::once("magick".to_owned())
            .chain(arguments)
            .collect(),
    };
    let transform_json = serde_json::to_vec_pretty(&transform)?;
    fs::write(&transform_path, transform_json)?;
    Ok((output, content_hash(&[&input_hash, "transform-v1"])))
}

fn recognize_tesseract(
    input: &Path,
    page_path: &Path,
    settings: &TesseractSettings,
    languages: &[String],
    pass: &str,
    raster_dpi: u32,
    run_id: &str,
) -> Result<PathBuf> {
    let output_path = page_path.join(format!("tesseract-{pass}.alto.xml"));
    let arguments = vec![
        input.display().to_string(),
        "stdout".to_owned(),
        "--dpi".to_owned(),
        raster_dpi.to_string(),
        "-l".to_owned(),
        languages.join("+"),
        "--psm".to_owned(),
        settings.page_segmentation_mode.to_string(),
        "alto".to_owned(),
    ];
    let input_hash = content_hash(&[
        run_id,
        &serde_json::to_string(settings)?,
        pass,
        &languages.join("+"),
        &raster_dpi.to_string(),
    ]);
    let receipt = page_path.join(format!("tesseract-{pass}.stage.json"));
    if !stage_is_current(&receipt, &input_hash, std::slice::from_ref(&output_path))? {
        let command_output = execute("tesseract", &arguments)?;
        if command_output.stdout.is_empty() {
            bail!("Tesseract produced empty ALTO output");
        }
        fs::write(&output_path, command_output.stdout)?;
        write_receipt(
            &receipt,
            &format!("tesseract-{pass}"),
            &input_hash,
            "tesseract",
            &arguments,
            std::slice::from_ref(&output_path),
        )?;
    }
    Ok(output_path)
}

#[derive(Debug, Serialize)]
struct BlockRefinement {
    region_id: String,
    crop: String,
    source_lines: usize,
    recognized_lines: usize,
    replaced_lines: usize,
}

fn recognize_tesseract_blocks(
    input: &Path,
    page_path: &Path,
    settings: &TesseractSettings,
    layout: &AltoPage,
    raster_dpi: u32,
    run_id: &str,
) -> Result<AltoPage> {
    let output_path = page_path.join("tesseract-block-refined.alto.xml");
    let manifest_path = page_path.join("tesseract-block-refinements.json");
    let receipt_path = page_path.join("tesseract-blocks.stage.json");
    let input_hash = content_hash(&[
        run_id,
        &serde_json::to_string(settings)?,
        &serde_json::to_string(layout)?,
        "block-recognition-v1",
    ]);
    let outputs = [output_path.clone(), manifest_path.clone()];
    if stage_is_current(&receipt_path, &input_hash, &outputs)? {
        return parse_alto(&fs::read_to_string(output_path)?);
    }

    let blocks_path = page_path.join("blocks");
    fs::create_dir_all(&blocks_path)?;
    let mut refined = layout.clone();
    let mut records = Vec::new();
    for (region_index, region) in refined.regions.iter_mut().enumerate() {
        if region.lines.len() < 2 {
            continue;
        }
        let Some((x, y, width, height)) = padded_polygon_bounds(
            &region.polygon,
            layout.width,
            layout.height,
            settings.block_padding_pixels,
        ) else {
            continue;
        };
        let stem = format!("block-{region_index:04}");
        let crop_path = blocks_path.join(format!("{stem}.png"));
        let crop_arguments = vec![
            input.display().to_string(),
            "-crop".to_owned(),
            format!("{width}x{height}+{x}+{y}"),
            "+repage".to_owned(),
            crop_path.display().to_string(),
        ];
        run_resumable_command(
            &format!("crop-{stem}"),
            &content_hash(&[
                &input_hash,
                &serde_json::to_string(&region.polygon)?,
                &settings.block_padding_pixels.to_string(),
            ]),
            "magick",
            &crop_arguments,
            std::slice::from_ref(&crop_path),
            &blocks_path.join(format!("{stem}.crop.stage.json")),
        )?;

        let alto_path = blocks_path.join(format!("{stem}.xml"));
        let alto_stem = blocks_path.join(&stem);
        let tesseract_arguments = vec![
            crop_path.display().to_string(),
            alto_stem.display().to_string(),
            "--dpi".to_owned(),
            raster_dpi.to_string(),
            "-l".to_owned(),
            settings.primary_languages.join("+"),
            "--psm".to_owned(),
            settings.block_page_segmentation_mode.to_string(),
            "alto".to_owned(),
        ];
        run_resumable_command(
            &format!("recognize-{stem}"),
            &content_hash(&[
                &input_hash,
                &sha256_file(&crop_path)?,
                &settings.primary_languages.join("+"),
                &settings.block_page_segmentation_mode.to_string(),
            ]),
            "tesseract",
            &tesseract_arguments,
            std::slice::from_ref(&alto_path),
            &blocks_path.join(format!("{stem}.tesseract.stage.json")),
        )?;
        let candidate = parse_alto(&fs::read_to_string(&alto_path)?)?;
        let candidate_line_count = candidate
            .regions
            .iter()
            .map(|candidate_region| candidate_region.lines.len())
            .sum();
        let source_line_count = region.lines.len();
        let replaced_lines = replace_block_lines(&mut region.lines, &candidate, x, y);
        deduplicate_overlapping_lines(&mut region.lines);
        records.push(BlockRefinement {
            region_id: region.id.clone(),
            crop: relative_or_absolute(&crop_path),
            source_lines: source_line_count,
            recognized_lines: candidate_line_count,
            replaced_lines,
        });
    }

    fs::write(
        &output_path,
        write_alto(&refined, &relative_or_absolute(input)),
    )?;
    fs::write(&manifest_path, serde_json::to_vec_pretty(&records)?)?;
    write_receipt(
        &receipt_path,
        "tesseract-block-refinement",
        &input_hash,
        "tesseract",
        &[
            "layout block crops".to_owned(),
            format!("--psm {}", settings.block_page_segmentation_mode),
            format!("--padding {}", settings.block_padding_pixels),
        ],
        &outputs,
    )?;
    Ok(refined)
}

fn replace_block_lines(
    source: &mut [AltoLine],
    candidate: &AltoPage,
    offset_x: u32,
    offset_y: u32,
) -> usize {
    let mut candidates: Vec<_> = candidate
        .regions
        .iter()
        .flat_map(|region| region.lines.iter().cloned())
        .collect();
    for line in &mut candidates {
        translate_points(&mut line.polygon, offset_x, offset_y);
        for word in &mut line.words {
            translate_points(&mut word.polygon, offset_x, offset_y);
        }
    }
    let mut used = BTreeSet::new();
    let mut replaced = 0;
    for line in source {
        let Some((candidate_index, _)) = candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| !used.contains(index))
            .filter_map(|(index, candidate)| {
                let distance = vertical_center_distance(&line.polygon, &candidate.polygon)?;
                let tolerance =
                    polygon_height(&line.polygon)?.max(polygon_height(&candidate.polygon)?) * 1.5;
                (distance <= tolerance).then_some((index, distance))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
        else {
            continue;
        };
        used.insert(candidate_index);
        let candidate = &candidates[candidate_index];
        line.text.clone_from(&candidate.text);
        line.confidence = candidate.confidence;
        line.polygon.clone_from(&candidate.polygon);
        line.words = candidate.words.clone();
        for (word_index, word) in line.words.iter_mut().enumerate() {
            word.id = format!("{}-block-word-{:04}", line.id, word_index + 1);
        }
        replaced += 1;
    }
    replaced
}

fn deduplicate_overlapping_lines(lines: &mut Vec<AltoLine>) {
    let mut retained: Vec<AltoLine> = Vec::with_capacity(lines.len());
    for line in lines.drain(..) {
        if let Some(existing) = retained
            .iter_mut()
            .find(|existing| polygon_iou(&existing.polygon, &line.polygon) >= 0.8)
        {
            if line.confidence > existing.confidence {
                *existing = line;
            }
        } else {
            retained.push(line);
        }
    }
    *lines = retained;
}

fn translate_points(points: &mut [Point], offset_x: u32, offset_y: u32) {
    for point in points {
        point.x += offset_x as f32;
        point.y += offset_y as f32;
    }
}

fn vertical_center_distance(left: &[Point], right: &[Point]) -> Option<f32> {
    let left_center = polygon_vertical_center(left)?;
    let right_center = polygon_vertical_center(right)?;
    Some((left_center - right_center).abs())
}

fn polygon_vertical_center(points: &[Point]) -> Option<f32> {
    let minimum = points.iter().map(|point| point.y).reduce(f32::min)?;
    let maximum = points.iter().map(|point| point.y).reduce(f32::max)?;
    Some((minimum + maximum) / 2.0)
}

fn polygon_height(points: &[Point]) -> Option<f32> {
    let minimum = points.iter().map(|point| point.y).reduce(f32::min)?;
    let maximum = points.iter().map(|point| point.y).reduce(f32::max)?;
    Some(maximum - minimum)
}

fn padded_polygon_bounds(
    polygon: &[Point],
    page_width: u32,
    page_height: u32,
    padding: u32,
) -> Option<(u32, u32, u32, u32)> {
    let min_x = polygon
        .iter()
        .map(|point| point.x)
        .reduce(f32::min)?
        .floor()
        .max(0.0) as u32;
    let min_y = polygon
        .iter()
        .map(|point| point.y)
        .reduce(f32::min)?
        .floor()
        .max(0.0) as u32;
    let max_x = polygon
        .iter()
        .map(|point| point.x)
        .reduce(f32::max)?
        .ceil()
        .max(0.0) as u32;
    let max_y = polygon
        .iter()
        .map(|point| point.y)
        .reduce(f32::max)?
        .ceil()
        .max(0.0) as u32;
    let x = min_x.saturating_sub(padding);
    let y = min_y.saturating_sub(padding);
    let right = max_x.saturating_add(padding).min(page_width);
    let bottom = max_y.saturating_add(padding).min(page_height);
    let width = right.saturating_sub(x);
    let height = bottom.saturating_sub(y);
    (width > 0 && height > 0).then_some((x, y, width, height))
}

#[derive(Debug, Serialize)]
struct IsolatedWordRecognition {
    line_id: String,
    word_id: String,
    crop: String,
    detected_text: String,
    detected_language: Option<String>,
    label_language: Option<String>,
    announced_languages: Vec<String>,
    routed_language: String,
    pdf_text: Option<String>,
    detected_confidence: f32,
    trials: Vec<ScriptTrial>,
    selected_language: Option<String>,
    isolated_text: Option<String>,
    isolated_confidence: Option<f32>,
    used_isolated_text: bool,
}

/// Tesseract models that read exactly one script, so a crop can be arbitrated
/// between them. `eng` and `lat` share the Latin script and are left to the
/// English-primary pass.
const SINGLE_SCRIPT_LANGUAGES: &[&str] = &["heb", "ara", "syr", "grc"];

struct WordCandidate {
    text: String,
    confidence: f32,
}

fn word_recognition_modes(settings: &TesseractSettings) -> Vec<u8> {
    let mut modes = Vec::new();
    for mode in [
        settings.word_page_segmentation_mode,
        settings.word_fallback_page_segmentation_mode,
    ]
    .into_iter()
    .chain(
        settings
            .word_additional_page_segmentation_modes
            .iter()
            .copied(),
    ) {
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    modes
}

/// Restores punctuation that Tesseract emits as a separate visual-order token
/// before a right-to-left word. The TSV order `. אלֶף` represents the printed
/// logical word `אלֶף.` rather than two tokens.
fn normalize_word_candidate(text: &str, language: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if !matches!(language, "heb" | "ara" | "syr") || words.len() < 2 {
        return words.join(" ");
    }
    let leading_marks = words
        .iter()
        .take_while(|word| {
            !word
                .chars()
                .any(|character| character.is_alphabetic() || character.is_numeric())
        })
        .count();
    if leading_marks == 0 || leading_marks == words.len() {
        return words.join(" ");
    }
    format!(
        "{}{}",
        words[leading_marks..].join(" "),
        words[..leading_marks].join("")
    )
}

/// Drops crop-edge noise without discarding punctuation independently visible
/// in the page reading. OCR may mistake a foreign word for digits while still
/// locating its final full stop correctly; that full stop belongs to the
/// source even though the letters themselves are replaced.
fn trim_unattested_edge_punctuation(detected: &str, candidate: &str) -> String {
    let is_edge_mark =
        |character: char| !character.is_alphanumeric() && canonical_combining_class(character) == 0;
    let detected_leading: Vec<char> = detected.chars().take_while(|c| is_edge_mark(*c)).collect();
    let detected_trailing: Vec<char> = detected
        .chars()
        .rev()
        .take_while(|c| is_edge_mark(*c))
        .collect();
    candidate
        .trim_matches(|character| {
            is_edge_mark(character)
                && !detected_leading.contains(&character)
                && !detected_trailing.contains(&character)
        })
        .to_owned()
}

/// Selects the crop reading with the greatest amount of confidence-backed
/// evidence in the requested script. Confidence alone favors short fragments:
/// a clear single Aleph can outscore a slightly less certain complete pointed
/// word. Scaling confidence by the square root of linguistic characters keeps
/// that fragment from winning without letting a long, weak hallucination take
/// over.
fn select_word_candidate(candidates: Vec<WordCandidate>, language: &str) -> Option<WordCandidate> {
    let evidence = |candidate: &WordCandidate| {
        if !word_matches_language(&candidate.text, language) {
            return 0.0;
        }
        let linguistic = candidate
            .text
            .chars()
            .filter(|character| {
                character.is_alphabetic() || canonical_combining_class(*character) != 0
            })
            .count();
        candidate.confidence * (linguistic as f32).sqrt()
    };
    candidates
        .into_iter()
        .filter(|candidate| evidence(candidate) > 0.0)
        .max_by(|left, right| {
            evidence(left)
                .total_cmp(&evidence(right))
                .then_with(|| left.confidence.total_cmp(&right.confidence))
        })
}

fn recognize_crop_candidates(
    words_path: &Path,
    stem: &str,
    crop_path: &Path,
    language: &str,
    modes: &[u8],
    scaled_dpi: u32,
    input_hash: &str,
) -> Result<Vec<WordCandidate>> {
    let crop_hash = sha256_file(crop_path)?;
    let mut candidates = Vec::new();
    for mode in modes {
        let trial_stem = format!("{stem}-{language}-psm{mode}");
        let tsv_stem = words_path.join(&trial_stem);
        let tsv_path = words_path.join(format!("{trial_stem}.tsv"));
        let arguments = vec![
            crop_path.display().to_string(),
            tsv_stem.display().to_string(),
            "--dpi".to_owned(),
            scaled_dpi.to_string(),
            "-l".to_owned(),
            language.to_owned(),
            "--psm".to_owned(),
            mode.to_string(),
            "tsv".to_owned(),
        ];
        run_resumable_command(
            &format!("recognize-{trial_stem}"),
            &content_hash(&[input_hash, &crop_hash, language, &mode.to_string()]),
            "tesseract",
            &arguments,
            std::slice::from_ref(&tsv_path),
            &words_path.join(format!("{trial_stem}.tesseract.stage.json")),
        )?;
        if let Some(mut candidate) = parse_tesseract_word_tsv(&fs::read_to_string(&tsv_path)?)? {
            candidate.text = normalize_word_candidate(&candidate.text, language);
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn roman_token_key(text: &str) -> String {
    text.trim_matches(|character: char| {
        !character.is_alphanumeric() && canonical_combining_class(character) == 0
    })
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
}

fn is_roman_text(text: &str) -> bool {
    use unicode_script::{Script, UnicodeScript};
    let letters: Vec<char> = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect();
    !letters.is_empty()
        && letters
            .iter()
            .all(|character| character.script() == Script::Latin)
}

fn should_refine_roman_word(
    word: &AltoWord,
    pdf_text: Option<&str>,
    confidence_threshold: f32,
) -> bool {
    let Some(pdf_text) = pdf_text.filter(|text| is_roman_text(text)) else {
        return false;
    };
    if roman_token_key(&word.text) == roman_token_key(pdf_text) {
        return false;
    }
    let detected_has_letters = word.text.chars().any(char::is_alphabetic);
    !detected_has_letters || (is_roman_text(&word.text) && word.confidence < confidence_threshold)
}

fn select_roman_consensus_candidate(
    candidates: Vec<WordCandidate>,
    pdf_text: &str,
    detected_confidence: f32,
) -> Option<WordCandidate> {
    let expected = roman_token_key(pdf_text);
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate.confidence >= 0.50
                && candidate.confidence + 0.02 >= detected_confidence
                && is_roman_text(&candidate.text)
                && roman_token_key(&candidate.text) == expected
        })
        .max_by(|left, right| left.confidence.total_cmp(&right.confidence))
}

fn aligned_pdf_word<'a>(page: &'a AltoPage, target: &AltoWord) -> Option<&'a AltoWord> {
    page.regions
        .iter()
        .flat_map(|region| &region.lines)
        .flat_map(|line| &line.words)
        .filter_map(|candidate| {
            let overlap = polygon_intersection_over_smaller(&target.polygon, &candidate.polygon);
            (overlap >= 0.45).then_some((candidate, overlap))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(candidate, _)| candidate)
}

fn polygon_intersection_over_smaller(left: &[Point], right: &[Point]) -> f32 {
    let bounds = |polygon: &[Point]| {
        let min_x = polygon.iter().map(|point| point.x).reduce(f32::min)?;
        let min_y = polygon.iter().map(|point| point.y).reduce(f32::min)?;
        let max_x = polygon.iter().map(|point| point.x).reduce(f32::max)?;
        let max_y = polygon.iter().map(|point| point.y).reduce(f32::max)?;
        Some((min_x, min_y, max_x, max_y))
    };
    let (
        Some((left_x1, left_y1, left_x2, left_y2)),
        Some((right_x1, right_y1, right_x2, right_y2)),
    ) = (bounds(left), bounds(right))
    else {
        return 0.0;
    };
    let intersection_width = left_x2.min(right_x2) - left_x1.max(right_x1);
    let intersection_height = left_y2.min(right_y2) - left_y1.max(right_y1);
    if intersection_width <= 0.0 || intersection_height <= 0.0 {
        return 0.0;
    }
    let left_area = (left_x2 - left_x1) * (left_y2 - left_y1);
    let right_area = (right_x2 - right_x1) * (right_y2 - right_y1);
    let smaller = left_area.min(right_area);
    if smaller <= 0.0 {
        0.0
    } else {
        intersection_width * intersection_height / smaller
    }
}

fn recognize_tesseract_words(
    input: &Path,
    page_path: &Path,
    settings: &TesseractSettings,
    classified: &AltoPage,
    raster_dpi: u32,
    run_id: &str,
    pdf_text_page: Option<&AltoPage>,
) -> Result<AltoPage> {
    let output_path = page_path.join("tesseract-word-recognized.alto.xml");
    let manifest_path = page_path.join("tesseract-word-recognitions.json");
    let receipt_path = page_path.join("tesseract-words.stage.json");
    let input_hash = content_hash(&[
        run_id,
        &serde_json::to_string(settings)?,
        &serde_json::to_string(classified)?,
        &serde_json::to_string(&pdf_text_page)?,
        "isolated-word-recognition-v6",
    ]);
    let outputs = [output_path.clone(), manifest_path.clone()];
    if stage_is_current(&receipt_path, &input_hash, &outputs)? {
        return parse_alto(&fs::read_to_string(output_path)?);
    }

    let words_path = page_path.join("words");
    fs::create_dir_all(&words_path)?;
    let trial_languages: Vec<String> = settings
        .multilingual_languages
        .iter()
        .filter(|language| SINGLE_SCRIPT_LANGUAGES.contains(&language.as_str()))
        .cloned()
        .collect();
    let mut refined = classified.clone();
    let mut records = Vec::new();
    let mut ordinal = 0_usize;
    for region in &mut refined.regions {
        for line in &mut region.lines {
            let announced = announced_line_languages(line, &settings.multilingual_languages);
            let labels = printed_label_languages(&line.words);
            let pdf_texts: Vec<Option<String>> = line
                .words
                .iter()
                .map(|word| {
                    pdf_text_page
                        .and_then(|page| aligned_pdf_word(page, word))
                        .map(|candidate| candidate.text.clone())
                })
                .collect();
            for (index, word) in line.words.iter_mut().enumerate() {
                let pdf_text = pdf_texts[index].clone();
                let roman_refinement = word.language.is_none()
                    && should_refine_roman_word(
                        word,
                        pdf_text.as_deref(),
                        settings.roman_word_refinement_confidence,
                    );
                let Some(routed_language) = word
                    .language
                    .clone()
                    .or_else(|| roman_refinement.then(|| "eng".to_owned()))
                else {
                    continue;
                };
                ordinal += 1;
                let detected_text = word.text.clone();
                let detected_confidence = word.confidence;
                let detected_language =
                    detected_word_language(&detected_text).map(ToOwned::to_owned);
                // A structural route is as decisive as a printed label: the
                // edition sets its lemmas in square Hebrew whatever the
                // multilingual pass made of them.
                let label_language = if word.structural_language {
                    Some(routed_language.clone())
                } else {
                    labels[index].map(ToOwned::to_owned)
                };
                let stem = format!("word-{ordinal:05}");
                let crop_path = words_path.join(format!("{stem}.png"));
                let (x, y, width, height) = padded_word_bounds(
                    word,
                    classified.width,
                    classified.height,
                    settings.word_padding_pixels,
                )
                .with_context(|| format!("word {} has empty geometry", word.id))?;
                let scale_percent = if roman_refinement {
                    settings.roman_word_scale_percent
                } else {
                    settings.word_scale_percent
                };
                let crop_arguments = vec![
                    input.display().to_string(),
                    "-crop".to_owned(),
                    format!("{width}x{height}+{x}+{y}"),
                    "+repage".to_owned(),
                    "-bordercolor".to_owned(),
                    "white".to_owned(),
                    "-border".to_owned(),
                    "8".to_owned(),
                    "-resize".to_owned(),
                    format!("{scale_percent}%"),
                    crop_path.display().to_string(),
                ];
                run_resumable_command(
                    &format!("crop-{stem}"),
                    &content_hash(&[&input_hash, &serde_json::to_string(&word.polygon)?]),
                    "magick",
                    &crop_arguments,
                    std::slice::from_ref(&crop_path),
                    &words_path.join(format!("{stem}.crop.stage.json")),
                )?;

                let threshold_crop_path = roman_refinement.then(|| {
                    words_path.join(format!(
                        "{stem}-threshold-{}.png",
                        settings.roman_word_threshold_percent
                    ))
                });
                if let Some(threshold_crop_path) = threshold_crop_path.as_ref() {
                    let arguments = vec![
                        crop_path.display().to_string(),
                        "-threshold".to_owned(),
                        format!("{}%", settings.roman_word_threshold_percent),
                        threshold_crop_path.display().to_string(),
                    ];
                    run_resumable_command(
                        &format!("threshold-{stem}"),
                        &content_hash(&[
                            &input_hash,
                            &sha256_file(&crop_path)?,
                            &settings.roman_word_threshold_percent.to_string(),
                        ]),
                        "magick",
                        &arguments,
                        std::slice::from_ref(threshold_crop_path),
                        &words_path.join(format!("{stem}.threshold.stage.json")),
                    )?;
                }

                // Read the one crop with every single-script model the edition
                // needs. A word the multilingual pass mis-scripted can then be
                // recovered instead of being locked to its first reading.
                let scaled_dpi = raster_dpi.saturating_mul(u32::from(scale_percent)) / 100;
                let (trials, selected, use_isolated) = if roman_refinement {
                    let pdf_text = pdf_text.as_deref().context("missing PDF text witness")?;
                    let mut candidates = recognize_crop_candidates(
                        &words_path,
                        &format!("{stem}-plain"),
                        &crop_path,
                        "eng",
                        &settings.roman_word_page_segmentation_modes,
                        scaled_dpi,
                        &input_hash,
                    )?;
                    if let Some(threshold_crop_path) = threshold_crop_path.as_ref() {
                        candidates.extend(recognize_crop_candidates(
                            &words_path,
                            &format!("{stem}-threshold"),
                            threshold_crop_path,
                            "eng",
                            &settings.roman_word_page_segmentation_modes,
                            scaled_dpi,
                            &input_hash,
                        )?);
                    }
                    let candidate =
                        select_roman_consensus_candidate(candidates, pdf_text, detected_confidence);
                    let selected = candidate.map(|candidate| ScriptTrial {
                        language: "eng".to_owned(),
                        text: pdf_text.to_owned(),
                        confidence: candidate.confidence,
                    });
                    let trials = selected.iter().cloned().collect();
                    let use_isolated = selected.is_some();
                    (trials, selected, use_isolated)
                } else {
                    let mut trials = Vec::new();
                    for language in &trial_languages {
                        // A line, word, and sparse crop can produce materially
                        // different readings of pointed type. Compare every
                        // configured mode instead of accepting the first
                        // nonempty result.
                        let candidates = recognize_crop_candidates(
                            &words_path,
                            &stem,
                            &crop_path,
                            language,
                            &word_recognition_modes(settings),
                            scaled_dpi,
                            &input_hash,
                        )?;
                        let Some(candidate) = select_word_candidate(candidates, language) else {
                            continue;
                        };
                        // When the multilingual pass saw only punctuation
                        // there is nothing for unsupported crop-edge marks to
                        // correspond to, so they are dropped as noise.
                        let text = if detected_text.chars().any(char::is_alphabetic) {
                            candidate.text
                        } else {
                            trim_unattested_edge_punctuation(&detected_text, &candidate.text)
                        };
                        trials.push(ScriptTrial {
                            language: language.clone(),
                            text,
                            confidence: candidate.confidence,
                        });
                    }

                    let selected = select_script_trial(
                        &trials,
                        WordScriptContext {
                            routed: &routed_language,
                            detected: detected_language.as_deref(),
                            detected_confidence,
                            label: label_language.as_deref(),
                            announced: &announced,
                        },
                    )
                    .cloned();
                    let use_isolated = selected.as_ref().is_some_and(|selected| {
                        should_use_isolated_word(
                            &detected_text,
                            detected_language.as_deref(),
                            detected_confidence,
                            selected,
                        )
                    });
                    (trials, selected, use_isolated)
                };
                if let Some(selected) = selected.as_ref().filter(|_| use_isolated) {
                    word.text.clone_from(&selected.text);
                    word.confidence = selected.confidence;
                    word.language = Some(selected.language.clone());
                }
                records.push(IsolatedWordRecognition {
                    line_id: line.id.clone(),
                    word_id: word.id.clone(),
                    crop: relative_or_absolute(&crop_path),
                    detected_text,
                    detected_language,
                    label_language,
                    announced_languages: announced.clone(),
                    routed_language,
                    pdf_text,
                    detected_confidence,
                    trials,
                    selected_language: selected.as_ref().map(|selected| selected.language.clone()),
                    isolated_text: selected.as_ref().map(|selected| selected.text.clone()),
                    isolated_confidence: selected.as_ref().map(|selected| selected.confidence),
                    used_isolated_text: use_isolated,
                });
            }
            // Word text is the line's only source of truth downstream, so the
            // line has to be rebuilt from the arbitrated words.
            if !line.words.is_empty() {
                line.text = join_words(&line.words);
                line.confidence = word_confidence(&line.words);
            }
        }
    }

    fs::write(
        &output_path,
        write_alto(&refined, &relative_or_absolute(input)),
    )?;
    fs::write(&manifest_path, serde_json::to_vec_pretty(&records)?)?;
    write_receipt(
        &receipt_path,
        "tesseract-isolated-words",
        &input_hash,
        "tesseract",
        &[
            "single-language word crops".to_owned(),
            format!("--psm {:?}", word_recognition_modes(settings)),
            format!("--scale {}%", settings.word_scale_percent),
        ],
        &outputs,
    )?;
    Ok(refined)
}

fn padded_word_bounds(
    word: &AltoWord,
    page_width: u32,
    page_height: u32,
    padding: u32,
) -> Option<(u32, u32, u32, u32)> {
    let min_x = word
        .polygon
        .iter()
        .map(|point| point.x)
        .reduce(f32::min)?
        .floor()
        .max(0.0) as u32;
    let min_y = word
        .polygon
        .iter()
        .map(|point| point.y)
        .reduce(f32::min)?
        .floor()
        .max(0.0) as u32;
    let max_x = word
        .polygon
        .iter()
        .map(|point| point.x)
        .reduce(f32::max)?
        .ceil()
        .max(0.0) as u32;
    let max_y = word
        .polygon
        .iter()
        .map(|point| point.y)
        .reduce(f32::max)?
        .ceil()
        .max(0.0) as u32;
    let x = min_x.saturating_sub(padding);
    let y = min_y.saturating_sub(padding);
    let right = max_x.saturating_add(padding).min(page_width);
    let bottom = max_y.saturating_add(padding).min(page_height);
    let width = right.saturating_sub(x);
    let height = bottom.saturating_sub(y);
    (width > 0 && height > 0).then_some((x, y, width, height))
}

fn parse_tesseract_word_tsv(tsv: &str) -> Result<Option<WordCandidate>> {
    let mut words = Vec::new();
    for line in tsv.lines().skip(1) {
        let columns: Vec<_> = line.split('\t').collect();
        if columns.len() < 12 || columns[0] != "5" || columns[11].trim().is_empty() {
            continue;
        }
        let confidence = columns[10]
            .parse::<f32>()
            .with_context(|| format!("invalid Tesseract word confidence `{}`", columns[10]))?
            .clamp(0.0, 100.0)
            / 100.0;
        words.push((columns[11].trim().to_owned(), confidence));
    }
    if words.is_empty() {
        return Ok(None);
    }
    let (weighted, characters) =
        words
            .iter()
            .fold((0.0_f32, 0_usize), |(weighted, characters), word| {
                let length = word.0.chars().count().max(1);
                (weighted + word.1 * length as f32, characters + length)
            });
    Ok(Some(WordCandidate {
        text: words
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        confidence: weighted / characters as f32,
    }))
}

/// Whether an arbitrated single-script reading should replace the word.
///
/// A word the multilingual pass read in a different script, or read as
/// implausible Latin, is replaced outright: the arbitration has already
/// established that the selected script explains the crop. A word already read
/// in the selected script is replaced only by a clearly more confident reading
/// of comparable length, so isolated recognition cannot silently truncate it.
fn should_use_isolated_word(
    detected_text: &str,
    detected_language: Option<&str>,
    detected_confidence: f32,
    selected: &ScriptTrial,
) -> bool {
    if detected_language != Some(selected.language.as_str()) {
        return true;
    }
    let detected_letters = detected_text
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    let candidate_letters = selected
        .text
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    let comparable = candidate_letters.saturating_mul(2) >= detected_letters
        && normalized_disagreement(detected_text, &selected.text) <= 0.34;
    let clearly_more_confident = selected.confidence >= detected_confidence + 0.05;
    // An isolated crop removes the neighbouring type that confuses a page
    // model. Accept a strong same-script reading when it is within ten points
    // of the page confidence as well as when it wins outright; this recovers
    // small one-character errors without admitting weak or truncated crops.
    let strong_isolated =
        selected.confidence >= 0.70 && selected.confidence + 0.10 >= detected_confidence;
    comparable && (clearly_more_confident || strong_isolated)
}

fn recognize_kraken(
    input: &Path,
    page_path: &Path,
    settings: &KrakenSettings,
    run_id: &str,
) -> Result<PathBuf> {
    let output = page_path.join("kraken.alto.xml");
    let mut arguments = vec![
        "-i".to_owned(),
        input.display().to_string(),
        output.display().to_string(),
        "--alto".to_owned(),
        "segment".to_owned(),
        "-bl".to_owned(),
    ];
    if settings.segmentation_model != "default" {
        arguments.extend(["-i".to_owned(), settings.segmentation_model.clone()]);
    }
    arguments.extend([
        "ocr".to_owned(),
        "-m".to_owned(),
        settings.model_path.display().to_string(),
    ]);
    run_resumable_command(
        "kraken",
        &content_hash(&[run_id, &serde_json::to_string(settings)?]),
        "kraken",
        &arguments,
        std::slice::from_ref(&output),
        &page_path.join("kraken.stage.json"),
    )?;
    Ok(output)
}

fn write_page_parse(page_path: &Path, parsed: &ParsedPage) -> Result<()> {
    let mut json = serde_json::to_vec_pretty(parsed)?;
    json.push(b'\n');
    fs::write(page_path.join("parsed.json"), json)?;
    Ok(())
}

fn run_resumable_command(
    stage: &str,
    input_hash: &str,
    program: &str,
    arguments: &[String],
    outputs: &[PathBuf],
    receipt: &Path,
) -> Result<()> {
    if stage_is_current(receipt, input_hash, outputs)? {
        return Ok(());
    }
    execute(program, arguments)?;
    for output in outputs {
        if !output.is_file() {
            bail!(
                "{program} succeeded but did not create expected output {}",
                output.display()
            );
        }
    }
    write_receipt(receipt, stage, input_hash, program, arguments, outputs)
}

fn execute(program: &str, arguments: &[String]) -> Result<Output> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("failed to execute `{program}`; enter `nix develop`"))?;
    if !output.status.success() {
        bail!(
            "`{program}` failed with status {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn write_receipt(
    path: &Path,
    stage: &str,
    input_hash: &str,
    program: &str,
    arguments: &[String],
    outputs: &[PathBuf],
) -> Result<()> {
    let receipt = StageReceipt {
        stage: stage.to_owned(),
        input_hash: input_hash.to_owned(),
        command: std::iter::once(program.to_owned())
            .chain(arguments.iter().cloned())
            .collect(),
        outputs: outputs
            .iter()
            .map(|path| relative_or_absolute(path))
            .collect(),
        completed_at: Utc::now(),
    };
    fs::write(path, serde_json::to_vec_pretty(&receipt)?)?;
    Ok(())
}

fn stage_is_current(path: &Path, input_hash: &str, outputs: &[PathBuf]) -> Result<bool> {
    if !outputs.iter().all(|output| output.is_file()) || !path.is_file() {
        return Ok(false);
    }
    let receipt: StageReceipt = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("invalid stage receipt {}", path.display()))?;
    Ok(receipt.input_hash == input_hash)
}

fn command_version(program: &str) -> Result<String> {
    command_version_with_flag(program, "--version")
}

fn command_version_with_flag(program: &str, flag: &str) -> Result<String> {
    let output = Command::new(program)
        .arg(flag)
        .output()
        .with_context(|| format!("failed to execute `{program}`; enter `nix develop`"))?;
    if !output.status.success() {
        bail!("`{program} {flag}` failed");
    }
    let text = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    Ok(String::from_utf8_lossy(text)
        .lines()
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_owned())
}

fn tesseract_model_hash(languages: &[String]) -> Result<String> {
    let output = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .context("failed to locate Tesseract language models; enter `nix develop`")?;
    if !output.status.success() {
        bail!("`tesseract --list-langs` failed");
    }
    let listing = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let opening_quote = listing
        .find('"')
        .context("Tesseract did not report its tessdata directory")?;
    let closing_quote = listing[opening_quote + 1..]
        .find('"')
        .map(|offset| opening_quote + 1 + offset)
        .context("Tesseract reported an invalid tessdata directory")?;
    let tessdata = Path::new(&listing[opening_quote + 1..closing_quote]);

    let mut sorted_languages = languages.to_vec();
    sorted_languages.sort();
    sorted_languages.dedup();
    let mut hasher = Sha256::new();
    for language in sorted_languages {
        let model = tessdata.join(format!("{language}.traineddata"));
        let digest = sha256_file(&model)
            .with_context(|| format!("missing Tesseract model {}", model.display()))?;
        hasher.update(language.len().to_le_bytes());
        hasher.update(language.as_bytes());
        hasher.update(digest.as_bytes());
    }
    Ok(hex::encode(hasher.finalize()))
}

fn verify_model(settings: &KrakenSettings) -> Result<()> {
    if !settings.enabled {
        return Ok(());
    }
    if settings.model_sha256.len() != 64 {
        bail!("Kraken model SHA-256 must be registered before OCR");
    }
    let actual = sha256_file(&settings.model_path)
        .with_context(|| format!("missing Kraken model {}", settings.model_path.display()))?;
    if actual != settings.model_sha256 {
        bail!(
            "Kraken model hash mismatch: expected {}, got {actual}",
            settings.model_sha256
        );
    }
    Ok(())
}

fn image_dimensions(path: &Path) -> Result<[u32; 2]> {
    let arguments = vec![
        "identify".to_owned(),
        "-format".to_owned(),
        "%w %h".to_owned(),
        path.display().to_string(),
    ];
    let output = execute("magick", &arguments)?;
    let text = String::from_utf8(output.stdout).context("non-UTF-8 image dimensions")?;
    let mut parts = text.split_whitespace();
    let width = parts.next().context("missing image width")?.parse()?;
    let height = parts.next().context("missing image height")?.parse()?;
    Ok([width, height])
}

fn printed_page(source: &SourceRecord, pdf_page: u32) -> (String, bool) {
    if let Some(label) = source.printed_page_labels.get(&pdf_page) {
        return (label.clone(), false);
    }
    let number = i64::from(pdf_page) + i64::from(source.printed_page_offset);
    if number > 0 {
        (number.to_string(), false)
    } else {
        (format!("front-{pdf_page:04}"), true)
    }
}

fn content_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.len().to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn relative_or_absolute(path: &Path) -> String {
    let current = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    path.strip_prefix(&current)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Creates a deterministic count summary for run artifacts.
pub fn assignment_counts(parsed_pages: &[ParsedPage]) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for assignment in parsed_pages
        .iter()
        .flat_map(|page| page.assignments.iter().map(|(_, _, value)| value))
    {
        let key = match assignment {
            LineAssignment::Entry(_) => "entry",
            LineAssignment::FrontMatter => "front_matter",
            LineAssignment::Unparsed => "unparsed",
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::{
        deduplicate_overlapping_lines, entry_source_page, normalize_word_candidate,
        parse_page_spec, parse_pdf_text_layer, select_roman_consensus_candidate,
        select_word_candidate, should_refine_roman_word, should_replace_entry,
        should_use_isolated_word, trim_unattested_edge_punctuation, WordCandidate,
    };
    use crate::alto::{parse_alto, AltoWord, ScriptTrial};
    use crate::model::{CorpusEntry, Point};
    use std::collections::BTreeSet;

    #[test]
    fn page_specs_are_sorted_and_deduplicated() {
        assert_eq!(parse_page_spec("5,1-3,3").unwrap(), vec![1, 2, 3, 5]);
    }

    #[test]
    fn page_specs_reject_zero_and_backwards_ranges() {
        assert!(parse_page_spec("0").is_err());
        assert!(parse_page_spec("9-2").is_err());
    }

    #[test]
    fn isolated_word_selection_corrects_script_changes_but_rejects_drift() {
        let trial = |language: &str, text: &str, confidence: f32| ScriptTrial {
            language: language.to_owned(),
            text: text.to_owned(),
            confidence,
        };
        assert!(should_use_isolated_word(
            "وو",
            Some("ara"),
            0.56,
            &trial("heb", "חְטָא", 0.73),
        ));
        assert!(!should_use_isolated_word(
            "ابي",
            Some("ara"),
            0.57,
            &trial("ara", "دابى", 0.67),
        ));
        assert!(should_use_isolated_word(
            "אבות",
            Some("heb"),
            0.70,
            &trial("heb", "אָבות", 0.80),
        ));
        assert!(should_use_isolated_word(
            "παλεῖν",
            Some("grc"),
            0.79,
            &trial("grc", "καλεῖν", 0.74),
        ));
        assert!(!should_use_isolated_word(
            "אבות",
            Some("heb"),
            0.90,
            &trial("heb", "אָב", 0.72),
        ));
    }

    #[test]
    fn right_to_left_word_recognition_restores_trailing_punctuation() {
        assert_eq!(normalize_word_candidate(". אלֶף", "heb"), "אלֶף.");
        assert_eq!(normalize_word_candidate("word .", "eng"), "word .");
        assert_eq!(trim_unattested_edge_punctuation("75%.", "אלֶף."), "אלֶף.");
        assert_eq!(trim_unattested_edge_punctuation("&", ":אלֶף;"), "אלֶף");
    }

    #[test]
    fn crop_mode_arbitration_prefers_complete_script_evidence() {
        let selected = select_word_candidate(
            vec![
                WordCandidate {
                    text: "א".to_owned(),
                    confidence: 0.31,
                },
                WordCandidate {
                    text: "אלֶף".to_owned(),
                    confidence: 0.29,
                },
                WordCandidate {
                    text: "אבגדהוזחטי".to_owned(),
                    confidence: 0.05,
                },
            ],
            "heb",
        )
        .expect("one Hebrew reading");
        assert_eq!(selected.text, "אלֶף");
    }

    #[test]
    fn positioned_pdf_text_is_scaled_into_alto_geometry() {
        let page = parse_pdf_text_layer(
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><doc>
                <page width="100" height="200"><flow>
                  <block xMin="10" yMin="20" xMax="40" yMax="30">
                    <line xMin="10" yMin="20" xMax="40" yMax="30">
                      <word xMin="10" yMin="20" xMax="20" yMax="30">ox,</word>
                    </line>
                  </block>
                </flow></page>
            </doc></body></html>"#,
            200,
            400,
        )
        .expect("positioned text");
        let word = &page.regions[0].lines[0].words[0];
        assert_eq!(word.text, "ox,");
        assert_eq!(word.polygon[0], Point { x: 20.0, y: 40.0 });
        assert_eq!(word.polygon[2], Point { x: 40.0, y: 60.0 });
    }

    #[test]
    fn roman_refinement_requires_pdf_and_image_consensus() {
        let word = AltoWord {
            id: "word".to_owned(),
            polygon: Vec::new(),
            text: "oz,".to_owned(),
            confidence: 0.78,
            language: None,
            structural_language: false,
        };
        assert!(should_refine_roman_word(&word, Some("ox,"), 0.85));
        assert!(!should_refine_roman_word(&word, Some("oz,"), 0.85));
        let selected = select_roman_consensus_candidate(
            vec![
                WordCandidate {
                    text: "oz,".to_owned(),
                    confidence: 0.90,
                },
                WordCandidate {
                    text: "ox,".to_owned(),
                    confidence: 0.77,
                },
            ],
            "ox,",
            0.78,
        )
        .expect("independent readings agree");
        assert_eq!(selected.text, "ox,");
        assert!(select_roman_consensus_candidate(
            vec![WordCandidate {
                text: "pronuhciation".to_owned(),
                confidence: 0.56,
            }],
            "pronuhciation",
            0.74,
        )
        .is_none());
    }

    #[test]
    fn entry_source_page_uses_the_page_where_a_continuation_began() {
        let entry: CorpusEntry = serde_json::from_str(
            r#"{
                "id":"test:p1:e0001",
                "aliases":[],
                "edition":"test",
                "printed_page":"1",
                "entry_ordinal":1,
                "headword":null,
                "homograph":null,
                "grammatical_labels":[],
                "blocks":[{
                    "id":"block",
                    "kind":"paragraph",
                    "spans":[{
                        "id":"span",
                        "diplomatic":"continued definition",
                        "normalized":"continued definition",
                        "language":"en",
                        "script":"Latn",
                        "direction":"ltr",
                        "confidence":0.9,
                        "review_state":"machine",
                        "hypotheses":[],
                        "coordinates":[
                            {
                                "source_page":19,
                                "printed_page":"3",
                                "region_id":"r2",
                                "line_id":"l2",
                                "polygon":[],
                                "transform_id":"t",
                                "page_image":"p19.png"
                            },
                            {
                                "source_page":17,
                                "printed_page":"1",
                                "region_id":"r1",
                                "line_id":"l1",
                                "polygon":[],
                                "transform_id":"t",
                                "page_image":"p17.png"
                            }
                        ],
                        "warnings":[]
                    }]
                }],
                "senses":[],
                "citations":[],
                "cross_references":[],
                "etymology":[],
                "provenance":{
                    "edition":"test",
                    "source_sha256":"hash",
                    "scan_id":"scan",
                    "pipeline_run":"run"
                },
                "confidence":0.9,
                "review_state":"machine",
                "revision":0
            }"#,
        )
        .unwrap();

        assert_eq!(entry_source_page(&entry), Some(17));
        assert!(!should_replace_entry(&entry, &BTreeSet::from([19])));
        assert!(should_replace_entry(&entry, &BTreeSet::from([17])));
    }

    #[test]
    fn block_refinement_drops_a_lower_confidence_duplicate_line() {
        let mut page = parse_alto(
            r#"<?xml version="1.0"?>
            <alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
              <Layout><Page WIDTH="1000" HEIGHT="1000"><PrintSpace>
                <TextBlock ID="body" HPOS="100" VPOS="100" WIDTH="800" HEIGHT="100">
                  <TextLine ID="artifact" HPOS="100" VPOS="100" WIDTH="800" HEIGHT="50">
                    <String CONTENT="37-5 m. constr." WC="0.40"
                      HPOS="100" VPOS="100" WIDTH="800" HEIGHT="50"/>
                  </TextLine>
                  <TextLine ID="headword" HPOS="100" VPOS="100" WIDTH="800" HEIGHT="50">
                    <String CONTENT="אב m. constr." WC="0.90"
                      HPOS="100" VPOS="100" WIDTH="800" HEIGHT="50"/>
                  </TextLine>
                </TextBlock>
              </PrintSpace></Page></Layout>
            </alto>"#,
        )
        .unwrap();
        let lines = &mut page.regions[0].lines;

        deduplicate_overlapping_lines(lines);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].id, "headword");
    }
}
