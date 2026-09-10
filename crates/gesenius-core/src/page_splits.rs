//! Authoritative printed-page partitions shared by preparation and evaluation.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

/// A page's permitted use. Development is never implicitly used for fitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Partition {
    /// Explicitly available for fitting.
    Training,
    /// Regression and policy development only.
    Development,
    /// Model selection only.
    Validation,
    /// Frozen final evaluation only.
    FinalTest,
}

/// Versioned page inventory. Additional editorial metadata is preserved in the source file.
#[derive(Debug, Serialize, Deserialize)]
pub struct PageSplits {
    /// Hash of the exact loaded split file, including editorial policy metadata.
    #[serde(skip)]
    pub manifest_sha256: String,
    /// Schema version.
    pub manifest_version: u32,
    /// Source edition.
    pub edition: String,
    /// Immutable PDF digest.
    pub source_sha256: String,
    /// Pages explicitly authorized for fitting.
    #[serde(default)]
    pub training: Vec<String>,
    /// Development pages.
    pub development: Vec<String>,
    /// Validation pages.
    pub validation: Vec<String>,
    /// Final test pages.
    pub final_test: Vec<String>,
    /// Pages already exposed during development.
    pub excluded_from_final_test: Vec<String>,
}
impl PageSplits {
    /// Loads and rejects overlapping, repeated, or exposed test pages.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path)?;
        let mut splits: Self = toml::from_str(std::str::from_utf8(&bytes)?)?;
        splits.manifest_sha256 = crate::headwords::digest(&bytes);
        splits.validate()?;
        Ok(splits)
    }
    fn validate(&self) -> Result<()> {
        if self.manifest_version != 1
            || self.edition.is_empty()
            || self.source_sha256.len() != 64
            || !self.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("invalid split manifest identity/version");
        }
        let mut seen = BTreeSet::new();
        for pages in [
            &self.training,
            &self.development,
            &self.validation,
            &self.final_test,
        ] {
            for page in pages {
                if page.trim().is_empty() || !seen.insert(page) {
                    bail!("empty, repeated or overlapping split page: {page}");
                }
            }
        }
        for page in &self.final_test {
            if self.excluded_from_final_test.contains(page)
                || (self.edition == "robinson-1854"
                    && page.parse::<u32>().is_ok_and(|p| (1..=10).contains(&p)))
            {
                bail!("exposed page {page} cannot enter final test");
            }
        }
        Ok(())
    }
    /// Returns the explicit assignment; unlisted pages are errors, never rehashed.
    pub fn partition(&self, edition: &str, page: &str) -> Result<Partition> {
        if edition != self.edition {
            bail!("edition does not match split manifest");
        }
        [
            (&self.training, Partition::Training),
            (&self.development, Partition::Development),
            (&self.validation, Partition::Validation),
            (&self.final_test, Partition::FinalTest),
        ]
        .into_iter()
        .find(|(pages, _)| pages.iter().any(|p| p == page))
        .map(|(_, partition)| partition)
        .context("page absent from authoritative split manifest")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_inventory_rejects_leakage_and_unassigned_pages() {
        let mut splits: PageSplits = toml::from_str(include_str!(
            "../../../benchmarks/sample-inventory/robinson-1854-splits.toml"
        ))
        .unwrap();
        splits.validate().unwrap();
        assert_eq!(
            splits.partition("robinson-1854", "175").unwrap(),
            Partition::Validation
        );
        assert!(splits.partition("robinson-1854", "99999").is_err());
        splits.training.push("175".into());
        assert!(splits.validate().is_err());
        splits.training.clear();
        splits.final_test.push("1".into());
        assert!(splits.validate().is_err());
    }
}
