//! Reproducible OCR pipeline, corpus model, validation, review, and exports.

pub mod alto;
pub mod benchmark;
pub mod corpus_io;
pub mod export;
pub mod metrics;
pub mod model;
pub mod pipeline;
pub mod report;
pub mod review;
pub mod source;
pub mod training;
pub mod unicode;
pub mod validate;

pub use model::{CorpusEntry, CorpusManifest};
