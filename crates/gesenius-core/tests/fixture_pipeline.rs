//! End-to-end synthetic fixture pipeline and deterministic export tests.

use chrono::{DateTime, Utc};
use gesenius_core::alto::{
    classify_word_languages, parse_alto, parse_entries, parse_entries_continuing, EngineIdentity,
    LineAssignment, ParseContext,
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
    assert_eq!(page_two.entries[0].blocks.len(), 1);
    assert_eq!(page_two.entries[0].blocks[0].kind, BlockKind::Paragraph);
    assert_eq!(page_two.entries[0].blocks[0].spans.len(), 3);
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
      <TextLine ID="entry" HPOS="90" VPOS="155" WIDTH="360" HEIGHT="45">
        <String CONTENT="אָב" WC="0.96" HPOS="90" VPOS="155" WIDTH="60" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="father." WC="0.98" HPOS="158" VPOS="155" WIDTH="90" HEIGHT="45"/>
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

#[test]
fn short_citation_near_the_column_gutter_is_not_a_heading() {
    let alto = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="right-column" HPOS="510" VPOS="100" WIDTH="400" HEIGHT="100">
      <TextLine ID="sense-opening" HPOS="550" VPOS="100" WIDTH="360" HEIGHT="45">
        <String CONTENT="2. A numbered sense starts here" WC="0.98" HPOS="550" VPOS="100" WIDTH="360" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="short-citation" HPOS="510" VPOS="150" WIDTH="130" HEIGHT="45">
      <TextLine ID="citation-line" HPOS="510" VPOS="150" WIDTH="130" HEIGHT="45">
        <String CONTENT="Jer. 23, 1." WC="0.98" HPOS="510" VPOS="150" WIDTH="130" HEIGHT="45"/>
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
            "3",
            19,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );

    assert_eq!(parsed.entries[0].blocks.len(), 1);
    assert_eq!(parsed.entries[0].blocks[0].kind, BlockKind::Paragraph);
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 2);
}

#[test]
fn indented_line_at_normal_spacing_does_not_split_a_paragraph() {
    let alto = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="body-1" HPOS="50" VPOS="100" WIDTH="420" HEIGHT="100">
      <TextLine ID="opening" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45">
        <String CONTENT="The paragraph begins here" WC="0.98" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="continuation" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45">
        <String CONTENT="and continues without a break." WC="0.98" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="body-2" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="100">
      <TextLine ID="inset-continuation" HPOS="90" VPOS="200" WIDTH="380" HEIGHT="45">
        <String CONTENT="This inset is still normally spaced" WC="0.98" HPOS="90" VPOS="200" WIDTH="380" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="final-line" HPOS="50" VPOS="250" WIDTH="420" HEIGHT="45">
        <String CONTENT="and belongs to the same paragraph." WC="0.98" HPOS="50" VPOS="250" WIDTH="420" HEIGHT="45"/>
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
    assert_eq!(parsed.entries[0].blocks.len(), 1);
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 4);
}

#[test]
fn page_column_margin_recovers_indentation_from_tightly_cropped_regions() {
    let alto = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="first-paragraph" HPOS="50" VPOS="100" WIDTH="420" HEIGHT="150">
      <TextLine ID="first-opening" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45">
        <String CONTENT="The first paragraph begins here" WC="0.98" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="first-continuation" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45">
        <String CONTENT="and continues at the column margin" WC="0.98" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="first-final" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45">
        <String CONTENT="for another normally spaced line." WC="0.98" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="tightly-cropped-second" HPOS="90" VPOS="250" WIDTH="380" HEIGHT="45">
      <TextLine ID="second-opening" HPOS="90" VPOS="250" WIDTH="380" HEIGHT="45">
        <String CONTENT="The second paragraph is indented" WC="0.98" HPOS="90" VPOS="250" WIDTH="380" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="second-continuation" HPOS="50" VPOS="300" WIDTH="420" HEIGHT="45">
      <TextLine ID="second-final" HPOS="50" VPOS="300" WIDTH="420" HEIGHT="45">
        <String CONTENT="and continues at the column margin." WC="0.98" HPOS="50" VPOS="300" WIDTH="420" HEIGHT="45"/>
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
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 3);
    assert_eq!(parsed.entries[0].blocks[1].spans.len(), 2);
}

