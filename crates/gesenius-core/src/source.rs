//! Source catalogue and content-addressed scan storage.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default source catalogue path.
pub const DEFAULT_CATALOGUE: &str = "sources.toml";

/// A catalogue of source editions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCatalogue {
    /// Catalogue format version.
    pub catalogue_version: u32,
    /// Registered scans.
    pub sources: Vec<SourceRecord>,
}

impl SourceCatalogue {
    /// Loads and validates a TOML source catalogue.
    pub fn load(path: &Path) -> Result<Self> {
        let input = fs::read_to_string(path)
            .with_context(|| format!("failed to read source catalogue {}", path.display()))?;
        let catalogue: Self = toml::from_str(&input)
            .with_context(|| format!("invalid source catalogue {}", path.display()))?;
        catalogue.validate()?;
        Ok(catalogue)
    }

    /// Returns the uniquely named edition.
    pub fn edition(&self, edition: &str) -> Result<&SourceRecord> {
        self.sources
            .iter()
            .find(|source| source.edition == edition)
            .with_context(|| format!("edition `{edition}` is not registered"))
    }

    /// Ensures identifiers, hashes, and import locations are usable.
    pub fn validate(&self) -> Result<()> {
        if self.catalogue_version != 1 {
            bail!(
                "unsupported catalogue version {}; expected 1",
                self.catalogue_version
            );
        }
        for source in &self.sources {
            if source.edition.trim().is_empty() || source.scan_id.trim().is_empty() {
                bail!("source edition and scan_id must not be empty");
            }
            validate_sha256(&source.sha256)
                .with_context(|| format!("invalid SHA-256 for edition `{}`", source.edition))?;
            if source.download_url.is_none() && source.local_import_path.is_none() {
                bail!(
                    "edition `{}` needs download_url or local_import_path",
                    source.edition
                );
            }
        }
        Ok(())
    }
}

/// One immutable scanned source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRecord {
    /// Stable edition identifier used in entry IDs.
    pub edition: String,
    /// Bibliographic title.
    pub title: String,
    /// Edition or publication year.
    pub year: u16,
    /// Publisher statement.
    pub publisher: String,
    /// Public catalogue landing page.
    pub public_url: String,
    /// Direct PDF URL when remote fetching is supported.
    pub download_url: Option<String>,
    /// Repository-relative or absolute path for an owner-selected scan.
    pub local_import_path: Option<PathBuf>,
    /// Rights statement copied from the holding institution.
    pub rights: String,
    /// Lowercase hexadecimal SHA-256 of the exact PDF.
    pub sha256: String,
    /// Institution or archive scan identifier.
    pub scan_id: String,
    /// Printed page = PDF page + this offset.
    pub printed_page_offset: i32,
    /// Explicit PDF-page to printed-page labels for non-Arabic front matter.
    #[serde(default)]
    pub printed_page_labels: BTreeMap<u32, String>,
    /// Optional known PDF page count.
    pub page_count: Option<u32>,
    /// Other scans retained for later comparison.
    #[serde(default)]
    pub alternate_scans: Vec<AlternateScan>,
}

/// Alternate scan metadata without making it a processing default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlternateScan {
    /// Stable scan identifier.
    pub scan_id: String,
    /// Public catalogue URL.
    pub public_url: String,
    /// Why this scan may be useful.
    pub note: String,
}

/// Successful source verification result.
#[derive(Debug, Clone, Serialize)]
pub struct VerifiedSource {
    /// Edition identifier.
    pub edition: String,
    /// Expected and observed content hash.
    pub sha256: String,
    /// Content-addressed PDF path.
    pub path: PathBuf,
    /// File size.
    pub bytes: u64,
}

/// Returns the immutable cache location for a source.
#[must_use]
pub fn cached_source_path(cache_root: &Path, source: &SourceRecord) -> PathBuf {
    cache_root
        .join("sources")
        .join(&source.sha256)
        .join("source.pdf")
}

/// Fetches a registered source using `curl`, verifies it, and imports it atomically.
pub fn fetch_source(source: &SourceRecord, cache_root: &Path) -> Result<VerifiedSource> {
    let url = source.download_url.as_ref().with_context(|| {
        format!(
            "edition `{}` has no direct download; use `source import`",
            source.edition
        )
    })?;
    let destination = cached_source_path(cache_root, source);
    if destination.exists() {
        return verify_source(source, cache_root);
    }
    let directory = destination
        .parent()
        .context("cached source path has no parent")?;
    fs::create_dir_all(directory)?;
    let temporary = tempfile::NamedTempFile::new_in(directory)?;
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--retry",
            "3",
            "--silent",
            "--show-error",
        ])
        .arg("--output")
        .arg(temporary.path())
        .arg(url)
        .status()
        .context("failed to execute curl; enter the Nix development shell")?;
    if !status.success() {
        bail!("curl failed while fetching {url}");
    }
    verify_path_hash(temporary.path(), &source.sha256)?;
    temporary
        .persist(&destination)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to persist {}", destination.display()))?;
    verify_source(source, cache_root)
}

/// Imports an owner-provided PDF after checking its registered digest.
pub fn import_source(
    source: &SourceRecord,
    input: &Path,
    cache_root: &Path,
) -> Result<VerifiedSource> {
    if !input.is_file() {
        bail!("source import is not a file: {}", input.display());
    }
    verify_path_hash(input, &source.sha256)?;
    let destination = cached_source_path(cache_root, source);
    if destination.exists() {
        return verify_source(source, cache_root);
    }
    let directory = destination
        .parent()
        .context("cached source path has no parent")?;
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    let mut source_file = File::open(input)?;
    std::io::copy(&mut source_file, &mut temporary)?;
    temporary
        .persist(&destination)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to persist {}", destination.display()))?;
    verify_source(source, cache_root)
}

/// Verifies the cached copy of a registered source.
pub fn verify_source(source: &SourceRecord, cache_root: &Path) -> Result<VerifiedSource> {
    let path = cached_source_path(cache_root, source);
    if !path.is_file() {
        bail!(
            "edition `{}` is not cached at {}; run `source fetch` or `source import`",
            source.edition,
            path.display()
        );
    }
    verify_path_hash(&path, &source.sha256)?;
    Ok(VerifiedSource {
        edition: source.edition.clone(),
        sha256: source.sha256.clone(),
        bytes: fs::metadata(&path)?.len(),
        path,
    })
}

/// Calculates a lowercase hexadecimal SHA-256 digest.
pub fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn verify_path_hash(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if actual != expected {
        bail!(
            "SHA-256 mismatch for {}: expected {expected}, got {actual}",
            path.display()
        );
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{sha256_file, validate_sha256};
    use std::io::Write;

    #[test]
    fn hashes_files_incrementally() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"Gesenius").unwrap();
        assert_eq!(
            sha256_file(file.path()).unwrap(),
            "af0a721ec22ddcb45766a3bbecba2efea1ca9e16d260d5b2eee518362f8fa7ab"
        );
    }

    #[test]
    fn registered_hashes_are_strict() {
        assert!(validate_sha256(&"a".repeat(64)).is_ok());
        assert!(validate_sha256(&"A".repeat(64)).is_err());
        assert!(validate_sha256("pending").is_err());
    }
}
