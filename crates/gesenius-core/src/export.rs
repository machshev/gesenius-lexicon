//! Deterministic JSONL, TEI Lex-0 profile, and versioned SQLite exports.

use crate::corpus_io::{write_entries, write_manifest};
use crate::model::{
    AccuracyMetrics, BlockKind, CorpusEntry, CorpusManifest, Direction, EntryBlock, ReviewState,
    TextSpan, CORPUS_SCHEMA_VERSION, SQLITE_SCHEMA_VERSION,
};
use crate::validate::validate_corpus;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Transaction};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Supported publication artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Canonical one-entry-per-line JSON.
    Jsonl,
    /// TEI Lex-0 XML profile.
    Tei,
    /// Application-independent SQLite database.
    Sqlite,
}

/// Export request.
pub struct ExportOptions<'a> {
    /// Artifact format.
    pub format: ExportFormat,
    /// Output directory.
    pub output_directory: &'a Path,
    /// Materialized entries after review patches.
    pub entries: &'a [CorpusEntry],
    /// Immutable release metadata.
    pub manifest: &'a CorpusManifest,
}

/// Exported files.
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Primary artifact.
    pub artifact: PathBuf,
    /// Sidecar manifest.
    pub manifest: PathBuf,
}

/// Creates an export after corpus validation.
pub fn export(options: &ExportOptions<'_>) -> Result<ExportResult> {
    let report = validate_corpus(options.entries, None);
    if !report.is_valid() {
        bail!(
            "refusing to export corpus with {} validation errors",
            report.errors()
        );
    }
    fs::create_dir_all(options.output_directory)?;
    let artifact = match options.format {
        ExportFormat::Jsonl => {
            let path = options.output_directory.join("corpus.jsonl");
            write_entries(&path, options.entries)?;
            path
        }
        ExportFormat::Tei => {
            let path = options.output_directory.join("corpus.tei.xml");
            write_tei(&path, options.entries, options.manifest)?;
            path
        }
        ExportFormat::Sqlite => {
            let path = options.output_directory.join("corpus.sqlite3");
            write_sqlite(&path, options.entries, options.manifest)?;
            path
        }
    };
    let manifest_path = options.output_directory.join("manifest.json");
    write_manifest(&manifest_path, options.manifest)?;
    Ok(ExportResult {
        artifact,
        manifest: manifest_path,
    })
}

/// Constructs release metadata from the materialized corpus.
pub fn manifest_from_entries(
    corpus_version: &str,
    pipeline_commit: &str,
    entries: &[CorpusEntry],
    metrics: AccuracyMetrics,
) -> Result<CorpusManifest> {
    let mut source_hashes = BTreeMap::new();
    let mut model_hashes = BTreeMap::new();
    for entry in entries {
        if let Some(previous) = source_hashes.insert(
            entry.edition.clone(),
            entry.provenance.source_sha256.clone(),
        ) {
            if previous != entry.provenance.source_sha256 {
                bail!(
                    "edition `{}` contains multiple source hashes",
                    entry.edition
                );
            }
        }
        for hypothesis in entry.spans().flat_map(|span| span.hypotheses.iter()) {
            let identity = format!("{}:{}", hypothesis.engine, hypothesis.model);
            if let Some(previous) =
                model_hashes.insert(identity.clone(), hypothesis.model_hash.clone())
            {
                if previous != hypothesis.model_hash {
                    bail!("model identity `{identity}` contains multiple hashes");
                }
            }
        }
    }
    Ok(CorpusManifest {
        corpus_version: corpus_version.to_owned(),
        schema_version: CORPUS_SCHEMA_VERSION.to_owned(),
        pipeline_commit: pipeline_commit.to_owned(),
        generated_at: reproducible_timestamp()?,
        source_hashes,
        model_hashes,
        metrics,
        draft: entries
            .iter()
            .any(|entry| entry.review_state != ReviewState::Verified),
    })
}

