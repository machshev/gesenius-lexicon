//! Resumable content-addressed raster, preprocessing, OCR, and parsing pipeline.

use crate::alto::{
    fuse_multilingual_words, parse_alto, parse_entries_with_hypotheses_continuing, write_alto,
    EngineIdentity, LineAssignment, ParseContext, ParsedPage,
};
use crate::corpus_io::{load_entries, write_entries};
use crate::source::{sha256_file, verify_source, SourceCatalogue, SourceRecord};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
        Ok(())
    }
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
    /// Immutable Nix/tessdata model identity recorded in provenance.
    pub model_identity: String,
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
    if options.pages.is_empty() {
        bail!("page selection is empty");
    }
    let settings = PipelineSettings::load(options.settings_path)?;
    let catalogue = SourceCatalogue::load(options.catalogue_path)?;
    let source = catalogue.edition(options.edition)?;
    let verified = verify_source(source, options.cache_root)?;
    verify_model(&settings.kraken)?;

    let tesseract_version = command_version("tesseract")?;
    let primary_tesseract_models_sha256 =
        tesseract_model_hash(&settings.tesseract.primary_languages)?;
    let multilingual_tesseract_models_sha256 =
        tesseract_model_hash(&settings.tesseract.multilingual_languages)?;
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
            "{}:{}",
            settings.tesseract.model_identity,
            settings.tesseract.primary_languages.join("+")
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

    let mut parsed_pages = Vec::new();
    let mut previous_page_number = None;
    let mut continuation = None;
    for page_number in options.pages {
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
        let original = rasterize(
            &verified.path,
            *page_number,
            settings.raster_dpi,
            &page_path,
            &run_id,
        )?;
        let (processed, transform_id) =
            preprocess(&original, &page_path, &settings.preprocessing, &run_id)?;
        let primary_tesseract_alto = recognize_tesseract(
            &processed,
            &page_path,
            &settings.tesseract,
            &settings.tesseract.primary_languages,
            "primary",
            settings.raster_dpi,
            &run_id,
        )?;
        let multilingual_tesseract_alto = recognize_tesseract(
            &processed,
            &page_path,
            &settings.tesseract,
            &settings.tesseract.multilingual_languages,
            "multilingual",
            settings.raster_dpi,
            &run_id,
        )?;
        let primary_tesseract_page = parse_alto(&fs::read_to_string(&primary_tesseract_alto)?)?;
        let multilingual_tesseract_page =
            parse_alto(&fs::read_to_string(&multilingual_tesseract_alto)?)?;
        let fused_tesseract_page =
            fuse_multilingual_words(&primary_tesseract_page, &multilingual_tesseract_page);
        fs::write(
            page_path.join("tesseract-fused.alto.xml"),
            write_alto(&fused_tesseract_page, &relative_or_absolute(&processed)),
        )?;
        let kraken_page = if kraken_identity.is_some() {
            let kraken_alto = recognize_kraken(&processed, &page_path, &settings.kraken, &run_id)?;
            Some(parse_alto(&fs::read_to_string(&kraken_alto)?)?)
        } else {
            None
        };
        let canonical_page = kraken_page.as_ref().unwrap_or(&fused_tesseract_page);
        let mut hypotheses = Vec::new();
        if let (Some(page), Some(identity)) = (kraken_page.as_ref(), kraken_identity.as_ref()) {
            hypotheses.push((page, identity));
        }
        hypotheses.extend([
            (&primary_tesseract_page, &primary_tesseract_identity),
            (
                &multilingual_tesseract_page,
                &multilingual_tesseract_identity,
            ),
        ]);
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
    }

    let corpus_path = options
        .corpus_root
        .join(format!("{}.jsonl", options.edition));
    let selected_pages: BTreeSet<u32> = options.pages.iter().copied().collect();
    let mut entries = if corpus_path.exists() {
        load_entries(&corpus_path)?
    } else {
        Vec::new()
    };
    entries.retain(|entry| {
        !entry
            .spans()
            .flat_map(|span| span.coordinates.iter())
            .any(|coordinate| selected_pages.contains(&coordinate.source_page))
    });
    for parsed_entry in parsed_pages
        .iter()
        .flat_map(|page| page.entries.iter().cloned())
    {
        entries.retain(|entry| entry.id != parsed_entry.id);
        entries.push(parsed_entry);
    }
    write_entries(&corpus_path, &entries)?;

    let unparsed_lines = parsed_pages
        .iter()
        .flat_map(|page| page.assignments.iter())
        .filter(|(_, _, assignment)| matches!(assignment, LineAssignment::Unparsed))
        .count();
    Ok(RunResult {
        run_id,
        pages: options.pages.to_vec(),
        entries: parsed_pages.iter().map(|page| page.entries.len()).sum(),
        unparsed_lines,
        corpus_path,
        run_path,
    })
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
    let output = Command::new(program)
        .arg("--version")
        .output()
        .with_context(|| format!("failed to execute `{program}`; enter `nix develop`"))?;
    if !output.status.success() {
        bail!("`{program} --version` failed");
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
    use super::parse_page_spec;

    #[test]
    fn page_specs_are_sorted_and_deduplicated() {
        assert_eq!(parse_page_spec("5,1-3,3").unwrap(), vec![1, 2, 3, 5]);
    }

    #[test]
    fn page_specs_reject_zero_and_backwards_ranges() {
        assert!(parse_page_spec("0").is_err());
        assert!(parse_page_spec("9-2").is_err());
    }
}
