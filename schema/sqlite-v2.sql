PRAGMA user_version=2;
CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE editions (
  id TEXT PRIMARY KEY,
  source_sha256 TEXT NOT NULL CHECK(length(source_sha256)=64),
  scan_id TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE entries (
  id TEXT PRIMARY KEY,
  edition_id TEXT NOT NULL REFERENCES editions(id),
  printed_page TEXT NOT NULL,
  entry_ordinal INTEGER NOT NULL CHECK(entry_ordinal > 0),
  headword_diplomatic TEXT,
  headword_normalized TEXT,
  homograph INTEGER,
  confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
  review_state TEXT NOT NULL CHECK(review_state IN ('machine','corrected','verified')),
  revision INTEGER NOT NULL CHECK(revision >= 0),
  pipeline_run TEXT NOT NULL,
  UNIQUE(edition_id, printed_page, entry_ordinal)
) WITHOUT ROWID;
CREATE INDEX entries_headword ON entries(headword_normalized);
CREATE TABLE aliases (
  alias TEXT PRIMARY KEY,
  entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TABLE senses (
  id TEXT PRIMARY KEY,
  entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  label TEXT,
  UNIQUE(entry_id, ordinal)
) WITHOUT ROWID;
CREATE TABLE blocks (
  id TEXT PRIMARY KEY,
  entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  sense_id TEXT REFERENCES senses(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  ordinal INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE spans (
  id TEXT PRIMARY KEY,
  entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  block_id TEXT REFERENCES blocks(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  diplomatic TEXT NOT NULL,
  normalized TEXT NOT NULL,
  language TEXT,
  script TEXT NOT NULL,
  direction TEXT NOT NULL CHECK(direction IN ('ltr','rtl','mixed')),
  confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
  review_state TEXT NOT NULL CHECK(review_state IN ('machine','corrected','verified'))
) WITHOUT ROWID;
CREATE INDEX spans_normalized ON spans(normalized);
CREATE TABLE language_runs (
  span_id TEXT NOT NULL REFERENCES spans(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  start_offset INTEGER NOT NULL CHECK(start_offset >= 0),
  end_offset INTEGER NOT NULL CHECK(end_offset > start_offset),
  language TEXT NOT NULL,
  script TEXT NOT NULL,
  evidence TEXT NOT NULL CHECK(evidence IN ('unicode_script','printed_label','edition_default')),
  PRIMARY KEY(span_id, ordinal)
) WITHOUT ROWID;
CREATE INDEX language_runs_language ON language_runs(language);
CREATE TABLE ocr_hypotheses (
  span_id TEXT NOT NULL REFERENCES spans(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  engine TEXT NOT NULL,
  engine_version TEXT NOT NULL,
  model TEXT NOT NULL,
  model_hash TEXT NOT NULL,
  text TEXT NOT NULL,
  confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
  PRIMARY KEY(span_id, ordinal)
) WITHOUT ROWID;
CREATE TABLE source_coordinates (
  span_id TEXT NOT NULL REFERENCES spans(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  source_page INTEGER NOT NULL,
  printed_page TEXT,
  region_id TEXT NOT NULL,
  line_id TEXT NOT NULL,
  polygon_json TEXT NOT NULL,
  transform_id TEXT NOT NULL,
  page_image TEXT NOT NULL,
  PRIMARY KEY(span_id, ordinal)
) WITHOUT ROWID;
CREATE TABLE unicode_warnings (
  span_id TEXT NOT NULL REFERENCES spans(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  code_point TEXT NOT NULL,
  character_offset INTEGER NOT NULL,
  code TEXT NOT NULL,
  message TEXT NOT NULL,
  PRIMARY KEY(span_id, ordinal)
) WITHOUT ROWID;
CREATE TABLE citations (
  id TEXT PRIMARY KEY,
  entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  target TEXT,
  span_id TEXT NOT NULL REFERENCES spans(id)
) WITHOUT ROWID;
CREATE TABLE cross_references (
  id TEXT PRIMARY KEY,
  entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  target_entry_id TEXT,
  span_id TEXT NOT NULL REFERENCES spans(id)
) WITHOUT ROWID;
CREATE VIRTUAL TABLE entry_fts USING fts5(entry_id UNINDEXED, headword, english);