/// Validates generated TEI against the checked-in RELAX NG schema.
pub fn validate_tei_schema(tei_path: &Path, schema_path: &Path) -> Result<()> {
    let output = Command::new("xmllint")
        .args(["--noout", "--relaxng"])
        .arg(schema_path)
        .arg(tei_path)
        .output()
        .context("failed to execute xmllint; enter `nix develop`")?;
    if !output.status.success() {
        bail!(
            "TEI schema validation failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Checks SQLite integrity and all foreign keys.
pub fn validate_sqlite(path: &Path) -> Result<()> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        bail!("SQLite integrity check failed: {integrity}");
    }
    let foreign_key_violations: i64 =
        connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_violations != 0 {
        bail!("SQLite has {foreign_key_violations} foreign-key violations");
    }
    Ok(())
}

fn write_tei(path: &Path, entries: &[CorpusEntry], manifest: &CorpusManifest) -> Result<()> {
    let mut output = String::with_capacity(entries.len() * 1024);
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    output.push_str(
        "<?xml-model href=\"tei-lex0.rng\" type=\"application/xml\" schematypens=\"http://relaxng.org/ns/structure/1.0\"?>\n",
    );
    output.push_str("<TEI xmlns=\"http://www.tei-c.org/ns/1.0\"><teiHeader>");
    output.push_str(
        "<fileDesc><titleStmt><title>Gesenius Hebrew Lexicon OCR corpus</title></titleStmt>",
    );
    output.push_str("<publicationStmt><p>Machine-readable OCR draft; review state is attached to each entry and span.</p></publicationStmt>");
    output.push_str("<sourceDesc><listBibl>");
    for (edition, hash) in &manifest.source_hashes {
        let _ = write!(
            output,
            "<bibl xml:id=\"source_{}\"><title>{}</title><idno type=\"sha256\">{}</idno></bibl>",
            xml_id(edition),
            xml_escape(edition),
            xml_escape(hash)
        );
    }
    output.push_str("</listBibl></sourceDesc></fileDesc>");
    let _ = write!(
        output,
        "<encodingDesc><projectDesc><p>Corpus version {}; schema {}; pipeline {}; draft={}.</p></projectDesc></encodingDesc>",
        xml_escape(&manifest.corpus_version),
        xml_escape(&manifest.schema_version),
        xml_escape(&manifest.pipeline_commit),
        manifest.draft
    );
    let _ = write!(
        output,
        "<revisionDesc status=\"{}\"><change when=\"{}\">Generated deterministic export</change></revisionDesc>",
        if manifest.draft { "draft" } else { "published" },
        manifest.generated_at.to_rfc3339()
    );
    output.push_str("</teiHeader><text><body>");

    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|left, right| left.id.cmp(&right.id));
    for entry in sorted {
        let _ = write!(
            output,
            "<entry xml:id=\"{}\" n=\"{}\" type=\"{}\" source=\"#source_{}\">",
            xml_id(&entry.id),
            xml_escape(&entry.id),
            entry.review_state.as_str(),
            xml_id(&entry.edition)
        );
        if let Some(headword) = &entry.headword {
            output.push_str("<form type=\"lemma\">");
            write_span(&mut output, "orth", headword);
            if let Some(homograph) = entry.homograph {
                let _ = write!(output, "<num type=\"homograph\">{homograph}</num>");
            }
            output.push_str("</form>");
        }
        for label in &entry.grammatical_labels {
            write_span(&mut output, "gram", label);
        }
        for block in &entry.blocks {
            write_block(&mut output, block);
        }
        for sense in &entry.senses {
            let _ = write!(
                output,
                "<sense xml:id=\"{}\"{}>",
                xml_id(&sense.id),
                sense
                    .label
                    .as_ref()
                    .map_or_else(String::new, |label| format!(" n=\"{}\"", xml_escape(label)))
            );
            for block in &sense.blocks {
                write_block(&mut output, block);
            }
            output.push_str("</sense>");
        }
        for citation in &entry.citations {
            output.push_str("<cit type=\"example\">");
            write_span(&mut output, "quote", &citation.text);
            if let Some(target) = &citation.target {
                let _ = write!(
                    output,
                    "<ref type=\"citation\" target=\"{}\"/>",
                    xml_escape(target)
                );
            }
            output.push_str("</cit>");
        }
        for reference in &entry.cross_references {
            output.push_str("<xr>");
            let target = reference
                .target_entry_id
                .as_ref()
                .map_or_else(String::new, |id| format!(" target=\"#{}\"", xml_id(id)));
            let _ = write!(output, "<ref{target}>");
            write_span_content(&mut output, &reference.text);
            output.push_str("</ref></xr>");
        }
        for block in &entry.etymology {
            output.push_str("<etym>");
            for span in &block.spans {
                write_span(&mut output, "seg", span);
            }
            output.push_str("</etym>");
        }
        let _ = write!(
            output,
            "<note type=\"provenance\">printed-page={} entry-ordinal={} scan={} source-sha256={} pipeline-run={} confidence={:.4} revision={}</note>",
            xml_escape(&entry.printed_page),
            entry.entry_ordinal,
            xml_escape(&entry.provenance.scan_id),
            xml_escape(&entry.provenance.source_sha256),
            xml_escape(&entry.provenance.pipeline_run),
            entry.confidence,
            entry.revision
        );
        output.push_str("</entry>");
    }
    output.push_str("</body></text></TEI>\n");
    fs::write(path, output)?;
    Ok(())
}

