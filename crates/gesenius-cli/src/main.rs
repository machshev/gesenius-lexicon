//! Command-line interface for the reproducible Gesenius corpus pipeline.

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use gesenius_core::alto::parse_alto;
use gesenius_core::benchmark::{evaluate_alto, GoldBenchmark};
use gesenius_core::corpus_io::load_entries;
use gesenius_core::export::{
    export, manifest_from_entries, validate_sqlite, validate_tei_schema, ExportFormat,
    ExportOptions,
};
use gesenius_core::model::AccuracyMetrics;
use gesenius_core::pipeline::{parse_page_spec, run_with_progress, RunOptions, RunProgress};
use gesenius_core::report::{compare_editions, write_report};
use gesenius_core::review::{serve, ReviewServerOptions, ReviewStore};
use gesenius_core::source::{
    fetch_source, import_source, verify_source, SourceCatalogue, DEFAULT_CATALOGUE,
};
use gesenius_core::training::{execute_kraken_training, prepare, PilotCatalogue};
use gesenius_core::validate::validate_corpus;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(
    version,
    about = "Reproducible local-first Gesenius OCR and Unicode corpus pipeline"
)]
struct Cli {
    #[arg(long, global = true, default_value = DEFAULT_CATALOGUE)]
    catalogue: PathBuf,
    #[arg(long, global = true, default_value = "pipeline.toml")]
    pipeline_config: PathBuf,
    #[arg(long, global = true, default_value = ".cache/gesenius")]
    cache: PathBuf,
    #[arg(long, global = true, default_value = "corpus/machine")]
    corpus: PathBuf,
    #[arg(long, global = true, default_value = "corpus/review/patches.jsonl")]
    patches: PathBuf,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch, import, or verify immutable source PDFs.
    Source {
        #[command(subcommand)]
        command: SourceCommands,
    },
    /// Run selected PDF pages through rasterization, OCR, and parsing.
    Run(RunArguments),
    /// Measure an ALTO hypothesis against immutable human/frontier gold lines.
    Benchmark(BenchmarkArguments),
    /// Prepare pilot ground truth and optionally fine-tune Kraken.
    Train(TrainArguments),
    /// Validate corpus, Unicode, provenance, and run assignments.
    Validate(ValidateArguments),
    /// Start local review services.
    Review {
        #[command(subcommand)]
        command: ReviewCommands,
    },
    /// Export a materialized reviewed corpus.
    Export(ExportArguments),
    /// Compare edition coverage, quality, and editorial content.
    Report(ReportArguments),
}

