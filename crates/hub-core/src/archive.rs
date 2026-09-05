use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

const MAX_EXPANDED_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// Extract only regular files and directories into a new staging directory.
/// Reject portable-path ambiguities, links and collisions before writing any member.
pub fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    ensure!(
        !destination.exists(),
        "Extraction destination must not already exist"
    );
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    ensure!(zip.len() <= 100_000, "Archive contains too many entries");
    let mut names = HashSet::new();
    let mut expanded = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index(index)?;
        validate_path(entry.name())?;
        let key = entry.name().trim_end_matches('/').to_lowercase();
        ensure!(
            names.insert(key),
            "Archive contains duplicate or case-colliding paths"
        );
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            ensure!(
                kind == 0 || kind == 0o100000 || kind == 0o040000,
                "Archive links and special files are not allowed"
            );
        }
        expanded = expanded
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        ensure!(
            expanded <= MAX_EXPANDED_BYTES,
            "Archive expands beyond 64 GiB limit"
        );
    }
    let parent = destination
        .parent()
        .context("Extraction path has no parent")?;
    fs::create_dir_all(parent)?;
    ensure!(
        fs2::available_space(parent)? > expanded.saturating_add(64 * 1024 * 1024),
        "Not enough disk space to extract archive"
    );
    fs::create_dir(destination)?;
    let result = (|| -> Result<()> {
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let output = destination.join(entry.enclosed_name().context("Unsafe archive path")?);
            if entry.is_dir() {
                fs::create_dir_all(&output)?;
                continue;
            }
            fs::create_dir_all(output.parent().context("Missing parent directory")?)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            // Bound actual decompression too: ZIP expanded-size metadata can be forged.
            let expected = entry.size();
            let written = io::copy(&mut (&mut entry).take(expected + 1), &mut file)?;
            ensure!(written == expected, "Expanded file size mismatch");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if entry.unix_mode().unwrap_or(0) & 0o111 != 0 {
                    0o755
                } else {
                    0o644
                };
                fs::set_permissions(&output, fs::Permissions::from_mode(mode))?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}
fn validate_path(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && !name.contains('\\') && !name.contains(':') && !name.contains('\0'),
        "Archive contains an unsafe portable path"
    );
    let path = Path::new(name);
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "Archive path traversal is not allowed"
    );
    for piece in name.trim_end_matches('/').split('/') {
        ensure!(
            !piece.is_empty() && piece != "." && piece != ".." && !piece.ends_with(['.', ' ']),
            "Archive contains an ambiguous path"
        );
        let stem = piece.split('.').next().unwrap_or("").to_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit()),
            "Archive contains a reserved Windows filename"
        );
    }
    Ok(())
}
/// SDK archives commonly wrap files in ABI/ or system-images/...; never guess
/// between multiple bootable images in one archive.
pub fn find_image_root(directory: &Path) -> Result<PathBuf> {
    fn visit(path: &Path, depth: usize, found: &mut Vec<PathBuf>) -> Result<()> {
        if path.join("system.img").is_file()
            && (path.join("kernel-ranchu").is_file() || path.join("kernel-ranchu-64").is_file())
            && path.join("ramdisk.img").is_file()
        {
            found.push(path.to_path_buf());
            return Ok(());
        }
        if depth >= 8 {
            return Ok(());
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                visit(&entry.path(), depth + 1, found)?;
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(directory, 0, &mut found)?;
    if found.len() != 1 {
        bail!(
            "Archive must contain exactly one system.img, ramdisk.img and kernel-ranchu image set; found {}",
            found.len()
        );
    }
    Ok(found.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn zip_with(path: &Path, names: &[&str]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for n in names {
            zip.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"content").unwrap();
        }
        zip.finish().unwrap();
    }
    #[test]
    fn traversal_and_collisions_leave_no_output() {
        for names in [
            vec!["../escape"],
            vec!["safe", "SAFE"],
            vec!["C:/escape"],
            vec!["dir\\escape"],
            vec!["CON.txt"],
        ] {
            let d = tempfile::tempdir().unwrap();
            let z = d.path().join("x.zip");
            zip_with(&z, &names);
            let out = d.path().join("out");
            assert!(extract_zip(&z, &out).is_err(), "{names:?}");
            assert!(!out.exists());
        }
    }
    #[test]
    fn wrapped_image_is_found() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("x.zip");
        zip_with(
            &z,
            &[
                "arm64-v8a/system.img",
                "arm64-v8a/kernel-ranchu",
                "arm64-v8a/ramdisk.img",
            ],
        );
        let out = d.path().join("out");
        extract_zip(&z, &out).unwrap();
        assert_eq!(find_image_root(&out).unwrap(), out.join("arm64-v8a"));
    }
    #[test]
    fn symbolic_links_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("links.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.add_symlink(
            "link",
            "../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        let output = dir.path().join("out");
        assert!(extract_zip(&archive, &output).is_err());
        assert!(!output.exists());
    }
}