fn write_block(output: &mut String, block: &EntryBlock) {
    let kind = block_kind(block.kind);
    let _ = write!(
        output,
        "<note xml:id=\"{}\" type=\"{}\">",
        xml_id(&block.id),
        kind
    );
    for span in &block.spans {
        write_span(output, "seg", span);
    }
    output.push_str("</note>");
}

fn write_span(output: &mut String, element: &str, span: &TextSpan) {
    let language = span.language.as_ref().map_or_else(String::new, |language| {
        format!(" xml:lang=\"{}\"", xml_escape(language))
    });
    let _ = write!(
        output,
        "<{element} xml:id=\"{}\"{language} type=\"script:{} direction:{} review:{}\" cert=\"{:.4}\">",
        xml_id(&span.id),
        xml_escape(&span.script),
        direction(span.direction),
        span.review_state.as_str(),
        span.confidence
    );
    write_span_content(output, span);
    let _ = write!(output, "</{element}>");
}

fn write_span_content(output: &mut String, span: &TextSpan) {
    if span.diplomatic == span.normalized {
        output.push_str(&xml_escape(&span.normalized));
    } else {
        let _ = write!(
            output,
            "<choice><orig>{}</orig><reg>{}</reg></choice>",
            xml_escape(&span.diplomatic),
            xml_escape(&span.normalized)
        );
    }
}

fn write_sqlite(path: &Path, entries: &[CorpusEntry], manifest: &CorpusManifest) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to replace generated {}", path.display()))?;
    }
    let mut connection = Connection::open(path)?;
    connection.execute_batch(
        "PRAGMA page_size=4096;
         PRAGMA journal_mode=OFF;
         PRAGMA synchronous=OFF;
         PRAGMA foreign_keys=ON;
         PRAGMA encoding='UTF-8';",
    )?;
    let transaction = connection.transaction()?;
    create_schema(&transaction)?;
    insert_manifest(&transaction, manifest)?;
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|left, right| left.id.cmp(&right.id));
    for entry in sorted {
        insert_entry(&transaction, entry)?;
    }
    transaction.commit()?;
    connection.execute_batch("VACUUM; PRAGMA optimize;")?;
    validate_sqlite(path)
}

fn create_schema(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(include_str!("../../../schema/sqlite-v1.sql"))?;
    Ok(())
}

