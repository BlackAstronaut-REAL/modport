use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use tempfile::Builder;
use zip::ZipArchive;

pub const MAX_EXPANDED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 250_000;

/// Safely extract a JAR/ZIP into a new destination directory.
/// Zip-slip paths and symbolic links are rejected, and expansion is bounded.
pub fn extract_archive(archive_path: &Path, destination: &Path) -> Result<PathBuf> {
    if destination.exists() {
        bail!("Extraction destination already exists: {}", destination.display());
    }
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let staged = Builder::new().prefix(".mojmodelbridge-extract-").tempdir_in(parent)?;
    let staged_path = staged.path().to_path_buf();

    let file = File::open(archive_path)
        .with_context(|| format!("Unable to open archive {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("{} is not a readable ZIP/JAR archive", archive_path.display()))?;

    if archive.len() > MAX_ARCHIVE_ENTRIES {
        bail!("Archive has {} entries; limit is {}", archive.len(), MAX_ARCHIVE_ENTRIES);
    }

    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let relative = entry.enclosed_name()
            .ok_or_else(|| anyhow::anyhow!("Rejected unsafe archive path: {}", entry.name()))?
            .to_path_buf();
        let output = staged_path.join(relative);

        // ZIP symlinks can redirect subsequent writes, so skip them entirely.
        if entry.unix_mode().map(|mode| (mode & 0o170000) == 0o120000).unwrap_or(false) {
            continue;
        }
        if entry.is_dir() {
            fs::create_dir_all(&output)?;
            continue;
        }

        let declared = entry.size();
        if declared > MAX_EXPANDED_BYTES.saturating_sub(expanded) {
            bail!("Archive expansion exceeds the {} byte safety limit", MAX_EXPANDED_BYTES);
        }

        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&output)?;
        let copied = io::copy(&mut entry, &mut out)?;
        if copied != declared {
            bail!("Truncated archive entry {} (expected {}, wrote {})", entry.name(), declared, copied);
        }
        expanded = expanded.saturating_add(copied);
        if expanded > MAX_EXPANDED_BYTES {
            bail!("Archive expansion exceeds the {} byte safety limit", MAX_EXPANDED_BYTES);
        }
    }

    // Persist the temporary directory only after every entry is extracted.
    let staged = staged.keep();
    fs::rename(&staged, destination).with_context(|| {
        format!("Unable to move extracted archive into {}", destination.display())
    })?;
    Ok(destination.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::FileOptions;

    #[test]
    fn extracts_a_jar_like_archive() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("mod.jar");
        {
            let file = File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer.start_file("assets/demo/test.txt", FileOptions::default()).unwrap();
            writer.write_all(b"native engine").unwrap();
            writer.finish().unwrap();
        }
        let destination = temp.path().join("extracted");
        extract_archive(&zip_path, &destination).unwrap();
        assert_eq!(fs::read(destination.join("assets/demo/test.txt")).unwrap(), b"native engine");
    }

    #[test]
    fn refuses_to_overwrite_existing_destination() {
        let temp = tempdir().unwrap();
        let destination = temp.path().join("already-there");
        fs::create_dir(&destination).unwrap();
        let result = extract_archive(&temp.path().join("missing.jar"), &destination);
        assert!(result.is_err());
    }
}