#[derive(Subcommand)]
enum SourceCommands {
    /// Download a catalogue source using its direct public URL.
    Fetch {
        /// Registered edition ID.
        #[arg(long)]
        edition: String,
    },
    /// Import an owner-selected local PDF after hash verification.
    Import {
        /// Registered edition ID.
        #[arg(long)]
        edition: String,
        /// PDF path; defaults to local_import_path from the catalogue.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Recompute and compare cached source hashes.
    Verify {
        /// One edition; omit to verify every registered source.
        #[arg(long)]
        edition: Option<String>,
    },
}

#[derive(Args)]
struct RunArguments {
    /// Registered edition ID.
    #[arg(long)]
    edition: String,
    /// One-based PDF pages, for example `17-20,45`.
    #[arg(long)]
    pages: String,
}

#[derive(Args)]
struct BenchmarkArguments {
    /// Human- or frontier-transcribed gold fixture.
    #[arg(long)]
    gold: PathBuf,
    /// ALTO hypothesis to evaluate.
    #[arg(long)]
    alto: PathBuf,
}

#[derive(Args)]
struct TrainArguments {
    /// Fixed 24-page-per-edition pilot definition.
    #[arg(long, default_value = "pilot.toml")]
    pilot: PathBuf,
    /// Ground-truth and model output root.
    #[arg(long, default_value = "training")]
    output: PathBuf,
    /// Run `ketos train` after preparing crops and splits.
    #[arg(long)]
    execute: bool,
    /// Fine-tuned checkpoint output directory, required with `--execute`.
    #[arg(long)]
    output_model: Option<PathBuf>,
    /// Optional Kraken recognition model to fine-tune.
    #[arg(long)]
    base_model: Option<PathBuf>,
}

#[derive(Args)]
struct ValidateArguments {
    /// Also inspect parsed page assignments below this run root.
    #[arg(long)]
    run_root: Option<PathBuf>,
    /// Emit the full machine-readable report as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum ReviewCommands {
    /// Serve scan overlays, OCR hypotheses, diagnostics, and structured edits.
    Serve {
        /// Loopback bind address.
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArgument {
    Jsonl,
    Tei,
    Sqlite,
}

impl From<FormatArgument> for ExportFormat {
    fn from(value: FormatArgument) -> Self {
        match value {
            FormatArgument::Jsonl => Self::Jsonl,
            FormatArgument::Tei => Self::Tei,
            FormatArgument::Sqlite => Self::Sqlite,
        }
    }
}

#[derive(Args)]
struct ExportArguments {
    /// Artifact type.
    #[arg(long, value_enum)]
    format: FormatArgument,
    /// Output directory containing the artifact and manifest.
    #[arg(long)]
    output: PathBuf,
    /// Corpus release version.
    #[arg(long, default_value = "0.1.0-draft")]
    corpus_version: String,
    /// Pilot aggregate accuracy metric JSON.
    #[arg(long)]
    metrics: Option<PathBuf>,
    /// RELAX NG profile used for TEI validation.
    #[arg(long, default_value = "schema/tei-lex0.rng")]
    tei_schema: PathBuf,
}

#[derive(Args)]
struct ReportArguments {
    /// JSON and Markdown report output directory.
    #[arg(long, default_value = "reports")]
    output: PathBuf,
    /// Pilot aggregate accuracy metric JSON.
    #[arg(long)]
    metrics: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Source { command } => source_command(&cli.catalogue, &cli.cache, command),
        Commands::Run(arguments) => run_command(&cli, arguments),
        Commands::Benchmark(arguments) => benchmark_command(arguments),
        Commands::Train(arguments) => train_command(&cli, arguments),
        Commands::Validate(arguments) => validate_command(&cli, arguments),
        Commands::Review { command } => review_command(&cli, command),
        Commands::Export(arguments) => export_command(&cli, arguments),
        Commands::Report(arguments) => report_command(&cli, arguments),
    }
}

fn benchmark_command(arguments: &BenchmarkArguments) -> Result<()> {
    let benchmark = GoldBenchmark::load(&arguments.gold)?;
    let alto = fs::read_to_string(&arguments.alto)
        .with_context(|| format!("failed to read ALTO {}", arguments.alto.display()))?;
    print_json(&evaluate_alto(&benchmark, &parse_alto(&alto)?))
}

fn source_command(catalogue_path: &Path, cache: &Path, command: &SourceCommands) -> Result<()> {
    let catalogue = SourceCatalogue::load(catalogue_path)?;
    match command {
        SourceCommands::Fetch { edition } => {
            print_json(&fetch_source(catalogue.edition(edition)?, cache)?)
        }
        SourceCommands::Import { edition, path } => {
            let source = catalogue.edition(edition)?;
            let input = path
                .clone()
                .or_else(|| source.local_import_path.clone())
                .context("no --path and no local_import_path registered")?;
            print_json(&import_source(source, &input, cache)?)
        }
        SourceCommands::Verify { edition } => {
            let results = if let Some(edition) = edition {
                vec![verify_source(catalogue.edition(edition)?, cache)?]
            } else {
                catalogue
                    .sources
                    .iter()
                    .map(|source| verify_source(source, cache))
                    .collect::<Result<Vec<_>>>()?
            };
            print_json(&results)
        }
    }
}

fn run_command(cli: &Cli, arguments: &RunArguments) -> Result<()> {
    let pages = parse_page_spec(&arguments.pages)?;
    let commit = pipeline_commit();
    let result = run_with_progress(
        &RunOptions {
            edition: &arguments.edition,
            pages: &pages,
            catalogue_path: &cli.catalogue,
            settings_path: &cli.pipeline_config,
            cache_root: &cli.cache,
            corpus_root: &cli.corpus,
            pipeline_commit: &commit,
        },
        print_run_progress,
    )?;
    print_json(&result)
}

fn print_run_progress(progress: RunProgress) {
    if let Some(page_number) = progress.page_number {
        eprintln!(
            "[page {}/{}, PDF {page_number}] {}",
            progress.page_index, progress.page_count, progress.message
        );
    } else {
        eprintln!("[run] {}", progress.message);
    }
}

fn train_command(cli: &Cli, arguments: &TrainArguments) -> Result<()> {
    let entries = materialized_entries(cli)?;
    let pilot = PilotCatalogue::load(&arguments.pilot)?;
    let result = prepare(&entries, &pilot, &arguments.output)?;
    if arguments.execute {
        let output_model = arguments
            .output_model
            .as_deref()
            .context("--output-model is required with --execute")?;
        execute_kraken_training(
            &arguments.output,
            output_model,
            arguments.base_model.as_deref(),
        )?;
    }
    print_json(&result)
}

fn validate_command(cli: &Cli, arguments: &ValidateArguments) -> Result<()> {
    let entries = materialized_entries(cli)?;
    let report = validate_corpus(&entries, arguments.run_root.as_deref());
    if arguments.json {
        print_json(&report)?;
    } else {
        println!(
            "{} entries: {} errors, {} warnings",
            entries.len(),
            report.errors(),
            report.warnings()
        );
        for issue in &report.issues {
            println!(
                "{:?} {} {}: {}",
                issue.severity, issue.code, issue.location, issue.message
            );
        }
    }
    if !report.is_valid() {
        bail!("corpus validation failed");
    }
    Ok(())
}

fn review_command(cli: &Cli, command: &ReviewCommands) -> Result<()> {
    match command {
        ReviewCommands::Serve { bind } => serve(&ReviewServerOptions {
            bind,
            corpus_root: &cli.corpus,
            patch_path: &cli.patches,
            asset_roots: &[cli.cache.clone(), std::env::current_dir()?],
            confidence_threshold: 0.8,
            disagreement_threshold: 0.15,
        }),
    }
}

fn export_command(cli: &Cli, arguments: &ExportArguments) -> Result<()> {
    let entries = materialized_entries(cli)?;
    let metrics = load_metrics(arguments.metrics.as_deref())?;
    let manifest = manifest_from_entries(
        &arguments.corpus_version,
        &pipeline_commit(),
        &entries,
        metrics,
    )?;
    let format = ExportFormat::from(arguments.format);
    let result = export(&ExportOptions {
        format,
        output_directory: &arguments.output,
        entries: &entries,
        manifest: &manifest,
    })?;
    match format {
        ExportFormat::Tei => validate_tei_schema(&result.artifact, &arguments.tei_schema)?,
        ExportFormat::Sqlite => validate_sqlite(&result.artifact)?,
        ExportFormat::Jsonl => {}
    }
    print_json(&json!({
        "artifact": result.artifact,
        "manifest": result.manifest,
        "draft": manifest.draft
    }))
}

fn report_command(cli: &Cli, arguments: &ReportArguments) -> Result<()> {
    let entries = materialized_entries(cli)?;
    let report = compare_editions(&entries, load_metrics(arguments.metrics.as_deref())?);
    write_report(&arguments.output, &report)?;
    print_json(&json!({
        "json": arguments.output.join("edition-comparison.json"),
        "markdown": arguments.output.join("edition-comparison.md")
    }))
}

fn materialized_entries(cli: &Cli) -> Result<Vec<gesenius_core::CorpusEntry>> {
    if cli.patches.exists() {
        ReviewStore::open(&cli.corpus, &cli.patches)?.materialized_entries()
    } else {
        let mut paths: Vec<_> = fs::read_dir(&cli.corpus)
            .with_context(|| format!("failed to read corpus root {}", cli.corpus.display()))?
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
}

fn load_metrics(path: Option<&Path>) -> Result<AccuracyMetrics> {
    path.map_or_else(
        || Ok(AccuracyMetrics::default()),
        |path| {
            serde_json::from_slice(&fs::read(path)?)
                .with_context(|| format!("invalid accuracy metrics {}", path.display()))
        },
    )
}

fn pipeline_commit() -> String {
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    let diff = Command::new("git")
        .args(["diff", "--binary", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_default();
    let untracked = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_default();
    if diff.is_empty() && untracked.is_empty() {
        revision
    } else {
        let mut hasher = Sha256::new();
        hasher.update(&diff);
        for path in String::from_utf8_lossy(&untracked).lines() {
            hasher.update(path.as_bytes());
            if let Ok(content) = fs::read(path) {
                hasher.update(content);
            }
        }
        let digest = format!("{:x}", hasher.finalize());
        format!("{revision}-dirty-{}", &digest[..12])
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
