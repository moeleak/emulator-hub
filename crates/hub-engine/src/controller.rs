use crate::proto::{self, emulator_controller_client::EmulatorControllerClient};
use anyhow::{Context, Result, bail, ensure};
use hub_core::{InstalledImage, Instance};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    sync::{Mutex, mpsc, watch},
};
use tonic::{Request, metadata::MetadataValue, transport::Channel};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    pub emulator: PathBuf,
    pub adb: PathBuf,
    pub gpu: String,
    pub startup_timeout_secs: u64,
    pub version: Option<String>,
    pub sdk_root: Option<PathBuf>,
    pub audio: bool,
}
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            emulator: "emulator".into(),
            adb: "adb".into(),
            gpu: "auto".into(),
            startup_timeout_secs: 90,
            version: None,
            sdk_root: None,
            audio: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    pub sequence: u32,
}
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
    pub timestamp_us: u64,
}
#[derive(Debug, Clone)]
pub struct EngineStatus {
    pub running: bool,
    pub booted: bool,
    pub version: String,
    pub uptime_ms: u64,
}

#[derive(Clone)]
pub struct EngineController {
    config: EngineConfig,
}
impl EngineController {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }
    /// Existing instances keep their original engine when the application's
    /// default engine is updated. ADB and runtime preferences remain current.
    pub async fn for_instance(mut config: EngineConfig, instance: &Instance) -> Result<Self> {
        let path = instance.directory.join("engine-pin.json");
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::new(config));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Read engine pin {}", path.display()));
            }
        };
        let pin: EnginePin = serde_json::from_slice(&bytes)
            .with_context(|| format!("Invalid engine pin {}", path.display()))?;
        ensure!(
            pin.executable.is_file(),
            "Pinned engine {} is missing at {}. Restore that engine package at this location to start this instance; its data and snapshots are preserved.",
            pin.version,
            pin.executable.display()
        );
        config.sdk_root = pin.sdk_root.or_else(|| {
            pin.executable
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        });
        config.emulator = pin.executable;
        // This is the original executable's detected version, not a catalog label.
        // launch() still checks the actual binary against the unchanged pin.
        config.version = Some(pin.version);
        Ok(Self::new(config))
    }
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }
    pub async fn launch(
        &self,
        instance: &Instance,
        image: &InstalledImage,
    ) -> Result<RunningInstance> {
        ensure!(
            instance.spec.image_key == image.key,
            "Instance is pinned to a different image"
        );
        ensure!(
            image.package.abi.compatible_with_host(),
            "This image ABI {} does not match the host architecture {}",
            image.package.abi,
            std::env::consts::ARCH
        );
        ensure!(
            image.directory.join("system.img").is_file(),
            "Installed system image is missing"
        );
        ensure!(
            [
                "auto",
                "host",
                "software",
                "swiftshader_indirect",
                "swiftshader"
            ]
            .contains(&self.config.gpu.as_str()),
            "Unsupported GPU mode"
        );
        let process_lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(instance.directory.join("running.lock"))?;
        fs2::FileExt::try_lock_exclusive(&process_lock)
            .context("This instance is already running")?;
        // Catalog release labels may include the Hub build revision. Compatibility
        // checks and snapshot pins use the actual executable's upstream version.
        let version = self.detect_version().await?;
        // Ensure the standard host ADB identity exists before the emulator seeds
        // its guest authorized key. This also uses the provisioned ADB on fresh hosts.
        let mut adb = Command::new(&self.config.adb);
        adb.arg("start-server").kill_on_drop(true);
        hide_window(&mut adb);
        let output = tokio::time::timeout(Duration::from_secs(20), adb.output())
            .await
            .context("ADB server startup timed out")??;
        ensure!(
            output.status.success(),
            "Could not start ADB server: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(minimum) = &image.package.min_engine_version {
            ensure!(
                version_tuple(&version) >= version_tuple(minimum),
                "Image requires emulator {minimum} or later; selected engine is {version}"
            );
        }
        let pin_path = instance.directory.join("engine-pin.json");
        if pin_path.exists() {
            let pin: EnginePin = serde_json::from_slice(&tokio::fs::read(&pin_path).await?)?;
            ensure!(
                pin.version == version && pin.executable == self.config.emulator,
                "Instance is pinned to engine {} at {}. Restore that engine or create a new instance to preserve snapshot compatibility",
                pin.version,
                pin.executable.display()
            );
        }
        let sdk_root = crate::prepare_runtime_sdk(
            &instance.directory.join("sdk"),
            &self.config.adb,
            self.config.sdk_root.as_deref(),
        )
        .await?;
        if !pin_path.exists() {
            hub_core::write_json(
                &pin_path,
                &EnginePin {
                    version: version.clone(),
                    executable: self.config.emulator.clone(),
                    sdk_root: Some(sdk_root.clone()),
                },
            )
            .await?;
        }
        let lock_directory = instance
            .directory
            .parent()
            .context("Instance directory has no parent")?
            .join(".ports");
        let reserved = reserve_console_ports(&lock_directory)?;
        let console_port = reserved.port;
        let grpc_reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
        let grpc_port = grpc_reservation.local_addr()?.port();
        let runtime = instance.directory.join("runtime");
        tokio::fs::create_dir_all(&runtime).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).await?;
        }
        let logfile = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(instance.directory.join("emulator.log"))?;
        let mut command = Command::new(&self.config.emulator);
        command.args([
            "-avd",
            &instance.avd_name,
            "-no-window",
            "-no-boot-anim",
            "-no-metrics",
            "-no-snapshot-save",
            "-grpc-use-token",
            "-gpu",
            &self.config.gpu,
        ]);
        command
            .arg("-port")
            .arg(console_port.to_string())
            .arg("-grpc")
            .arg(grpc_port.to_string());
        if !self.config.audio {
            command.arg("-no-audio");
        }
        // Keep Google's shared ADB key location: changing ANDROID_EMULATOR_HOME
        // would seed a different guest key than the host ADB server uses.
        command.env("ANDROID_AVD_HOME", &instance.avd_home);
        if let Some(adb_directory) = self
            .config
            .adb
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            let mut paths = vec![adb_directory.to_path_buf()];
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            command.env("PATH", std::env::join_paths(paths)?);
        }
        command
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("TMPDIR", &runtime)
            .env("TMP", &runtime)
            .env("TEMP", &runtime);
        command
            .env("ANDROID_SDK_ROOT", &sdk_root)
            .env("ANDROID_HOME", &sdk_root);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(logfile.try_clone()?))
            .stderr(Stdio::from(logfile))
            .kill_on_drop(true);
        hide_window(&mut command);
        drop(reserved.console);
        drop(reserved.adb);
        drop(grpc_reservation);
        let mut child = command.spawn().with_context(|| {
            format!(
                "Could not start emulator {}",
                self.config.emulator.display()
            )
        })?;
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(self.config.startup_timeout_secs);
        let (client, token) = loop {
            if let Some(status) = child.try_wait()? {
                bail!(
                    "Emulator exited during startup ({status}); see {}",
                    instance.directory.join("emulator.log").display()
                );
            }
            if tokio::time::Instant::now() > deadline {
                let _ = child.kill().await;
                bail!(
                    "Timed out connecting to authenticated emulator gRPC; see {}",
                    instance.directory.join("emulator.log").display()
                );
            }
            if let Some(token) = find_grpc_token(&runtime, grpc_port) {
                let endpoint = Channel::from_shared(format!("http://127.0.0.1:{grpc_port}"))?
                    .connect_timeout(Duration::from_secs(1));
                if let Ok(channel) = endpoint.connect().await {
                    let mut client = EmulatorControllerClient::new(channel)
                        .max_decoding_message_size(64 * 1024 * 1024);
                    if client.get_status(auth_request((), &token)?).await.is_ok() {
                        break (client, token);
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        let (frame_tx, frame_rx) = watch::channel(None);
        let (error_tx, error_rx) = watch::channel(None);
        let stream_client = client.clone();
        let stream_token = token.clone();
        let frame_task = tokio::spawn(async move {
            if let Err(error) = frame_stream(stream_client, &stream_token, frame_tx).await {
                let _ = error_tx.send(Some(format!("Display stream stopped: {error:#}")));
            }
        });
        let mut updated = instance.clone();
        updated.engine_version = Some(version.clone());
        hub_core::write_json(&instance.directory.join("instance.json"), &updated).await?;
        Ok(RunningInstance {
            inner: Arc::new(RunningInner {
                child: Mutex::new(child),
                client,
                token,
                config: EngineConfig {
                    sdk_root: Some(sdk_root),
                    ..self.config.clone()
                },
                serial: format!("emulator-{console_port}"),
                version,
                abi: image.package.abi.clone(),
                frame_rx,
                error_rx,
                inputs: Mutex::new(InputState::default()),
                tasks: std::sync::Mutex::new(vec![frame_task.abort_handle()]),
                _process_lock: process_lock,
                port_lock: reserved.lock,
                resources_released: std::sync::atomic::AtomicBool::new(false),
            }),
            id: instance.id.clone(),
        })
    }
    async fn detect_version(&self) -> Result<String> {
        let mut command = Command::new(&self.config.emulator);
        command.arg("-version").kill_on_drop(true);
        hide_window(&mut command);
        let output = tokio::time::timeout(Duration::from_secs(20), command.output())
            .await
            .context("Emulator version check timed out")??;
        ensure!(output.status.success(), "Emulator version check failed");
        let value = String::from_utf8_lossy(&output.stdout);
        let version = value
            .lines()
            .find_map(|line| line.strip_prefix("Android emulator version "))
            .and_then(|v| v.split_whitespace().next())
            .context("Cannot parse emulator version output")?;
        Ok(version.to_string())
    }
}
#[derive(Serialize, Deserialize)]
struct EnginePin {
    version: String,
    executable: PathBuf,
    #[serde(default)]
    sdk_root: Option<PathBuf>,
}
#[derive(Clone)]
pub struct RunningInstance {
    pub id: String,
    inner: Arc<RunningInner>,
}
impl std::fmt::Debug for RunningInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningInstance")
            .field("id", &self.id)
            .field("serial", &self.inner.serial)
            .finish()
    }
}
struct RunningInner {
    abi: hub_core::Abi,
    child: Mutex<Child>,
    client: EmulatorControllerClient<Channel>,
    token: String,
    config: EngineConfig,
    serial: String,
    version: String,
    frame_rx: watch::Receiver<Option<Frame>>,
    error_rx: watch::Receiver<Option<String>>,
    inputs: Mutex<InputState>,
    tasks: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
    _process_lock: std::fs::File,
    port_lock: std::fs::File,
    resources_released: std::sync::atomic::AtomicBool,
}
impl Drop for RunningInner {
    fn drop(&mut self) {
        if let Ok(tasks) = self.tasks.lock() {
            for task in tasks.iter() {
                task.abort();
            }
        }
    }
}
#[derive(Default)]
struct InputState {
    keys: HashSet<String>,
    touch: Option<(i32, i32)>,
}
impl RunningInstance {
    pub fn frames(&self) -> watch::Receiver<Option<Frame>> {
        self.inner.frame_rx.clone()
    }
    pub fn stream_errors(&self) -> watch::Receiver<Option<String>> {
        self.inner.error_rx.clone()
    }
    pub fn serial(&self) -> &str {
        &self.inner.serial
    }
    pub fn sdk_root(&self) -> Option<&Path> {
        self.inner.config.sdk_root.as_deref()
    }
    pub async fn status(&self) -> Result<EngineStatus> {
        if self.inner.child.lock().await.try_wait()?.is_some() {
            return Ok(EngineStatus {
                running: false,
                booted: false,
                version: self.inner.version.clone(),
                uptime_ms: 0,
            });
        }
        let result = self
            .inner
            .client
            .clone()
            .get_status(self.request(())?)
            .await?
            .into_inner();
        Ok(EngineStatus {
            running: true,
            booted: result.booted,
            version: result.version,
            uptime_ms: result.uptime,
        })
    }
    fn request<T>(&self, message: T) -> Result<Request<T>> {
        auth_request(message, &self.inner.token)
    }
    /// Coordinates are guest display pixels; root UI scales from its letterboxed viewport.
    pub async fn send_touch(&self, x: i32, y: i32, down: bool) -> Result<()> {
        let mut inputs = self.inner.inputs.lock().await;
        self.inner
            .client
            .clone()
            .send_touch(self.request(proto::TouchEvent {
                touches: vec![proto::Touch {
                    x,
                    y,
                    identifier: 0,
                    pressure: if down { 1 } else { 0 },
                    ..Default::default()
                }],
                display: 0,
            })?)
            .await?;
        inputs.touch = if down { Some((x, y)) } else { None };
        Ok(())
    }
    pub async fn send_key(&self, key: &str, down: bool) -> Result<()> {
        let mut inputs = self.inner.inputs.lock().await;
        self.inner
            .client
            .clone()
            .send_key(self.request(proto::KeyboardEvent {
                key: key.into(),
                event_type: if down { 0 } else { 1 },
                ..Default::default()
            })?)
            .await?;
        if down {
            inputs.keys.insert(key.into());
        } else {
            inputs.keys.remove(key);
        }
        Ok(())
    }
    pub async fn press_key(&self, key: &str) -> Result<()> {
        self.inner
            .client
            .clone()
            .send_key(self.request(proto::KeyboardEvent {
                key: key.into(),
                event_type: 2,
                ..Default::default()
            })?)
            .await?;
        Ok(())
    }
    pub async fn send_text(&self, text: &str) -> Result<()> {
        ensure!(
            text.is_ascii() && text.len() <= 1024,
            "Direct text input supports at most 1024 ASCII bytes; use clipboard for Unicode or longer text"
        );
        self.inner
            .client
            .clone()
            .send_key(self.request(proto::KeyboardEvent {
                text: text.into(),
                ..Default::default()
            })?)
            .await?;
        Ok(())
    }
    /// 120 units are one wheel tick, matching Google's control protocol.
    pub async fn send_wheel(&self, dx: i32, dy: i32) -> Result<()> {
        self.inner
            .client
            .clone()
            .inject_wheel(self.request(tokio_stream::iter([proto::WheelEvent {
                dx,
                dy,
                display: 0,
            }]))?)
            .await?;
        Ok(())
    }
    pub async fn release_inputs(&self) -> Result<()> {
        let mut inputs = self.inner.inputs.lock().await;
        let mut client = self.inner.client.clone();
        if let Some((x, y)) = inputs.touch {
            client
                .send_touch(self.request(proto::TouchEvent {
                    touches: vec![proto::Touch {
                        x,
                        y,
                        identifier: 0,
                        pressure: 0,
                        ..Default::default()
                    }],
                    display: 0,
                })?)
                .await?;
            inputs.touch = None;
        }
        let keys: Vec<_> = inputs.keys.iter().cloned().collect();
        for key in keys {
            client
                .send_key(self.request(proto::KeyboardEvent {
                    key: key.clone(),
                    event_type: 1,
                    ..Default::default()
                })?)
                .await?;
            inputs.keys.remove(&key);
        }
        Ok(())
    }
    pub async fn set_clipboard(&self, text: &str) -> Result<()> {
        ensure!(
            text.len() <= 4 * 1024 * 1024,
            "Clipboard text exceeds 4 MiB"
        );
        self.inner
            .client
            .clone()
            .set_clipboard(self.request(proto::ClipData { text: text.into() })?)
            .await?;
        Ok(())
    }
    pub async fn get_clipboard(&self) -> Result<String> {
        Ok(self
            .inner
            .client
            .clone()
            .get_clipboard(self.request(())?)
            .await?
            .into_inner()
            .text)
    }
    pub async fn screenshot_png(&self) -> Result<Vec<u8>> {
        Ok(self
            .inner
            .client
            .clone()
            .get_screenshot(self.request(proto::ImageFormat {
                format: 0,
                ..Default::default()
            })?)
            .await?
            .into_inner()
            .image)
    }
    pub async fn install_apk(&self, apk: &Path) -> Result<String> {
        ensure!(apk.is_file(), "APK file does not exist");
        crate::validate_apk_abi(apk, &self.inner.abi).await?;
        self.adb(&[
            std::ffi::OsStr::new("install"),
            std::ffi::OsStr::new("-r"),
            apk.as_os_str(),
        ])
        .await
    }
    pub async fn push_file(&self, file: &Path, guest_path: &str) -> Result<String> {
        ensure!(file.is_file(), "File does not exist");
        ensure!(
            guest_path.starts_with("/sdcard/") && !guest_path.contains(['\r', '\n', '\0']),
            "File destination must be inside /sdcard/"
        );
        self.adb(&[
            std::ffi::OsStr::new("push"),
            file.as_os_str(),
            std::ffi::OsStr::new(guest_path),
        ])
        .await
    }
    pub async fn adb(&self, args: &[&std::ffi::OsStr]) -> Result<String> {
        let mut command = Command::new(&self.inner.config.adb);
        command
            .arg("-s")
            .arg(&self.inner.serial)
            .args(args)
            .kill_on_drop(true);
        hide_window(&mut command);
        let output = tokio::time::timeout(Duration::from_secs(180), command.output())
            .await
            .context("ADB command timed out")??;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        ensure!(output.status.success(), "ADB failed: {}", text.trim());
        Ok(text)
    }
    pub async fn save_snapshot(&self, name: &str) -> Result<()> {
        self.snapshot("save", name).await
    }
    pub async fn load_snapshot(&self, name: &str) -> Result<()> {
        self.release_inputs().await?;
        self.snapshot("load", name).await
    }
    async fn snapshot(&self, operation: &str, name: &str) -> Result<()> {
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
            "Snapshot name must use 1–64 letters, numbers, underscores or hyphens"
        );
        let response = self
            .adb(&[
                "emu".as_ref(),
                "avd".as_ref(),
                "snapshot".as_ref(),
                operation.as_ref(),
                name.as_ref(),
            ])
            .await?;
        ensure!(
            !response.lines().any(|line| line.starts_with("KO:")),
            "Snapshot operation failed: {response}"
        );
        Ok(())
    }
    pub async fn stop(&self) -> Result<()> {
        if self.inner.child.lock().await.try_wait()?.is_some() {
            return self.release_process_resources();
        }
        let _ = self.release_inputs().await;
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            self.inner
                .client
                .clone()
                .set_vm_state(self.request(proto::VmRunState { state: 5 })?),
        )
        .await;
        let mut child = self.inner.child.lock().await;
        match tokio::time::timeout(Duration::from_secs(15), child.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_) => {
                child.kill().await?;
                child.wait().await?;
            }
        }
        self.release_process_resources()
    }
    fn release_process_resources(&self) -> Result<()> {
        if self
            .inner
            .resources_released
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Ok(());
        }
        if let Ok(tasks) = self.inner.tasks.lock() {
            for task in tasks.iter() {
                task.abort();
            }
        }
        fs2::FileExt::unlock(&self.inner._process_lock)?;
        fs2::FileExt::unlock(&self.inner.port_lock)?;
        Ok(())
    }
    /// Optional PCM streaming for custom host audio sinks. Normal launches retain
    /// the emulator's native cross-platform output, so audio works without a sink.
    pub async fn stream_audio(&self) -> Result<mpsc::Receiver<AudioFrame>> {
        let mut request = self.request(proto::AudioFormat {
            sampling_rate: 48000,
            channels: 1,
            format: 1,
            mode: 1,
        })?;
        request.metadata_mut().remove("grpc-timeout");
        let mut stream = self
            .inner
            .client
            .clone()
            .stream_audio(request)
            .await?
            .into_inner();
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            while let Ok(Some(packet)) = stream.message().await {
                let format = packet.format.unwrap_or_default();
                let samples = packet
                    .audio
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let frame = AudioFrame {
                    sample_rate: format.sampling_rate as u32,
                    channels: if format.channels == 1 { 2 } else { 1 },
                    samples,
                    timestamp_us: packet.timestamp,
                };
                if tx.is_closed() {
                    break;
                }
                let _ = tx.try_send(frame);
            }
        });
        self.inner
            .tasks
            .lock()
            .map_err(|_| anyhow::anyhow!("Audio task lock poisoned"))?
            .push(task.abort_handle());
        Ok(rx)
    }
}
fn auth_request<T>(message: T, token: &str) -> Result<Request<T>> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}"))?,
    );
    request.set_timeout(Duration::from_secs(15));
    Ok(request)
}
async fn frame_stream(
    mut client: EmulatorControllerClient<Channel>,
    token: &str,
    tx: watch::Sender<Option<Frame>>,
) -> Result<()> {
    let mut request = auth_request(
        proto::ImageFormat {
            format: 1,
            ..Default::default()
        },
        token,
    )?;
    // A display stream lives for the VM lifetime; no per-request 15-second deadline.
    request.metadata_mut().remove("grpc-timeout");
    let mut stream = client.stream_screenshot(request).await?.into_inner();
    loop {
        let packet =
            tokio::select! { value=stream.message()=>value?, _=tx.closed()=>return Ok(()) };
        let Some(packet) = packet else {
            bail!("The emulator closed its display stream");
        };
        let Some(format) = packet.format else {
            continue;
        };
        if packet.image.is_empty() {
            let _ = tx.send(None);
            continue;
        }
        let count = u64::from(format.width) * u64::from(format.height) * 4;
        ensure!(
            count == packet.image.len() as u64 && count <= 64 * 1024 * 1024,
            "Emulator returned malformed RGBA frame"
        );
        // EmulatorController images use the guest's oriented display coordinates.
        if tx
            .send(Some(Frame {
                width: format.width,
                height: format.height,
                rgba: Arc::new(packet.image),
                sequence: packet.seq,
            }))
            .is_err()
        {
            return Ok(());
        }
    }
}
pub(crate) fn version_tuple(version: &str) -> (u32, u32, u32) {
    let mut p = version.split('.').map(|s| {
        s.split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("0")
            .parse()
            .unwrap_or(0)
    });
    (
        p.next().unwrap_or(0),
        p.next().unwrap_or(0),
        p.next().unwrap_or(0),
    )
}
struct PortReservation {
    port: u16,
    console: std::net::TcpListener,
    adb: std::net::TcpListener,
    lock: std::fs::File,
}
fn reserve_console_ports(directory: &Path) -> Result<PortReservation> {
    std::fs::create_dir_all(directory)?;
    for port in (5554..5682).step_by(2) {
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(format!("{port}.lock")))?;
        if fs2::FileExt::try_lock_exclusive(&lock).is_err() {
            continue;
        }
        if let Ok(console) = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            && let Ok(adb) = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port + 1))
        {
            return Ok(PortReservation {
                port,
                console,
                adb,
                lock,
            });
        }
    }
    bail!("No free emulator console/ADB port pair")
}
fn find_grpc_token(runtime: &Path, port: u16) -> Option<String> {
    let mut roots = vec![
        runtime.to_path_buf(),
        runtime.join("avd/running"),
        std::env::temp_dir().join("avd/running"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join("Library/Caches/TemporaryItems/avd/running"));
        roots.push(home.join(".android/avd/running"));
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        roots.push(PathBuf::from(runtime).join("avd/running"));
    }
    for root in roots {
        let Ok(files) = std::fs::read_dir(root) else {
            continue;
        };
        for file in files.flatten() {
            if !file.file_name().to_string_lossy().starts_with("pid_") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(file.path()) else {
                continue;
            };
            let mut found_port = None;
            let mut token = None;
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    match k.trim() {
                        "grpc.port" => found_port = v.trim().parse::<u16>().ok(),
                        "grpc.token" => token = Some(v.trim().to_string()),
                        _ => {}
                    }
                }
            }
            if found_port == Some(port) {
                return token.filter(|v| !v.is_empty());
            }
        }
    }
    None
}
#[cfg(windows)]
fn hide_window(command: &mut Command) {
    command.creation_flags(0x08000000);
}
#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_instance(directory: &Path) -> Instance {
        std::fs::create_dir_all(directory).unwrap();
        Instance {
            id: uuid::Uuid::new_v4().to_string(),
            spec: hub_core::InstanceSpec::new("Pinned device", "image"),
            directory: directory.to_path_buf(),
            avd_name: "pinned_device".into(),
            avd_home: directory.join("avd"),
            engine_version: Some("36.1.9.0".into()),
        }
    }
    #[tokio::test]
    async fn resolving_an_instance_keeps_its_old_engine_after_default_update() {
        let directory = tempfile::tempdir().unwrap();
        let instance = test_instance(&directory.path().join("instance"));
        let old_root = directory.path().join("old-sdk");
        let old_executable = old_root.join("emulator/emulator");
        std::fs::create_dir_all(old_executable.parent().unwrap()).unwrap();
        std::fs::write(&old_executable, b"engine fixture").unwrap();
        let current = EngineConfig {
            emulator: directory.path().join("new-sdk/emulator/emulator"),
            adb: directory.path().join("current-platform-tools/adb"),
            sdk_root: Some(directory.path().join("new-sdk")),
            version: Some("37.0.0-catalog-label".into()),
            audio: false,
            ..Default::default()
        };
        let unpinned = EngineController::for_instance(current.clone(), &instance)
            .await
            .unwrap();
        assert_eq!(unpinned.config().emulator, current.emulator);
        // Exercise compatibility with the original pin format lacking sdk_root.
        let pin = serde_json::to_vec(
            &serde_json::json!({"version":"36.1.9.0","executable":old_executable}),
        )
        .unwrap();
        let pin_path = instance.directory.join("engine-pin.json");
        std::fs::write(&pin_path, &pin).unwrap();
        let resolved = EngineController::for_instance(current.clone(), &instance)
            .await
            .unwrap();
        assert_eq!(resolved.config().emulator, old_executable);
        assert_eq!(resolved.config().version.as_deref(), Some("36.1.9.0"));
        assert_eq!(resolved.config().sdk_root, Some(old_root));
        assert_eq!(resolved.config().adb, current.adb);
        assert!(!resolved.config().audio);
        assert_eq!(std::fs::read(pin_path).unwrap(), pin);
    }
    #[tokio::test]
    async fn a_missing_pinned_engine_does_not_fall_back_or_rewrite_the_pin() {
        let directory = tempfile::tempdir().unwrap();
        let instance = test_instance(&directory.path().join("instance"));
        let pin_path = instance.directory.join("engine-pin.json");
        let pin = serde_json::to_vec(&EnginePin {
            version: "36.1.9.0".into(),
            executable: directory.path().join("missing/emulator"),
            sdk_root: None,
        })
        .unwrap();
        std::fs::write(&pin_path, &pin).unwrap();
        let current_executable = directory.path().join("current-emulator");
        std::fs::write(&current_executable, b"current engine fixture").unwrap();
        let error = EngineController::for_instance(
            EngineConfig {
                emulator: current_executable,
                ..Default::default()
            },
            &instance,
        )
        .await
        .err()
        .expect("missing pinned engine must fail");
        assert!(
            error
                .to_string()
                .contains("Pinned engine 36.1.9.0 is missing")
        );
        assert!(error.to_string().contains("Restore that engine package"));
        assert_eq!(std::fs::read(pin_path).unwrap(), pin);
    }
    #[test]
    fn version_compare() {
        assert!(version_tuple("36.1.9") > version_tuple("35.4.9"));
        assert!(version_tuple("36.1.10-preview") > version_tuple("36.1.9"));
    }
    #[test]
    fn port_lock_covers_socket_handoff_between_launches() {
        let directory = tempfile::tempdir().unwrap();
        let first = reserve_console_ports(directory.path()).unwrap();
        let original_port = first.port;
        drop(first.console);
        drop(first.adb);
        let second = reserve_console_ports(directory.path()).unwrap();
        assert_ne!(
            original_port, second.port,
            "Released sockets must stay reserved until the first process exits"
        );
    }
    #[test]
    fn token_discovery_matches_only_own_port() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pid_5_info.ini"),
            "grpc.port=12345\ngrpc.token=private\n",
        )
        .unwrap();
        assert_eq!(
            find_grpc_token(dir.path(), 12345).as_deref(),
            Some("private")
        );
        assert!(find_grpc_token(dir.path(), 12346).is_none());
    }
}
