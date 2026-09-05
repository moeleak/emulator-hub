use anyhow::{Context, Result, ensure};
use hub_core::Abi;
use std::{collections::BTreeSet, path::Path};

/// Dalvik-only APKs have no native ABI restriction. Native libraries are under
/// lib/<ABI>/; asset-delivered or downloaded libraries cannot be inferred here.
pub fn inspect_apk_abis(path: &Path) -> Result<Vec<String>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)
        .context("APK is not a valid ZIP archive")?;
    ensure!(archive.len() <= 100_000, "APK contains too many entries");
    let mut abis = BTreeSet::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let mut parts = file.name().split('/');
        if parts.next() == Some("lib")
            && let (Some(abi), Some(name)) = (parts.next(), parts.next())
            && name.ends_with(".so")
            && parts.next().is_none()
        {
            abis.insert(abi.to_string());
        }
    }
    Ok(abis.into_iter().collect())
}
pub async fn validate_apk_abi(path: &Path, guest: &Abi) -> Result<()> {
    let path = path.to_path_buf();
    let abis = tokio::task::spawn_blocking(move || inspect_apk_abis(&path)).await??;
    ensure!(
        abis.is_empty() || abis.iter().any(|abi| abi == guest.as_str()),
        "APK native libraries target {}, but this Android instance uses {}. Select an APK for {} or a universal APK; v1 does not translate native CPU instructions.",
        abis.join(", "),
        guest,
        guest
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_abi_rejection_and_universal_acceptance() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.apk");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            zip.start_file(
                "lib/arm64-v8a/libapp.so",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.finish().unwrap();
        }
        assert!(validate_apk_abi(&path, &Abi::Arm64V8a).await.is_ok());
        assert!(validate_apk_abi(&path, &Abi::X86_64).await.is_err());
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            for abi in ["x86_64", "arm64-v8a"] {
                zip.start_file(
                    format!("lib/{abi}/libapp.so"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            }
            zip.finish().unwrap();
        }
        assert!(validate_apk_abi(&path, &Abi::X86_64).await.is_ok());
    }
}
