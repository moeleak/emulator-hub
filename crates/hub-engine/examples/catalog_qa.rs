//! Live repository validation. `--install-platform-tools` additionally downloads,
//! verifies, safely extracts and runs the official ADB version command in a temp directory.
use anyhow::{Context, Result, ensure};
use hub_core::*;
use hub_engine::provision;

#[tokio::main]
async fn main() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("emulator-hub-catalog-qa-")
        .tempdir()?;
    let hub = Hub::open(HubPaths::new(directory.path())).await?;
    let catalog = hub.refresh_catalog().await?;
    for error in &catalog.errors {
        eprintln!("{}: {}", error.source_id, error.message);
    }
    ensure!(catalog.errors.is_empty(), "Some live repositories failed");
    let lineage: Vec<_> = catalog
        .images
        .iter()
        .filter(|p| p.source_id == "lineageos-avd")
        .collect();
    ensure!(
        lineage.len() >= 2,
        "LineageOS catalog is missing dual-architecture images"
    );
    ensure!(
        lineage.iter().all(|p| p.api
            == ApiVersion {
                major: 36,
                minor: 1
            }),
        "Imported LineageOS catalog API differs from 36.1"
    );
    let google: Vec<_> = catalog
        .images
        .iter()
        .filter(|p| {
            p.source_id.starts_with("google-")
                && p.api
                    == ApiVersion {
                        major: 36,
                        minor: 1,
                    }
        })
        .collect();
    ensure!(
        !google.is_empty(),
        "Google SDK discovery did not expose 36.1 images"
    );
    for image in lineage {
        println!(
            "LineageOS {}: API {} {} revision {}, {} bytes, min engine {:?}",
            image.name, image.api, image.abi, image.revision, image.size, image.min_engine_version
        );
    }
    println!(
        "Google API 36.1 images: {} (all discovered catalogs: {} images)",
        google.len(),
        catalog.images.len()
    );
    for image in google.iter().take(3) {
        println!(
            "Google {}: {} min engine {:?}",
            image.id, image.api, image.min_engine_version
        );
    }
    let tools = provision::discover_official_tools().await?;
    let adb = tools
        .iter()
        .find(|p| p.id == "platform-tools")
        .context("No ADB for this host")?;
    println!(
        "Official platform-tools {}: {} bytes",
        adb.version, adb.size
    );
    if std::env::args().any(|a| a == "--install-platform-tools") {
        ensure!(
            !adb.license.is_empty(),
            "Official tool license metadata missing"
        );
        let path = provision::install_tool(hub.paths(), adb, None).await?;
        let result = tokio::process::Command::new(&path)
            .arg("version")
            .output()
            .await?;
        ensure!(result.status.success(), "Provisioned ADB cannot execute");
        println!(
            "Verified and executed {}",
            String::from_utf8_lossy(&result.stdout).trim()
        );
    }
    Ok(())
}
