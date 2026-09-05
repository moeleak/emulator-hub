//! Download a source-built engine or opt-in official SDK tools, without republishing Google's artifacts.
use crate::EngineConfig;
use anyhow::{Context, Result, bail, ensure};
use hub_core::{Checksum, ChecksumAlgorithm, DownloadControl, DownloadProgress, HubPaths};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Component, PathBuf},
};
use tokio::sync::{mpsc, watch};

pub const DEFAULT_ENGINE_CATALOG: &str =
    "https://raw.githubusercontent.com/lineageos-avd/android-emulator/main/catalog.json";
const SDK_REPOSITORY: &str = "https://dl.google.com/android/repository/repository2-3.xml";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub url: String,
    pub size: u64,
    pub checksum: Checksum,
    pub executable: PathBuf,
    pub license: String,
    pub license_id: String,
    pub source: String,
}
#[derive(Debug, Deserialize)]
struct OwnCatalog {
    schema_version: u32,
    engines: Vec<OwnEngine>,
}
#[derive(Debug, Deserialize)]
struct OwnEngine {
    host_os: String,
    host_arch: String,
    version: String,
    url: String,
    size: u64,
    sha256: String,
    executable: PathBuf,
}

/// Default provision set: a published source-built engine plus official platform-tools.
/// An empty catalog is surfaced as unavailable, never silently replaced with Google binaries.
pub async fn discover_default_tools() -> Result<Vec<ToolPackage>> {
    discover_tools(DEFAULT_ENGINE_CATALOG, false).await
}
pub async fn discover_tools(
    engine_catalog: &str,
    use_official_emulator: bool,
) -> Result<Vec<ToolPackage>> {
    let mut official = discover_official_tools().await?;
    if use_official_emulator {
        return Ok(official);
    }
    let engine = discover_own_engine(engine_catalog).await?;
    official.retain(|p| p.id == "platform-tools");
    official.insert(0, engine);
    Ok(official)
}
pub async fn discover_own_engine(url: &str) -> Result<ToolPackage> {
    let client = hub_core::http_client()?;
    let response = client
        .get(hub_core::sources::validate_url(url)?)
        .send()
        .await?
        .error_for_status()?;
    let bytes = hub_core::sources::bounded_body(response, 2 * 1024 * 1024).await?;
    parse_engine_catalog(&bytes)
}
pub fn parse_engine_catalog(bytes: &[u8]) -> Result<ToolPackage> {
    let catalog: OwnCatalog = serde_json::from_slice(bytes)?;
    ensure!(
        catalog.schema_version == 1,
        "Unsupported engine catalog version"
    );
    let mut candidates: Vec<_> = catalog
        .engines
        .into_iter()
        .filter(|e| e.host_os == std::env::consts::OS && e.host_arch == std::env::consts::ARCH)
        .collect();
    candidates.sort_by(|a, b| {
        super::controller::version_tuple(&b.version)
            .cmp(&super::controller::version_tuple(&a.version))
    });
    let e=candidates.into_iter().next().with_context(||format!("The source-built engine has not been published for {} {} yet. Check the engine build workflow, or explicitly choose Google official tools in settings.",std::env::consts::OS,std::env::consts::ARCH))?;
    let package = ToolPackage {
        id: "emulator".into(),
        name: "LineageOS AVD Emulator (source build)".into(),
        version: e.version,
        url: e.url,
        size: e.size,
        checksum: Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: e.sha256,
        },
        executable: e.executable,
        license: String::new(),
        license_id: String::new(),
        source: "lineageos-avd".into(),
    };
    validate_tool(&package)?;
    Ok(package)
}
pub async fn discover_official_tools() -> Result<Vec<ToolPackage>> {
    let client = hub_core::http_client()?;
    let response = client
        .get(SDK_REPOSITORY)
        .send()
        .await?
        .error_for_status()?;
    let bytes = hub_core::sources::bounded_body(response, 32 * 1024 * 1024).await?;
    parse_official_tools(std::str::from_utf8(&bytes)?)
}
fn text<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(name))
        .and_then(|n| n.text())
}
pub fn parse_official_tools(xml: &str) -> Result<Vec<ToolPackage>> {
    let doc = roxmltree::Document::parse(xml)?;
    let base = reqwest::Url::parse(SDK_REPOSITORY)?;
    let mut tools = Vec::new();
    let mut found = HashSet::new();
    for node in doc
        .descendants()
        .filter(|n| n.has_tag_name("remotePackage"))
    {
        let Some(id) = node
            .attribute("path")
            .filter(|v| matches!(*v, "emulator" | "platform-tools"))
        else {
            continue;
        };
        if node
            .children()
            .find(|n| n.has_tag_name("channelRef"))
            .and_then(|n| n.attribute("ref"))
            .is_some_and(|v| v != "channel-0")
        {
            continue;
        }
        if found.contains(id) {
            continue;
        }
        let Some(archive) = node
            .descendants()
            .find(|n| n.has_tag_name("archive") && hub_core::sources::archive_matches_host(*n))
        else {
            continue;
        };
        let complete = archive
            .children()
            .find(|n| n.has_tag_name("complete"))
            .context("Tool archive lacks complete download")?;
        let revision = node
            .children()
            .find(|n| n.has_tag_name("revision"))
            .context("Tool lacks revision")?;
        let version = ["major", "minor", "micro"]
            .iter()
            .map(|p| text(revision, p).unwrap_or("0"))
            .collect::<Vec<_>>()
            .join(".");
        let checksum = complete
            .children()
            .find(|n| n.has_tag_name("checksum"))
            .context("Tool lacks checksum")?;
        let algorithm = match checksum.attribute("type").unwrap_or("sha1") {
            "sha1" => ChecksumAlgorithm::Sha1,
            "sha256" => ChecksumAlgorithm::Sha256,
            other => bail!("Unsupported tool checksum {other}"),
        };
        let license_id = node
            .children()
            .find(|n| n.has_tag_name("uses-license"))
            .and_then(|n| n.attribute("ref"))
            .unwrap_or("");
        let license = doc
            .descendants()
            .find(|n| n.has_tag_name("license") && n.attribute("id") == Some(license_id))
            .and_then(|n| n.text())
            .unwrap_or("");
        let name = if id == "emulator" { "emulator" } else { "adb" };
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let package = ToolPackage {
            id: id.into(),
            name: text(node, "display-name").unwrap_or(id).into(),
            version,
            url: base
                .join(text(complete, "url").context("Tool lacks URL")?)?
                .to_string(),
            size: text(complete, "size").context("Tool lacks size")?.parse()?,
            checksum: Checksum {
                algorithm,
                value: checksum.text().unwrap_or("").into(),
            },
            executable: PathBuf::from(format!("{id}/{name}{suffix}")),
            license: license.into(),
            license_id: license_id.into(),
            source: "google".into(),
        };
        validate_tool(&package)?;
        found.insert(id.to_string());
        tools.push(package);
    }
    ensure!(
        tools.iter().any(|p| p.id == "platform-tools"),
        "Official platform-tools are unavailable on this host"
    );
    ensure!(
        tools.iter().any(|p| p.id == "emulator"),
        "Official emulator is unavailable on this host"
    );
    Ok(tools)
}
fn validate_tool(package: &ToolPackage) -> Result<()> {
    ensure!(
        matches!(package.id.as_str(), "emulator" | "platform-tools"),
        "Unsupported tool package"
    );
    hub_core::sources::validate_url(&package.url)?;
    ensure!(package.size > 0, "Tool size must be positive");
    let expected = match package.checksum.algorithm {
        ChecksumAlgorithm::Sha1 => 40,
        ChecksumAlgorithm::Sha256 => 64,
    };
    ensure!(
        package.checksum.value.len() == expected
            && package
                .checksum
                .value
                .bytes()
                .all(|v| v.is_ascii_hexdigit()),
        "Invalid tool checksum"
    );
    ensure!(
        !package.executable.as_os_str().is_empty()
            && package
                .executable
                .components()
                .all(|v| matches!(v, Component::Normal(_))),
        "Tool executable must be a relative path inside the archive"
    );
    Ok(())
}
/// Callers display and persist the supplied Google license text before invoking installation.
pub async fn install_tools(
    paths: &HubPaths,
    packages: &[ToolPackage],
    progress: Option<mpsc::UnboundedSender<DownloadProgress>>,
) -> Result<EngineConfig> {
    let mut config = EngineConfig::default();
    let mut emulator_found = false;
    let mut adb_found = false;
    for package in packages {
        let executable = install_tool(paths, package, progress.clone()).await?;
        let destination = paths.engines.join(format!(
            "{}-{}",
            package.id,
            package.checksum.value.to_ascii_lowercase()
        ));
        match package.id.as_str() {
            "emulator" => {
                config.emulator = executable;
                config.version = Some(package.version.clone());
                config.sdk_root = Some(destination);
                emulator_found = true;
            }
            "platform-tools" => {
                config.adb = executable;
                adb_found = true;
            }
            _ => unreachable!(),
        }
    }
    ensure!(
        emulator_found && adb_found,
        "Provisioning needs both emulator and platform-tools packages"
    );
    config.sdk_root = Some(
        crate::prepare_runtime_sdk(
            &paths.engines.join("sdks"),
            &config.adb,
            config.sdk_root.as_deref(),
        )
        .await?,
    );
    hub_core::write_json(&paths.root.join("engine.json"), &config).await?;
    Ok(config)
}
/// Install one verified tool without changing the selected emulator configuration.
/// Returns the installed executable; the caller has already presented its license.
pub async fn install_tool(
    paths: &HubPaths,
    package: &ToolPackage,
    progress: Option<mpsc::UnboundedSender<DownloadProgress>>,
) -> Result<PathBuf> {
    let client = hub_core::http_client()?;
    validate_tool(package)?;
    let key = format!(
        "{}-{}",
        package.id,
        package.checksum.value.to_ascii_lowercase()
    );
    tokio::fs::create_dir_all(&paths.engines).await?;
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(paths.engines.join(format!("{key}.lock")))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("This engine tool is already being installed")?;
    let destination = paths.engines.join(&key);
    let executable = destination.join(&package.executable);
    if !destination.join("installed.json").is_file() || !executable.is_file() {
        ensure!(
            !destination.exists(),
            "Tool directory exists but is incomplete; remove this incomplete installation before retrying: {}",
            destination.display()
        );
        let archive = paths.downloads.join(format!("tool-{key}.zip"));
        let (_tx, control) = watch::channel(DownloadControl::Running);
        hub_core::download_verified(
            &client,
            &package.url,
            &archive,
            &package.checksum,
            package.size,
            progress.clone(),
            control,
        )
        .await?;
        let staging = paths
            .engines
            .join(format!(".staging-{}", uuid::Uuid::new_v4()));
        let source = archive.clone();
        let output = staging.clone();
        tokio::task::spawn_blocking(move || hub_core::extract_zip(&source, &output)).await??;
        if !staging.join(&package.executable).is_file() {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            bail!("Engine archive does not contain its declared executable");
        }
        hub_core::write_json(&staging.join("installed.json"), package).await?;
        tokio::fs::rename(&staging, &destination).await?;
    }
    Ok(executable)
}
pub async fn load_installed_engine(paths: &HubPaths) -> Result<Option<EngineConfig>> {
    let path = paths.root.join("engine.json");
    if !path.exists() {
        return Ok(None);
    }
    let config: EngineConfig = serde_json::from_slice(&tokio::fs::read(path).await?)?;
    ensure!(
        config.emulator.is_file() && config.adb.is_file(),
        "Saved emulator tools are missing; reinstall in Settings"
    );
    Ok(Some(config))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_source_catalog_is_unavailable() {
        assert!(
            parse_engine_catalog(br#"{"schema_version":1,"engines":[]}"#)
                .unwrap_err()
                .to_string()
                .contains("not been published")
        );
    }
    #[test]
    fn executable_cannot_escape() {
        let p = ToolPackage {
            id: "emulator".into(),
            name: "test".into(),
            version: "1".into(),
            url: "https://example.com/engine.zip".into(),
            size: 1,
            checksum: Checksum {
                algorithm: ChecksumAlgorithm::Sha256,
                value: "0".repeat(64),
            },
            executable: PathBuf::from("../outside"),
            license: String::new(),
            license_id: String::new(),
            source: "test".into(),
        };
        assert!(validate_tool(&p).is_err());
    }
}