fn insert_manifest(transaction: &Transaction<'_>, manifest: &CorpusManifest) -> Result<()> {
    let values = [
        (
            "corpus_version",
            serde_json::to_string(&manifest.corpus_version)?,
        ),
        (
            "schema_version",
            serde_json::to_string(&manifest.schema_version)?,
        ),
        (
            "sqlite_schema_version",
            serde_json::to_string(&SQLITE_SCHEMA_VERSION)?,
        ),
        (
            "pipeline_commit",
            serde_json::to_string(&manifest.pipeline_commit)?,
        ),
        (
            "generated_at",
            serde_json::to_string(&manifest.generated_at.to_rfc3339())?,
        ),
        ("draft", serde_json::to_string(&manifest.draft)?),
        (
            "source_hashes",
            serde_json::to_string(&manifest.source_hashes)?,
        ),
        (
            "model_hashes",
            serde_json::to_string(&manifest.model_hashes)?,
        ),
        ("metrics", serde_json::to_string(&manifest.metrics)?),
    ];
    for (key, value) in values {
        transaction.execute(
            "INSERT INTO metadata(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    Ok(())
}

fn insert_entry(transaction: &Transaction<'_>, entry: &CorpusEntry) -> Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO editions(id, source_sha256, scan_id) VALUES (?1, ?2, ?3)",
        params![
            entry.edition,
            entry.provenance.source_sha256,
            entry.provenance.scan_id
        ],
    )?;
    transaction.execute(
        "INSERT INTO entries(
          id, edition_id, printed_page, entry_ordinal, headword_diplomatic,
          headword_normalized, homograph, confidence, review_state, revision, pipeline_run
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            entry.id,
            entry.edition,
            entry.printed_page,
            entry.entry_ordinal,
            entry.headword.as_ref().map(|span| &span.diplomatic),
            entry.headword.as_ref().map(|span| &span.normalized),
            entry.homograph,
            entry.confidence,
            entry.review_state.as_str(),
            entry.revision,
            entry.provenance.pipeline_run
        ],
    )?;
    for alias in &entry.aliases {
        transaction.execute(
            "INSERT INTO aliases(alias, entry_id) VALUES (?1, ?2)",
            params![alias, entry.id],
        )?;
    }

    let mut span_ids = BTreeMap::new();
    if let Some(headword) = &entry.headword {
        insert_span(transaction, entry, None, "headword", 0, headword)?;
        span_ids.insert(headword.id.clone(), ());
    }
    for (ordinal, span) in entry.grammatical_labels.iter().enumerate() {
        insert_span(transaction, entry, None, "grammar", ordinal, span)?;
        span_ids.insert(span.id.clone(), ());
    }
    for (ordinal, block) in entry.blocks.iter().enumerate() {
        insert_block(
            transaction,
            entry,
            None,
            "entry",
            ordinal,
            block,
            &mut span_ids,
        )?;
    }
    for (sense_ordinal, sense) in entry.senses.iter().enumerate() {
        transaction.execute(
            "INSERT INTO senses(id, entry_id, ordinal, label) VALUES (?1,?2,?3,?4)",
            params![sense.id, entry.id, sense_ordinal, sense.label],
        )?;
        for (block_ordinal, block) in sense.blocks.iter().enumerate() {
            insert_block(
                transaction,
                entry,
                Some(&sense.id),
                "sense",
                block_ordinal,
                block,
                &mut span_ids,
            )?;
        }
    }
    for (ordinal, block) in entry.etymology.iter().enumerate() {
        insert_block(
            transaction,
            entry,
            None,
            "etymology",
            ordinal,
            block,
            &mut span_ids,
        )?;
    }
    for (ordinal, citation) in entry.citations.iter().enumerate() {
        if !span_ids.contains_key(&citation.text.id) {
            insert_span(
                transaction,
                entry,
                None,
                "citation",
                ordinal,
                &citation.text,
            )?;
            span_ids.insert(citation.text.id.clone(), ());
        }
        transaction.execute(
            "INSERT INTO citations(id, entry_id, ordinal, target, span_id) VALUES (?1,?2,?3,?4,?5)",
            params![
                citation.id,
                entry.id,
                ordinal,
                citation.target,
                citation.text.id
            ],
        )?;
    }
    for (ordinal, reference) in entry.cross_references.iter().enumerate() {
        if !span_ids.contains_key(&reference.text.id) {
            insert_span(
                transaction,
                entry,
                None,
                "cross_reference",
                ordinal,
                &reference.text,
            )?;
            span_ids.insert(reference.text.id.clone(), ());
        }
        transaction.execute(
            "INSERT INTO cross_references(id, entry_id, ordinal, target_entry_id, span_id)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                reference.id,
                entry.id,
                ordinal,
                reference.target_entry_id,
                reference.text.id
            ],
        )?;
    }

    let english = entry
        .spans()
        .map(|span| span.normalized.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    transaction.execute(
        "INSERT INTO entry_fts(entry_id, headword, english) VALUES (?1,?2,?3)",
        params![
            entry.id,
            entry.headword.as_ref().map(|span| &span.normalized),
            english
        ],
    )?;
    Ok(())
}

