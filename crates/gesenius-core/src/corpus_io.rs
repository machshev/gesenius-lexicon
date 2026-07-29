//! Deterministic JSONL and manifest persistence.

use crate::model::{CorpusEntry, CorpusManifest};
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// Loads one corpus entry per non-empty line.
pub fn load_entries(path: &Path) -> Result<Vec<CorpusEntry>> {
    let file = File::open(path)
        .with_context(|| format!("failed to open corpus JSONL {}", path.display()))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            result => Some((index, result)),
        })
        .map(|(index, line)| {
            let line = line?;
            serde_json::from_str(&line)
                .with_context(|| format!("invalid JSONL at {} line {}", path.display(), index + 1))
        })
        .collect()
}

/// Writes entries sorted by edition, printed page, ordinal, and ID.
pub fn write_entries(path: &Path, entries: &[CorpusEntry]) -> Result<()> {
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|left, right| {
        (
            &left.edition,
            printed_page_sort_key(&left.printed_page),
            left.entry_ordinal,
            &left.id,
        )
            .cmp(&(
                &right.edition,
                printed_page_sort_key(&right.printed_page),
                right.entry_ordinal,
                &right.id,
            ))
    });
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        for entry in sorted {
            serde_json::to_writer(&mut writer, entry)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to persist {}", path.display()))?;
    Ok(())
}

/// Loads an export manifest.
pub fn load_manifest(path: &Path) -> Result<CorpusManifest> {
    let input = fs::read(path)
        .with_context(|| format!("failed to read corpus manifest {}", path.display()))?;
    serde_json::from_slice(&input)
        .with_context(|| format!("invalid corpus manifest {}", path.display()))
}

/// Writes stable pretty JSON with a trailing newline.
pub fn write_manifest(path: &Path, manifest: &CorpusManifest) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    let mut json = serde_json::to_vec_pretty(manifest)?;
    json.push(b'\n');
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&json)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to persist {}", path.display()))?;
    Ok(())
}

fn printed_page_sort_key(page: &str) -> (u8, u32, String) {
    page.parse::<u32>().map_or_else(
        |_| (0, 0, page.to_owned()),
        |number| (1, number, String::new()),
    )
}

#[cfg(test)]
mod tests {
    use super::printed_page_sort_key;

    #[test]
    fn roman_front_matter_sorts_before_numbered_pages() {
        assert!(printed_page_sort_key("ix") < printed_page_sort_key("1"));
        assert!(printed_page_sort_key("2") < printed_page_sort_key("10"));
    }
}
