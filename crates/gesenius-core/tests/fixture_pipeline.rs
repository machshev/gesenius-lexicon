//! End-to-end synthetic fixture pipeline and deterministic export tests.

use chrono::{DateTime, Utc};
use gesenius_core::alto::{
    parse_alto, parse_entries, parse_entries_continuing, EngineIdentity, LineAssignment,
    ParseContext,
};
use gesenius_core::corpus_io::{load_entries, write_entries};
use gesenius_core::export::{
    export, validate_sqlite, validate_tei_schema, ExportFormat, ExportOptions,
};
use gesenius_core::model::{
    AccuracyMetrics, BlockKind, CorpusManifest, ReviewState, CORPUS_SCHEMA_VERSION,
};
use gesenius_core::pipeline::PipelineSettings;
use gesenius_core::review::ReviewStore;
use gesenius_core::source::SourceCatalogue;
use gesenius_core::training::PilotCatalogue;
use gesenius_core::validate::validate_corpus;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn engine(name: &str) -> EngineIdentity {
    EngineIdentity {
        engine: name.to_owned(),
        version: "fixture-1".to_owned(),
        model: format!("{name}-fixture"),
        model_hash: if name == "kraken" {
            "b".repeat(64)
        } else {
            "c".repeat(64)
        },
    }
}

fn context<'a>(
    edition: &'a str,
    printed_page: &'a str,
    source_page: u32,
    image: &'a str,
) -> ParseContext<'a> {
    ParseContext {
        edition,
        printed_page,
        source_page,
        source_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        scan_id: "fixture-scan",
        pipeline_run: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        page_image: image,
        transform_id: "identity-fixture",
        front_matter: false,
    }
}