fn insert_block(
    transaction: &Transaction<'_>,
    entry: &CorpusEntry,
    sense_id: Option<&str>,
    role: &str,
    ordinal: usize,
    block: &EntryBlock,
    span_ids: &mut BTreeMap<String, ()>,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO blocks(id, entry_id, sense_id, role, ordinal) VALUES (?1,?2,?3,?4,?5)",
        params![block.id, entry.id, sense_id, role, ordinal],
    )?;
    for (span_ordinal, span) in block.spans.iter().enumerate() {
        if span_ids.insert(span.id.clone(), ()).is_some() {
            bail!("span `{}` is stored in multiple structural roles", span.id);
        }
        insert_span(
            transaction,
            entry,
            Some(&block.id),
            block_kind(block.kind),
            span_ordinal,
            span,
        )?;
    }
    Ok(())
}

fn insert_span(
    transaction: &Transaction<'_>,
    entry: &CorpusEntry,
    block_id: Option<&str>,
    role: &str,
    ordinal: usize,
    span: &TextSpan,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO spans(
           id,entry_id,block_id,role,ordinal,diplomatic,normalized,language,script,
           direction,confidence,review_state
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            span.id,
            entry.id,
            block_id,
            role,
            ordinal,
            span.diplomatic,
            span.normalized,
            span.language,
            span.script,
            direction(span.direction),
            span.confidence,
            span.review_state.as_str()
        ],
    )?;
    for (hypothesis_ordinal, hypothesis) in span.hypotheses.iter().enumerate() {
        transaction.execute(
            "INSERT INTO ocr_hypotheses(
               span_id,ordinal,engine,engine_version,model,model_hash,text,confidence
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                span.id,
                hypothesis_ordinal,
                hypothesis.engine,
                hypothesis.engine_version,
                hypothesis.model,
                hypothesis.model_hash,
                hypothesis.text,
                hypothesis.confidence
            ],
        )?;
    }
    for (coordinate_ordinal, coordinate) in span.coordinates.iter().enumerate() {
        transaction.execute(
            "INSERT INTO source_coordinates(
               span_id,ordinal,source_page,printed_page,region_id,line_id,polygon_json,
               transform_id,page_image
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                span.id,
                coordinate_ordinal,
                coordinate.source_page,
                coordinate.printed_page,
                coordinate.region_id,
                coordinate.line_id,
                serde_json::to_string(&coordinate.polygon)?,
                coordinate.transform_id,
                coordinate.page_image
            ],
        )?;
    }
    for (warning_ordinal, warning) in span.warnings.iter().enumerate() {
        transaction.execute(
            "INSERT INTO unicode_warnings(
               span_id,ordinal,code_point,character_offset,code,message
             ) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                span.id,
                warning_ordinal,
                warning.code_point,
                warning.character_offset,
                warning.code,
                warning.message
            ],
        )?;
    }
    Ok(())
}

fn reproducible_timestamp() -> Result<DateTime<Utc>> {
    if let Ok(value) = std::env::var("SOURCE_DATE_EPOCH") {
        let seconds = value
            .parse::<i64>()
            .context("SOURCE_DATE_EPOCH must be integer seconds")?;
        return DateTime::from_timestamp(seconds, 0).context("SOURCE_DATE_EPOCH is out of range");
    }
    Ok(Utc::now())
}

const fn direction(value: Direction) -> &'static str {
    match value {
        Direction::Ltr => "ltr",
        Direction::Rtl => "rtl",
        Direction::Mixed => "mixed",
    }
}

const fn block_kind(value: BlockKind) -> &'static str {
    match value {
        BlockKind::Form => "form",
        BlockKind::Grammar => "grammar",
        BlockKind::Definition => "definition",
        BlockKind::Etymology => "etymology",
        BlockKind::Citation => "citation",
        BlockKind::CrossReference => "cross_reference",
        BlockKind::Unclassified => "unclassified",
    }
}

fn xml_id(value: &str) -> String {
    let mut result = String::from("g_");
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
            result.push(character);
        } else {
            let _ = write!(result, "_{:x}_", u32::from(character));
        }
    }
    result
}

fn xml_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    output
}