#[test]
fn ocr_tokens_with_digits_are_not_grammar_labels() {
    let alto = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="body" HPOS="50" VPOS="100" WIDTH="420" HEIGHT="150">
      <TextLine ID="headword" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45">
        <String CONTENT="אָב" WC="0.98" HPOS="90" VPOS="100" WIDTH="60" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="m." WC="0.98" HPOS="158" VPOS="100" WIDTH="30" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="father" WC="0.98" HPOS="196" VPOS="100" WIDTH="100" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="continuation" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45">
        <String CONTENT="the explanation continues with examples" WC="0.98" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="ocr-shaped-continuation" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45">
        <String CONTENT="P82" WC="0.30" HPOS="50" VPOS="200" WIDTH="50" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="and" WC="0.98" HPOS="108" VPOS="200" WIDTH="45" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="M72" WC="0.30" HPOS="161" VPOS="200" WIDTH="50" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="ordinary-in-hebrew-prose" HPOS="50" VPOS="250" WIDTH="420" HEIGHT="45">
        <String CONTENT="So" WC="0.98" HPOS="50" VPOS="250" WIDTH="35" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="in" WC="0.98" HPOS="93" VPOS="250" WIDTH="25" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="Heb." WC="0.98" HPOS="126" VPOS="250" WIDTH="50" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="only" WC="0.98" HPOS="184" VPOS="250" WIDTH="50" HEIGHT="45"/>
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
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 4);
}

#[test]
fn stem_label_hypothesis_keeps_an_indented_sense_in_its_entry() {
    let canonical = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="body" HPOS="50" VPOS="100" WIDTH="420" HEIGHT="250">
      <TextLine ID="headword" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45">
        <String CONTENT="אָבַד" WC="0.98" HPOS="90" VPOS="100" WIDTH="80" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="fut." WC="0.98" HPOS="178" VPOS="100" WIDTH="50" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="headword-continuation" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45">
        <String CONTENT="the entry introduction continues" WC="0.98" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="headword-final" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45">
        <String CONTENT="at the normal column margin." WC="0.98" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="piel-sense" HPOS="90" VPOS="250" WIDTH="380" HEIGHT="45">
        <String CONTENT="אִבֵּד" WC="0.70" HPOS="90" VPOS="250" WIDTH="80" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="1." WC="0.98" HPOS="178" VPOS="250" WIDTH="30" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="to" WC="0.98" HPOS="216" VPOS="250" WIDTH="30" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="lose" WC="0.98" HPOS="254" VPOS="250" WIDTH="60" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="piel-continuation" HPOS="50" VPOS="300" WIDTH="420" HEIGHT="45">
        <String CONTENT="and the Piel sense continues." WC="0.98" HPOS="50" VPOS="300" WIDTH="420" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let english = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="body" HPOS="50" VPOS="100" WIDTH="420" HEIGHT="250">
      <TextLine ID="headword" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45">
        <String CONTENT="ABAD" WC="0.70" HPOS="90" VPOS="100" WIDTH="80" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="fut." WC="0.98" HPOS="178" VPOS="100" WIDTH="50" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="headword-continuation" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45">
        <String CONTENT="the entry introduction continues" WC="0.98" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="headword-final" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45">
        <String CONTENT="at the normal column margin." WC="0.98" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="piel-sense" HPOS="90" VPOS="250" WIDTH="380" HEIGHT="45">
        <String CONTENT="PIEL" WC="0.90" HPOS="90" VPOS="250" WIDTH="80" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="ABED" WC="0.70" HPOS="178" VPOS="250" WIDTH="80" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="1." WC="0.98" HPOS="266" VPOS="250" WIDTH="30" HEIGHT="45"/>
        <SP WIDTH="8"/><String CONTENT="to" WC="0.98" HPOS="304" VPOS="250" WIDTH="30" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="piel-continuation" HPOS="50" VPOS="300" WIDTH="420" HEIGHT="45">
        <String CONTENT="and the Piel sense continues." WC="0.98" HPOS="50" VPOS="300" WIDTH="420" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let parsed = parse_entries(
        (&canonical, &engine("fused")),
        Some((&english, &engine("tesseract"))),
        &context(
            "robinson-1854",
            "3",
            19,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );

    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].blocks.len(), 2);
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 3);
    assert_eq!(parsed.entries[0].blocks[1].spans.len(), 2);
}

