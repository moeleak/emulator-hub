//! cargo run -p hub-engine --example smoke -- --image-dir <SDK image> --emulator <binary> --adb <binary>
//! Uses a new private data directory and reads the supplied system image without modifying it.
use anyhow::{Context, Result, ensure};
use hub_core::*;
use hub_engine::*;
use std::{path::PathBuf, time::Duration};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let value = |name: &str| -> Result<PathBuf> {
        let i = args
            .iter()
            .position(|s| s == name)
            .with_context(|| format!("Required argument {name}"))?;
        Ok(PathBuf::from(
            args.get(i + 1).context("Missing argument value")?,
        ))
    };
    let image_dir = std::fs::canonicalize(value("--image-dir")?)?;
    let emulator = value("--emulator")?;
    let adb = value("--adb")?;
    let properties = std::fs::read_to_string(image_dir.join("source.properties"))?;
    let property = |key: &str| {
        properties.lines().find_map(|l| {
            l.split_once('=')
                .filter(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().to_string())
        })
    };
    let abi = property("SystemImage.Abi")
        .and_then(|v| Abi::from_sdk(&v))
        .context("Image ABI unavailable")?;
    let api = property("AndroidVersion.ApiLevel").unwrap_or("36".into());
    let (major, minor) = api.split_once('.').unwrap_or((&api, "0"));
    let temp = tempfile::Builder::new()
        .prefix("emulator-hub-smoke-")
        .tempdir()?;
    let paths = HubPaths::new(temp.path());
    let hub = Hub::open(paths.clone()).await?;
    let package = ImagePackage {
        id: "smoke-existing-local-image".into(),
        source_id: "local-smoke".into(),
        name: "Private smoke image".into(),
        revision: "existing".into(),
        api: ApiVersion {
            major: major.parse()?,
            minor: minor.parse()?,
        },
        abi,
        url: image_dir.to_string_lossy().into(),
        size: std::fs::metadata(image_dir.join("system.img"))?.len(),
        checksum: Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: "0".repeat(64),
        },
        license: String::new(),
        license_id: String::new(),
        min_engine_version: None,
        channel: "local-smoke".into(),
    };
    let image = InstalledImage {
        key: image_key(&package),
        package,
        directory: image_dir,
    };
    write_json(
        &paths.images.join(&image.key).join("installed.json"),
        &image,
    )
    .await?;
    let mut spec = InstanceSpec::new("Backend smoke", &image.key);
    spec.width = 720;
    spec.height = 1280;
    spec.density = 320;
    spec.memory_mb = 2048;
    spec.cpu_cores = 2;
    let instance = hub.create_instance(spec).await?;
    println!("Private smoke directory: {}", temp.path().display());
    let isolated_sdk = args.iter().any(|argument| argument == "--isolated-sdk");
    let config = EngineConfig {
        emulator: emulator.clone(),
        adb,
        sdk_root: if isolated_sdk {
            None
        } else {
            emulator
                .parent()
                .and_then(|p| p.parent())
                .map(PathBuf::from)
        },
        audio: false,
        ..Default::default()
    };
    let controller = EngineController::new(config);
    let handle = match controller.launch(&instance, &image).await {
        Ok(v) => v,
        Err(e) => {
            let log = std::fs::read_to_string(instance.directory.join("emulator.log"))
                .unwrap_or_default();
            eprintln!("{log}");
            return Err(e);
        }
    };
    let checks = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            let status = handle.status().await?;
            ensure!(status.running, "Emulator exited");
            if status.booted {
                println!("Booted emulator {}", status.version);
                break;
            }
            ensure!(tokio::time::Instant::now() < deadline, "Boot timed out");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if isolated_sdk {
            let sdk = handle
                .sdk_root()
                .context("Running instance has no SDK root")?;
            ensure!(
                sdk.starts_with(&instance.directory),
                "Isolated SDK unexpectedly reused the full source SDK"
            );
            let hardware = std::fs::read_to_string(
                instance
                    .avd_home
                    .join(format!("{}.avd/hardware-qemu.ini", instance.avd_name)),
            )?;
            let actual = hardware
                .lines()
                .filter_map(|line| line.split_once('='))
                .find(|(key, _)| key.trim() == "android.sdk.root")
                .map(|(_, value)| value.trim())
                .context("Engine did not record its SDK root")?;
            ensure!(
                std::fs::canonicalize(actual)? == std::fs::canonicalize(sdk)?,
                "Engine used a different SDK than the private runtime"
            );
            println!("Engine confirmed private SDK: {}", sdk.display());
        }
        let mut frames = handle.frames();
        tokio::time::timeout(Duration::from_secs(10), async {
            while frames.borrow().is_none() {
                frames.changed().await?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        {
            let frame = frames.borrow();
            let frame = frame.as_ref().unwrap();
            println!(
                "RGBA frame {}×{} ({} bytes)",
                frame.width,
                frame.height,
                frame.rgba.len()
            );
        }
        handle.set_clipboard("Emulator Hub smoke 验证").await?;
        ensure!(
            handle.get_clipboard().await? == "Emulator Hub smoke 验证",
            "Clipboard round trip failed"
        );
        handle.press_key("GoHome").await?;
        handle.send_touch(100, 100, true).await?;
        handle.release_inputs().await?;
        handle.send_wheel(0, 120).await?;
        let png = handle.screenshot_png().await?;
        ensure!(
            png.starts_with(b"\x89PNG\r\n\x1a\n"),
            "Screenshot is not PNG"
        );
        std::fs::write(instance.directory.join("smoke.png"), png)?;
        println!("Authenticated display, touch/key/wheel, clipboard and PNG passed");
        let kernel = handle
            .adb(&["shell".as_ref(), "uname".as_ref(), "-r".as_ref()])
            .await?;
        println!("Guest kernel: {}", kernel.trim());
        handle.save_snapshot("smoke").await?;
        handle.load_snapshot("smoke").await?;
        println!("Snapshot save/load passed");
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let stopped = handle.stop().await;
    drop(handle);
    if checks.is_err() || stopped.is_err() {
        let retained = temp.keep();
        eprintln!("Smoke evidence retained at {}", retained.display());
    } else {
        println!("Stopped private emulator cleanly");
    }
    checks?;
    stopped?;
    Ok(())
}
