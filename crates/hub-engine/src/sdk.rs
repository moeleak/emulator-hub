//! A valid SDK layout for a standalone engine and separately installed platform-tools.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeFile {
    path: PathBuf,
    size: u64,
    sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct RuntimeSdk {
    schema_version: u32,
    source_adb: PathBuf,
    files: Vec<RuntimeFile>,
}

/// Reuse a complete SDK only when its ADB matches the selected executable.
/// Otherwise create a content-addressed SDK in `cache` using the actual ADB,
/// known Windows companions and shared libraries under its own lib64 folder.
/// No symlink privileges, SDK platform downloads or modifications to the input SDK are needed.
pub async fn prepare_runtime_sdk(
    cache: &Path,
    adb: &Path,
    preferred: Option<&Path>,
) -> Result<PathBuf> {
    let cache = cache.to_path_buf();
    let adb = adb.to_path_buf();
    let preferred = preferred.map(Path::to_path_buf);
    tokio::task::spawn_blocking(move || prepare(&cache, &adb, preferred.as_deref())).await?
}
fn adb_name() -> &'static str {
    if cfg!(windows) { "adb.exe" } else { "adb" }
}
fn resolve_adb(adb: &Path) -> Result<PathBuf> {
    if adb.is_file() {
        return Ok(fs::canonicalize(adb)?);
    }
    if adb.components().count() == 1
        && let Some(paths) = std::env::var_os("PATH")
    {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(adb);
            if candidate.is_file() {
                return Ok(fs::canonicalize(candidate)?);
            }
            #[cfg(windows)]
            {
                let candidate = candidate.with_extension("exe");
                if candidate.is_file() {
                    return Ok(fs::canonicalize(candidate)?);
                }
            }
        }
    }
    anyhow::bail!(
        "ADB executable is missing: {}. Install or select platform-tools first.",
        adb.display()
    )
}
fn digest_file(path: &Path) -> Result<(u64, String)> {
    let mut input = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        ensure!(
            size <= 256 * 1024 * 1024,
            "ADB runtime file exceeds 256 MiB"
        );
        hash.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", hash.finalize())))
}
fn collect_libraries(
    directory: &Path,
    relative: &Path,
    depth: usize,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<()> {
    ensure!(depth <= 4, "ADB library directory is unexpectedly deep");
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = relative.join(entry.file_name());
        if kind.is_dir() {
            collect_libraries(&entry.path(), &target, depth + 1, files)?;
        } else {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if (name.ends_with(".dll")
                || name.ends_with(".dylib")
                || name.ends_with(".so")
                || name.contains(".so."))
                && entry.path().is_file()
            {
                files.push((entry.path(), target));
            }
        }
    }
    Ok(())
}
fn inventory(adb: &Path) -> Result<Vec<(PathBuf, RuntimeFile)>> {
    let parent = adb
        .parent()
        .context("ADB executable has no parent directory")?;
    let mut paths = vec![(adb.to_path_buf(), PathBuf::from(adb_name()))];
    for name in ["AdbWinApi.dll", "AdbWinUsbApi.dll"] {
        let path = parent.join(name);
        if path.is_file() {
            paths.push((path, PathBuf::from(name)));
        }
    }
    let libraries = parent.join("lib64");
    if libraries.is_dir() {
        collect_libraries(&libraries, Path::new("lib64"), 0, &mut paths)?;
    }
    paths.sort_by(|a, b| a.1.cmp(&b.1));
    ensure!(
        paths.len() <= 128,
        "ADB runtime contains too many libraries"
    );
    let mut total = 0u64;
    paths
        .into_iter()
        .map(|(source, path)| {
            let (size, sha256) = digest_file(&source)?;
            total += size;
            ensure!(total <= 256 * 1024 * 1024, "ADB runtime exceeds 256 MiB");
            Ok((source, RuntimeFile { path, size, sha256 }))
        })
        .collect()
}
fn matches_payload(root: &Path, files: &[RuntimeFile]) -> bool {
    root.join("platforms").is_dir()
        && files.iter().all(|file| {
            digest_file(&root.join("platform-tools").join(&file.path))
                .is_ok_and(|(size, digest)| size == file.size && digest == file.sha256)
        })
}
fn prepare(cache: &Path, adb: &Path, preferred: Option<&Path>) -> Result<PathBuf> {
    let adb = resolve_adb(adb)?;
    if let Some(root) = preferred
        && root.join("platforms").is_dir()
        && fs::canonicalize(root.join("platform-tools").join(adb_name()))
            .is_ok_and(|candidate| candidate == adb)
    {
        return Ok(root.to_path_buf());
    }

    let payload = inventory(&adb)?;
    let files: Vec<_> = payload.iter().map(|(_, file)| file.clone()).collect();
    if let Some(root) = preferred
        && matches_payload(root, &files)
    {
        return Ok(root.to_path_buf());
    }

    let marker = RuntimeSdk {
        schema_version: 1,
        source_adb: adb,
        files,
    };
    let encoded = serde_json::to_vec(&marker)?;
    let key = format!("{:x}", Sha256::digest(&encoded));
    fs::create_dir_all(cache)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(cache.join(format!("{key}.lock")))?;
    // Other instances may be preparing the same tiny runtime at the same time.
    fs2::FileExt::lock_exclusive(&lock)?;
    let destination = cache.join(&key);
    if destination.join("hub-sdk.json").is_file() && matches_payload(&destination, &marker.files) {
        return Ok(destination);
    }
    ensure!(
        !destination.exists(),
        "Cached SDK is incomplete or changed: {}. Remove that incomplete runtime and retry.",
        destination.display()
    );
    let needed: u64 = marker.files.iter().map(|f| f.size).sum();
    ensure!(
        fs2::available_space(cache)? > needed + 16 * 1024 * 1024,
        "Not enough disk space for the ADB runtime SDK"
    );
    let staging = cache.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        fs::create_dir_all(staging.join("platforms"))?;
        fs::create_dir_all(staging.join("platform-tools"))?;
        for (source, file) in payload {
            let output = staging.join("platform-tools").join(&file.path);
            fs::create_dir_all(
                output
                    .parent()
                    .context("Runtime payload path has no parent")?,
            )?;
            fs::copy(source, &output)?;
        }
        ensure!(
            matches_payload(&staging, &marker.files),
            "ADB changed while its runtime SDK was being prepared; retry after the tool update completes"
        );
        fs::write(
            staging.join("hub-sdk.json"),
            serde_json::to_vec_pretty(&marker)?,
        )?;
        fs::rename(&staging, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn standalone_adb_gets_a_real_private_sdk_without_copying_its_entire_parent() {
        let temp = tempfile::tempdir().unwrap();
        let tools = temp.path().join("bin");
        fs::create_dir_all(tools.join("lib64")).unwrap();
        let adb = tools.join(adb_name());
        fs::write(&adb, b"adb fixture").unwrap();
        fs::write(tools.join("AdbWinApi.dll"), b"companion fixture").unwrap();
        fs::write(tools.join("lib64/libc++.so"), b"runtime library fixture").unwrap();
        fs::write(tools.join("unrelated-program"), b"do not copy").unwrap();
        let cache = temp.path().join("cache");
        let root = prepare_runtime_sdk(&cache, &adb, None).await.unwrap();
        assert!(root.join("platforms").is_dir());
        assert_eq!(
            fs::read(root.join("platform-tools").join(adb_name())).unwrap(),
            b"adb fixture"
        );
        assert!(root.join("platform-tools/AdbWinApi.dll").is_file());
        assert!(root.join("platform-tools/lib64/libc++.so").is_file());
        assert!(!root.join("platform-tools/unrelated-program").exists());
        assert_eq!(
            prepare_runtime_sdk(&cache, &adb, Some(&root))
                .await
                .unwrap(),
            root
        );
    }
    #[tokio::test]
    async fn an_existing_sdk_is_reused_only_for_its_selected_adb() {
        let temp = tempfile::tempdir().unwrap();
        let sdk = temp.path().join("sdk");
        fs::create_dir_all(sdk.join("platforms")).unwrap();
        fs::create_dir_all(sdk.join("platform-tools")).unwrap();
        let adb = sdk.join("platform-tools").join(adb_name());
        fs::write(&adb, b"old adb").unwrap();
        let cache = temp.path().join("cache");
        assert_eq!(
            prepare_runtime_sdk(&cache, &adb, Some(&sdk)).await.unwrap(),
            sdk
        );
        let newer = temp.path().join(adb_name());
        fs::write(&newer, b"new adb").unwrap();
        let replacement = prepare_runtime_sdk(&cache, &newer, Some(&sdk))
            .await
            .unwrap();
        assert_ne!(replacement, sdk);
        assert_eq!(fs::read(adb).unwrap(), b"old adb");
        assert_eq!(
            fs::read(replacement.join("platform-tools").join(adb_name())).unwrap(),
            b"new adb"
        );
    }
}