#[test]
fn isolated_scan_mark_does_not_break_a_paragraph_across_columns() {
    let alto = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="left-column" HPOS="50" VPOS="1200" WIDTH="455" HEIGHT="150">
      <TextLine ID="left-final" HPOS="50" VPOS="1200" WIDTH="400" HEIGHT="45">
        <String CONTENT="the paragraph ends its left column here" WC="0.98" HPOS="50" VPOS="1200" WIDTH="400" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="scan-mark-line" HPOS="495" VPOS="1330" WIDTH="10" HEIGHT="20">
        <String CONTENT="=" WC="0.98" HPOS="495" VPOS="1330" WIDTH="10" HEIGHT="20"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="right-column" HPOS="550" VPOS="100" WIDTH="400" HEIGHT="45">
      <TextLine ID="right-opening" HPOS="550" VPOS="100" WIDTH="400" HEIGHT="45">
        <String CONTENT="and continues atop the right column." WC="0.98" HPOS="550" VPOS="100" WIDTH="400" HEIGHT="45"/>
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
    assert_eq!(parsed.entries[0].blocks.len(), 1);
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 2);
    assert!(parsed.assignments.iter().any(|(_, line, assignment)| {
        line == "scan-mark-line" && matches!(assignment, LineAssignment::Unparsed)
    }));
}

#[test]
fn running_head_is_ignored_and_flush_text_continues_previous_paragraph() {
    let page_one = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="body" HPOS="50" VPOS="1200" WIDTH="420" HEIGHT="50">
      <TextLine ID="previous-line" HPOS="50" VPOS="1200" WIDTH="400" HEIGHT="45">
        <String CONTENT="the paragraph begins here" WC="0.98" HPOS="50" VPOS="1200" WIDTH="400" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let first = parse_entries(
        (&page_one, &engine("tesseract")),
        None,
        &context(
            "robinson-1854",
            "1",
            17,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );
    let page_two = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="running-head" HPOS="470" VPOS="50" WIDTH="60" HEIGHT="30">
      <TextLine ID="running-head-line" HPOS="470" VPOS="50" WIDTH="60" HEIGHT="30">
        <String CONTENT="אָב" WC="0.99" HPOS="470" VPOS="50" WIDTH="60" HEIGHT="30"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="continued-body" HPOS="550" VPOS="100" WIDTH="400" HEIGHT="150">
      <TextLine ID="continued-line" HPOS="570" VPOS="100" WIDTH="380" HEIGHT="40">
        <String CONTENT="and continues slightly inset on this page" WC="0.98" HPOS="570" VPOS="100" WIDTH="380" HEIGHT="40"/>
      </TextLine>
      <TextLine ID="continued-line-2" HPOS="550" VPOS="150" WIDTH="400" HEIGHT="45">
        <String CONTENT="before returning to the column margin" WC="0.98" HPOS="550" VPOS="150" WIDTH="400" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="continued-line-3" HPOS="550" VPOS="200" WIDTH="400" HEIGHT="45">
        <String CONTENT="for one more continuation line." WC="0.98" HPOS="550" VPOS="200" WIDTH="400" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="new-paragraph" HPOS="550" VPOS="270" WIDTH="400" HEIGHT="100">
      <TextLine ID="indented-line" HPOS="590" VPOS="270" WIDTH="360" HEIGHT="45">
        <String CONTENT="A new paragraph." WC="0.98" HPOS="590" VPOS="270" WIDTH="360" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="new-paragraph-line-2" HPOS="550" VPOS="320" WIDTH="400" HEIGHT="45">
        <String CONTENT="continues flush left." WC="0.98" HPOS="550" VPOS="320" WIDTH="400" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let parsed = parse_entries_continuing(
        (&page_two, &engine("tesseract")),
        None,
        &context(
            "robinson-1854",
            "2",
            18,
            "fixtures/pages/robinson-damaged.pgm",
        ),
        first.entries.last().cloned(),
    );

    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].blocks.len(), 2);
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 4);
    assert_eq!(parsed.entries[0].blocks[1].spans.len(), 2);
    assert!(parsed.assignments.iter().any(|(_, line, assignment)| {
        line == "running-head-line" && matches!(assignment, LineAssignment::Unparsed)
    }));

    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("parsed.json"),
        serde_json::to_vec_pretty(&parsed).unwrap(),
    )
    .unwrap();
    let report = validate_corpus(&parsed.entries, Some(temporary.path()));
    assert_eq!(report.errors(), 0, "{:#?}", report.issues);
}