fn fixture_entries() -> Vec<gesenius_core::CorpusEntry> {
    let kraken = engine("kraken");
    let tesseract = engine("tesseract");
    let robinson_primary = parse_alto(include_str!(
        "../../../fixtures/alto/robinson-p001.kraken.xml"
    ))
    .unwrap();
    let robinson_secondary = parse_alto(include_str!(
        "../../../fixtures/alto/robinson-p001.tesseract.xml"
    ))
    .unwrap();
    let page_one = parse_entries(
        (&robinson_primary, &kraken),
        Some((&robinson_secondary, &tesseract)),
        &context(
            "robinson-1854",
            "1",
            17,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );
    assert_eq!(page_one.entries.len(), 2);
    assert!(page_one
        .assignments
        .iter()
        .any(|(_, _, assignment)| matches!(assignment, LineAssignment::Unparsed)));
    assert_eq!(page_one.entries[0].blocks[0].spans[0].hypotheses.len(), 2);

    let page_two_alto = parse_alto(include_str!(
        "../../../fixtures/alto/robinson-p002.kraken.xml"
    ))
    .unwrap();
    let page_two = parse_entries_continuing(
        (&page_two_alto, &kraken),
        None,
        &context(
            "robinson-1854",
            "2",
            18,
            "fixtures/pages/robinson-damaged.pgm",
        ),
        page_one.entries.last().cloned(),
    );
    assert_eq!(page_two.entries.len(), 2);
    assert_eq!(page_two.entries[0].blocks.len(), 2);
    assert_eq!(page_two.entries[0].blocks[0].kind, BlockKind::Paragraph);
    assert_eq!(page_two.entries[0].blocks[0].spans.len(), 2);
    assert!(page_two.entries[0]
        .blocks
        .iter()
        .flat_map(|block| &block.spans)
        .any(|span| span.diplomatic.ends_with("page.")));

    let tregelles_alto = parse_alto(include_str!(
        "../../../fixtures/alto/tregelles-p001.kraken.xml"
    ))
    .unwrap();
    let tregelles = parse_entries(
        (&tregelles_alto, &kraken),
        None,
        &context(
            "tregelles-1857",
            "1",
            12,
            "fixtures/pages/tregelles-columns.pgm",
        ),
    );

    vec![
        page_one.entries[0].clone(),
        page_two.entries[0].clone(),
        page_two.entries[1].clone(),
        tregelles.entries[0].clone(),
    ]
}

#[test]
fn leading_non_margin_content_is_retained_in_a_headless_entry() {
    let alto = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="header" HPOS="50" VPOS="10" WIDTH="900" HEIGHT="25">
      <TextLine ID="header-line" HPOS="50" VPOS="10" WIDTH="900" HEIGHT="25">
        <String CONTENT="LEXICON" WC="0.99" HPOS="400" VPOS="10" WIDTH="200" HEIGHT="25"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="column-1" HPOS="50" VPOS="100" WIDTH="420" HEIGHT="1000">
      <TextLine ID="introduction" HPOS="50" VPOS="100" WIDTH="400" HEIGHT="45">
        <String CONTENT="The" WC="0.98" HPOS="50" VPOS="100" WIDTH="45" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="name" WC="0.98" HPOS="103" VPOS="100" WIDTH="60" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="Aleph." WC="0.98" HPOS="171" VPOS="100" WIDTH="75" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="entry" HPOS="50" VPOS="155" WIDTH="400" HEIGHT="45">
        <String CONTENT="אָב" WC="0.96" HPOS="50" VPOS="155" WIDTH="60" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="father." WC="0.98" HPOS="118" VPOS="155" WIDTH="90" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let parsed = parse_entries(
        (&alto, &engine("tesseract")),
        None,
        &context(
            "robinson-1854",
            "1",
            17,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );

    assert_eq!(parsed.entries.len(), 2);
    assert!(parsed.entries[0].headword.is_none());
    assert_eq!(
        parsed.entries[0].blocks[0].spans[0].diplomatic,
        "The name Aleph."
    );
    assert_eq!(
        parsed.entries[1]
            .headword
            .as_ref()
            .map(|headword| headword.diplomatic.as_str()),
        Some("אָב")
    );
    assert!(parsed.assignments.iter().any(|(_, line, assignment)| {
        line == "header-line" && matches!(assignment, LineAssignment::Unparsed)
    }));
    assert!(parsed.assignments.iter().any(|(_, line, assignment)| {
        line == "introduction" && matches!(assignment, LineAssignment::Entry(_))
    }));
}

#[test]
fn displayed_headings_and_multiline_paragraphs_are_structured() {
    let alto = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="title" HPOS="300" VPOS="120" WIDTH="400" HEIGHT="50">
      <TextLine ID="title-line" HPOS="350" VPOS="120" WIDTH="300" HEIGHT="50">
        <String CONTENT="LEXICON." WC="0.99" HPOS="350" VPOS="120" WIDTH="300" HEIGHT="50"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="body" HPOS="50" VPOS="240" WIDTH="420" HEIGHT="110">
      <TextLine ID="body-1" HPOS="90" VPOS="240" WIDTH="360" HEIGHT="45">
        <String CONTENT="The first line of a paragraph" WC="0.98" HPOS="90" VPOS="240" WIDTH="360" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="body-2" HPOS="50" VPOS="290" WIDTH="400" HEIGHT="45">
        <String CONTENT="continues on the next line." WC="0.98" HPOS="50" VPOS="290" WIDTH="400" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let parsed = parse_entries(
        (&alto, &engine("tesseract")),
        None,
        &context(
            "robinson-1854",
            "1",
            17,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );

    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].blocks.len(), 2);
    assert_eq!(parsed.entries[0].blocks[0].kind, BlockKind::Heading);
    assert_eq!(parsed.entries[0].blocks[1].kind, BlockKind::Paragraph);
    assert_eq!(parsed.entries[0].blocks[1].spans.len(), 2);
    assert_ne!(
        parsed.entries[0].blocks[1].spans[0].id,
        parsed.entries[0].blocks[1].spans[1].id
    );
}

fn fixture_manifest() -> CorpusManifest {
    CorpusManifest {
        corpus_version: "fixture-1".to_owned(),
        schema_version: CORPUS_SCHEMA_VERSION.to_owned(),
        pipeline_commit: "fixture-commit".to_owned(),
        generated_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
        source_hashes: BTreeMap::from([
            ("robinson-1854".to_owned(), "a".repeat(64)),
            ("tregelles-1857".to_owned(), "a".repeat(64)),
        ]),
        model_hashes: BTreeMap::from([
            ("kraken:kraken-fixture".to_owned(), "b".repeat(64)),
            ("tesseract:tesseract-fixture".to_owned(), "c".repeat(64)),
        ]),
        metrics: AccuracyMetrics::default(),
        draft: true,
    }
}

#[test]
fn fixture_pipeline_validates_and_exports_deterministically() {
    let entries = fixture_entries();
    let report = validate_corpus(&entries, None);
    assert_eq!(report.errors(), 0, "{:#?}", report.issues);

    let temporary = tempfile::tempdir().unwrap();
    let first = temporary.path().join("first");
    let second = temporary.path().join("second");
    for format in [ExportFormat::Jsonl, ExportFormat::Tei, ExportFormat::Sqlite] {
        let first_directory = first.join(format!("{format:?}"));
        let second_directory = second.join(format!("{format:?}"));
        let first_result = export(&ExportOptions {
            format,
            output_directory: &first_directory,
            entries: &entries,
            manifest: &fixture_manifest(),
        })
        .unwrap();
        let second_result = export(&ExportOptions {
            format,
            output_directory: &second_directory,
            entries: &entries,
            manifest: &fixture_manifest(),
        })
        .unwrap();
        assert_eq!(
            fs::read(first_result.artifact).unwrap(),
            fs::read(second_result.artifact).unwrap()
        );
        if format == ExportFormat::Tei
            && std::env::var("GESENIUS_VALIDATE_TEI_EXTERNAL").as_deref() == Ok("1")
        {
            validate_tei_schema(
                &first_directory.join("corpus.tei.xml"),
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/tei-lex0.rng"),
            )
            .unwrap();
        }
    }

    let database = first.join("Sqlite/corpus.sqlite3");
    validate_sqlite(&database).unwrap();
    let connection = Connection::open(database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM entries", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM entry_fts WHERE entry_fts MATCH 'father'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn review_patches_use_optimistic_revisions_and_preserve_the_base() {
    let mut entries = fixture_entries();
    let temporary = tempfile::tempdir().unwrap();
    let corpus_root = temporary.path().join("machine");
    fs::create_dir_all(&corpus_root).unwrap();
    let base_path = corpus_root.join("fixtures.jsonl");
    write_entries(&base_path, &entries).unwrap();
    let original = fs::read(&base_path).unwrap();

    let store =
        ReviewStore::open(&corpus_root, &temporary.path().join("review/patches.jsonl")).unwrap();
    let entry = entries.remove(0);
    let mut replacement = entry.clone();
    replacement.blocks[0].spans[0]
        .diplomatic
        .push_str(" corrected");
    let patch = store
        .apply(
            0,
            "fixture-reviewer",
            Some("fixture correction".to_owned()),
            ReviewState::Corrected,
            replacement.clone(),
        )
        .unwrap();
    assert_eq!(patch.revision, 1);
    assert!(store
        .apply(
            0,
            "stale-reviewer",
            None,
            ReviewState::Corrected,
            replacement
        )
        .unwrap_err()
        .to_string()
        .contains("revision conflict"));
    assert_eq!(fs::read(&base_path).unwrap(), original);
    assert_eq!(
        store
            .materialized_entries()
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == entry.id)
            .unwrap()
            .revision,
        1
    );
    assert_eq!(load_entries(&base_path).unwrap()[0].revision, 0);
}

#[test]
fn checked_in_page_crops_are_real_images() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in [
        "fixtures/pages/robinson-damaged.pgm",
        "fixtures/pages/tregelles-columns.pgm",
    ] {
        let content = fs::read_to_string(workspace.join(path)).unwrap();
        assert!(content.starts_with("P2\n"));
        assert!(content.lines().count() > 12);
    }
}

#[test]
fn checked_in_catalogues_are_complete_and_pinned() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sources = SourceCatalogue::load(&workspace.join("sources.toml")).unwrap();
    assert_eq!(
        sources.edition("robinson-1854").unwrap().sha256,
        "466b061e770f212cb7d888d8dadc2a54575fb115bf6de9cdb24b0c280461ccaa"
    );
    let pilot = PilotCatalogue::load(&workspace.join("pilot.toml")).unwrap();
    assert_eq!(pilot.editions.len(), 2);
    assert!(pilot
        .editions
        .iter()
        .all(|edition| edition.pages.len() == 24));
    let pipeline = PipelineSettings::load(&workspace.join("pipeline.toml")).unwrap();
    assert_eq!(pipeline.raster_dpi, 400);
}
