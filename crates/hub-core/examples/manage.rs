//! Headless access to the same catalog, installation and instance state used by the UI.
//! cargo run -p hub-core --example manage -- <command> [--home PATH] [options]
use anyhow::{Context, Result, bail, ensure};
use hub_core::*;
use std::{collections::BTreeMap, path::PathBuf};

const USAGE: &str = "Emulator Hub management\n\
  manage list [--home PATH]\n\
  manage catalog [--home PATH]\n\
  manage import ZIP [--home PATH] [--name NAME --api MAJOR.MINOR --abi ABI --revision REV]\n\
  manage create --image IMAGE_KEY --name NAME [--home PATH] [--memory MB --cpus N --width PX --height PX --density DPI --disk MB]\n\n\
All successful commands print JSON. --home overrides EMULATOR_HUB_HOME and the standard application data directory.\n\
Import reads source.properties automatically. Supply all four metadata fields for an archive without it.\n\
ABIs: arm64-v8a, x86_64. A create command pins the selected installed image and uses private user data.";

fn options(args: impl Iterator<Item = String>) -> Result<BTreeMap<String, String>> {
    let mut args = args;
    let mut values = BTreeMap::new();
    while let Some(flag) = args.next() {
        ensure!(
            flag.starts_with("--"),
            "Unexpected argument {flag}; use --help"
        );
        let value = args
            .next()
            .with_context(|| format!("Missing value for {flag}"))?;
        ensure!(!value.starts_with("--"), "Missing value for {flag}");
        ensure!(
            values.insert(flag.clone(), value).is_none(),
            "Repeated option {flag}"
        );
    }
    Ok(values)
}
fn required(values: &mut BTreeMap<String, String>, key: &str) -> Result<String> {
    values
        .remove(key)
        .with_context(|| format!("Required option {key}"))
}
fn number(values: &mut BTreeMap<String, String>, key: &str, default: u32) -> Result<u32> {
    values
        .remove(key)
        .map(|v| {
            v.parse()
                .with_context(|| format!("{key} must be a positive integer"))
        })
        .unwrap_or(Ok(default))
}
fn exhausted(values: &BTreeMap<String, String>) -> Result<()> {
    ensure!(
        values.is_empty(),
        "Unknown options: {}",
        values.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    Ok(())
}
fn parse_api(value: &str) -> Result<ApiVersion> {
    let (major, minor) = value.split_once('.').unwrap_or((value, "0"));
    Ok(ApiVersion {
        major: major.parse().context("Invalid API major")?,
        minor: minor.parse().context("Invalid API minor")?,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        println!("{USAGE}");
        return Ok(());
    };
    if matches!(command.as_str(), "--help" | "help" | "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    ensure!(
        matches!(command.as_str(), "list" | "catalog" | "import" | "create"),
        "Unknown command {command}; use --help"
    );
    let archive = if command == "import" {
        Some(PathBuf::from(
            args.next().context("import needs a ZIP path")?,
        ))
    } else {
        None
    };
    let mut values = options(args)?;
    let paths = match values.remove("--home") {
        Some(home) => HubPaths::new(home),
        None => HubPaths::discover()?,
    };
    let hub = Hub::open(paths).await?;
    let output = match command.as_str() {
        "list" => {
            exhausted(&values)?;
            serde_json::json!({"images":hub.list_installed_images().await?,"instances":hub.list_instances().await?,"sources":hub.sources().await?})
        }
        "catalog" => {
            exhausted(&values)?;
            let catalog = hub.refresh_catalog().await?;
            let failures = catalog
                .errors
                .iter()
                .map(|e| serde_json::json!({"source_id":e.source_id,"message":e.message}))
                .collect::<Vec<_>>();
            serde_json::json!({"images":catalog.images,"errors":failures})
        }
        "import" => {
            let archive = archive.context("Missing archive")?;
            let mut metadata = match local_image_metadata(&archive).await {
                Ok(metadata) => metadata,
                Err(error) => {
                    let complete = ["--name", "--api", "--abi", "--revision"]
                        .iter()
                        .all(|key| values.contains_key(*key));
                    if !complete {
                        bail!(
                            "Cannot infer image metadata: {error:#}. Supply --name, --api, --abi and --revision."
                        );
                    }
                    LocalImageMetadata {
                        name: String::new(),
                        api: ApiVersion::default(),
                        abi: Abi::X86_64,
                        revision: String::new(),
                    }
                }
            };
            if let Some(name) = values.remove("--name") {
                metadata.name = name;
            }
            if let Some(api) = values.remove("--api") {
                metadata.api = parse_api(&api)?;
            }
            if let Some(abi) = values.remove("--abi") {
                metadata.abi = Abi::from_sdk(&abi).context("Unsupported ABI")?;
            }
            if let Some(revision) = values.remove("--revision") {
                metadata.revision = revision;
            }
            exhausted(&values)?;
            serde_json::to_value(hub.import_local_zip(&archive, metadata).await?)?
        }
        "create" => {
            let mut spec = InstanceSpec::new(
                required(&mut values, "--name")?,
                required(&mut values, "--image")?,
            );
            spec.memory_mb = number(&mut values, "--memory", spec.memory_mb)?;
            spec.cpu_cores = number(&mut values, "--cpus", spec.cpu_cores)?;
            spec.width = number(&mut values, "--width", spec.width)?;
            spec.height = number(&mut values, "--height", spec.height)?;
            spec.density = number(&mut values, "--density", spec.density)?;
            spec.data_disk_mb = number(&mut values, "--disk", spec.data_disk_mb)?;
            exhausted(&values)?;
            serde_json::to_value(hub.create_instance(spec).await?)?
        }
        _ => unreachable!(),
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