#[test]
fn flush_hebrew_example_does_not_cut_an_entry_mid_paragraph() {
    let page = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="body" HPOS="50" VPOS="100" WIDTH="420" HEIGHT="250">
      <TextLine ID="opening" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45">
        <String CONTENT="אָב a displayed headword" WC="0.98" HPOS="90" VPOS="100" WIDTH="380" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="prose" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45">
        <String CONTENT="the explanation continues with" WC="0.98" HPOS="50" VPOS="150" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="example" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45">
        <String CONTENT="אָבִיו, an example within the sentence," WC="0.98" HPOS="50" VPOS="200" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="continued-prose" HPOS="50" VPOS="250" WIDTH="420" HEIGHT="45">
        <String CONTENT="and the same paragraph continues." WC="0.98" HPOS="50" VPOS="250" WIDTH="420" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="next-entry" HPOS="90" VPOS="300" WIDTH="380" HEIGHT="45">
        <String CONTENT="אֵם the next displayed headword" WC="0.98" HPOS="90" VPOS="300" WIDTH="380" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let parsed = parse_entries(
        (&page, &engine("tesseract")),
        None,
        &context(
            "robinson-1854",
            "1",
            17,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );

    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(
        parsed.entries[0].headword.as_ref().unwrap().normalized,
        "אָב"
    );
    assert_eq!(parsed.entries[0].blocks[0].spans.len(), 4);
    assert_eq!(
        parsed.entries[1].headword.as_ref().unwrap().normalized,
        "אֵם"
    );
    assert!(parsed.assignments.iter().any(|(_, line, assignment)| {
        line == "example"
            && matches!(
                assignment,
                LineAssignment::Entry(entry) if entry == "robinson-1854:p1:e0001"
            )
    }));
}

#[test]
fn grammar_labeled_headwords_start_entries_without_block_relative_indentation() {
    let page = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="1000" HEIGHT="1400"><PrintSpace>
    <TextBlock ID="first-entry" HPOS="90" VPOS="100" WIDTH="500" HEIGHT="100">
      <TextLine ID="first-headword" HPOS="90" VPOS="100" WIDTH="500" HEIGHT="45">
        <String CONTENT="אָב" WC="0.98" HPOS="90" VPOS="100" WIDTH="60" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="m." WC="0.98" HPOS="160" VPOS="100" WIDTH="30" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="father" WC="0.98" HPOS="200" VPOS="100" WIDTH="100" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="flush-headword-block" HPOS="50" VPOS="220" WIDTH="540" HEIGHT="100">
      <TextLine ID="flush-headword" HPOS="50" VPOS="220" WIDTH="540" HEIGHT="45">
        <String CONTENT="אָבֶב" WC="0.98" HPOS="50" VPOS="220" WIDTH="80" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="in" WC="0.98" HPOS="140" VPOS="220" WIDTH="30" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="Heb." WC="0.98" HPOS="180" VPOS="220" WIDTH="50" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="not" WC="0.98" HPOS="240" VPOS="220" WIDTH="45" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="used;" WC="0.98" HPOS="295" VPOS="220" WIDTH="60" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="Chald." WC="0.98" HPOS="365" VPOS="220" WIDTH="70" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="Pa." WC="0.98" HPOS="445" VPOS="220" WIDTH="40" HEIGHT="45"/>
      </TextLine>
      <TextLine ID="language-prose" HPOS="50" VPOS="275" WIDTH="540" HEIGHT="45">
        <String CONTENT="Arab." WC="0.98" HPOS="50" VPOS="275" WIDTH="60" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="and" WC="0.98" HPOS="120" VPOS="275" WIDTH="40" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="Heb." WC="0.98" HPOS="170" VPOS="275" WIDTH="50" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="it" WC="0.98" HPOS="230" VPOS="275" WIDTH="20" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="continues." WC="0.98" HPOS="260" VPOS="275" WIDTH="120" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="proper-name-block" HPOS="90" VPOS="340" WIDTH="500" HEIGHT="100">
      <TextLine ID="proper-name" HPOS="90" VPOS="340" WIDTH="500" HEIGHT="45">
        <String CONTENT="RN" WC="0.60" HPOS="90" VPOS="340" WIDTH="50" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="Abagtha," WC="0.98" HPOS="150" VPOS="340" WIDTH="100" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="Pers." WC="0.98" HPOS="260" VPOS="340" WIDTH="55" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="pr." WC="0.98" HPOS="325" VPOS="340" WIDTH="35" HEIGHT="45"/>
        <SP WIDTH="10"/>
        <String CONTENT="n." WC="0.98" HPOS="370" VPOS="340" WIDTH="25" HEIGHT="45"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();

    let parsed = parse_entries(
        (&page, &engine("tesseract")),
        None,
        &context(
            "robinson-1854",
            "3",
            19,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );

    assert_eq!(parsed.entries.len(), 3);
    assert_eq!(
        parsed.entries[1].headword.as_ref().unwrap().normalized,
        "אָבֶב"
    );
    assert_eq!(
        parsed.entries[2].headword.as_ref().unwrap().normalized,
        "RN"
    );
}

#[test]
fn isolated_numeric_artifact_is_ignored_but_starred_headword_is_retained() {
    let multilingual = parse_alto(
        r#"<?xml version="1.0"?>
<alto xmlns="http://www.loc.gov/standards/alto/ns-v4#">
  <Layout><Page WIDTH="2000" HEIGHT="3400"><PrintSpace>
    <TextBlock ID="body" HPOS="100" VPOS="2800" WIDTH="800" HEIGHT="100">
      <TextLine ID="body-line" HPOS="100" VPOS="2800" WIDTH="800" HEIGHT="50">
        <String CONTENT="continuing prose" WC="0.98" HPOS="100" VPOS="2800" WIDTH="300" HEIGHT="50"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="star" HPOS="1030" VPOS="2900" WIDTH="20" HEIGHT="25">
      <TextLine ID="star-line" HPOS="1030" VPOS="2900" WIDTH="20" HEIGHT="25">
        <String CONTENT="*" WC="0.95" HPOS="1030" VPOS="2900" WIDTH="20" HEIGHT="25"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="headword" HPOS="1070" VPOS="2898" WIDTH="700" HEIGHT="60">
      <TextLine ID="headword-line" HPOS="1070" VPOS="2898" WIDTH="700" HEIGHT="60">
        <String CONTENT="ܢ" WC="0.67" HPOS="1070" VPOS="2898" WIDTH="70" HEIGHT="60"/>
        <SP WIDTH="20"/><String CONTENT="m." WC="0.92" HPOS="1160" VPOS="2920" WIDTH="50" HEIGHT="25"/>
      </TextLine>
    </TextBlock>
    <TextBlock ID="artifact" HPOS="690" VPOS="3240" WIDTH="20" HEIGHT="35">
      <TextLine ID="artifact-line" HPOS="690" VPOS="3240" WIDTH="20" HEIGHT="35">
        <String CONTENT="1" WC="0.94" HPOS="690" VPOS="3240" WIDTH="20" HEIGHT="35"/>
      </TextLine>
    </TextBlock>
  </PrintSpace></Page></Layout>
</alto>"#,
    )
    .unwrap();
    let classified = classify_word_languages(&multilingual, &["heb".to_owned(), "syr".to_owned()]);
    assert_eq!(
        classified.regions[2].lines[0].words[0].language.as_deref(),
        Some("heb")
    );

    let parsed = parse_entries(
        (&classified, &engine("tesseract")),
        None,
        &context(
            "robinson-1854",
            "1",
            17,
            "fixtures/pages/robinson-damaged.pgm",
        ),
    );
    assert!(parsed.assignments.iter().any(|(_, line, assignment)| {
        line == "artifact-line" && matches!(assignment, LineAssignment::Unparsed)
    }));
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
