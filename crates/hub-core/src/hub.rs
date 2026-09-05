use crate::*;
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use sha2::Digest;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, mpsc, watch};

#[derive(Debug, Clone)]
pub struct HubPaths {
    pub root: PathBuf,
    pub images: PathBuf,
    pub instances: PathBuf,
    pub downloads: PathBuf,
    pub engines: PathBuf,
}
impl HubPaths {
    pub fn discover() -> Result<Self> {
        if let Some(path) = std::env::var_os("EMULATOR_HUB_HOME") {
            return Ok(Self::new(path));
        }
        let dirs = directories::ProjectDirs::from("io", "moeleak", "Emulator Hub")
            .context("Could not locate application data directory")?;
        Ok(Self::new(dirs.data_local_dir()))
    }
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            images: root.join("images"),
            instances: root.join("instances"),
            downloads: root.join("downloads"),
            engines: root.join("engines"),
            root,
        }
    }
}
#[derive(Clone)]
pub struct Hub {
    paths: HubPaths,
    client: reqwest::Client,
    mutation: Arc<Mutex<()>>,
}
impl Hub {
    pub async fn open(paths: HubPaths) -> Result<Self> {
        for path in [
            &paths.root,
            &paths.images,
            &paths.instances,
            &paths.downloads,
            &paths.engines,
        ] {
            tokio::fs::create_dir_all(path).await?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&paths.root, std::fs::Permissions::from_mode(0o700)).await?;
        }
        let client = http_client()?;
        let hub = Self {
            paths,
            client,
            mutation: Arc::new(Mutex::new(())),
        };
        if !hub.paths.root.join("sources.json").exists() {
            hub.save_sources(&SourceConfig::defaults()).await?;
        }
        Ok(hub)
    }
    pub fn paths(&self) -> &HubPaths {
        &self.paths
    }
    pub async fn sources(&self) -> Result<Vec<SourceConfig>> {
        read_json(&self.paths.root.join("sources.json")).await
    }
    pub async fn save_sources(&self, sources: &[SourceConfig]) -> Result<()> {
        let _guard = self.mutation.lock().await;
        let mut ids = std::collections::HashSet::new();
        for source in sources {
            ensure!(
                !source.id.is_empty() && ids.insert(source.id.clone()),
                "Source IDs must be nonempty and unique"
            );
            sources::validate_url(&source.url)?;
        }
        write_json(&self.paths.root.join("sources.json"), &sources).await
    }
    pub async fn refresh_catalog(&self) -> Result<CatalogRefresh> {
        let mut sources = self.sources().await?;
        let mut result = CatalogRefresh::default();
        // The discovery index controls schema URLs; user-edited URLs remain authoritative.
        if sources.iter().any(|s| {
            s.enabled
                && s.url
                    .starts_with("https://dl.google.com/android/repository/sys-img/")
        }) {
            match sources::discover_google_sources(&self.client).await {
                Ok(discovered) => {
                    for source in &mut sources {
                        if source
                            .url
                            .starts_with("https://dl.google.com/android/repository/sys-img/")
                            && let Some(current) = discovered.iter().find(|s| s.id == source.id)
                        {
                            source.url.clone_from(&current.url);
                        }
                    }
                }
                Err(e) => result.errors.push(SourceFailure {
                    source_id: "google-discovery".into(),
                    message: format!("Discovery failed; using configured repository URLs: {e:#}"),
                }),
            }
        }
        let tasks = sources.into_iter().filter(|s| s.enabled).map(|source| {
            let client = self.client.clone();
            async move {
                let result = sources::fetch_catalog(&client, &source).await;
                (source, result)
            }
        });
        for (source, fetched) in futures_util::future::join_all(tasks).await {
            match fetched {
                Ok(mut images) => result.images.append(&mut images),
                Err(e) => result.errors.push(SourceFailure {
                    source_id: source.id,
                    message: format!("{e:#}"),
                }),
            }
        }
        result.images.sort_by(|a, b| {
            b.api
                .major
                .cmp(&a.api.major)
                .then(b.api.minor.cmp(&a.api.minor))
                .then(a.source_id.cmp(&b.source_id))
                .then(a.name.cmp(&b.name))
        });
        Ok(result)
    }
    pub async fn list_installed_images(&self) -> Result<Vec<InstalledImage>> {
        read_records(&self.paths.images, "installed.json").await
    }
    pub async fn list_instances(&self) -> Result<Vec<Instance>> {
        let mut values: Vec<Instance> =
            read_records(&self.paths.instances, "instance.json").await?;
        values.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
        Ok(values)
    }
    pub async fn install_image(
        &self,
        package: &ImagePackage,
        progress: Option<mpsc::UnboundedSender<DownloadProgress>>,
    ) -> Result<InstalledImage> {
        let (_control, receiver) = watch::channel(DownloadControl::Running);
        self.install_image_controlled(package, progress, receiver)
            .await
    }
    pub async fn install_image_controlled(
        &self,
        package: &ImagePackage,
        progress: Option<mpsc::UnboundedSender<DownloadProgress>>,
        control: watch::Receiver<DownloadControl>,
    ) -> Result<InstalledImage> {
        sources::validate_package(package)?;
        let key = image_key(package);
        let record = self.paths.images.join(&key).join("installed.json");
        if record.exists() {
            return read_json(&record).await;
        }
        let archive = self.paths.downloads.join(format!("{key}.zip"));
        download_verified(
            &self.client,
            &package.url,
            &archive,
            &package.checksum,
            package.size,
            progress.clone(),
            control.clone(),
        )
        .await?;
        ensure!(
            *control.borrow() != DownloadControl::Cancelled,
            "Installation cancelled"
        );
        self.install_archive(package, &archive, progress).await
    }
    pub async fn import_local_zip(
        &self,
        archive: &Path,
        metadata: LocalImageMetadata,
    ) -> Result<InstalledImage> {
        ensure!(
            metadata.api.major > 0
                && !metadata.name.trim().is_empty()
                && !metadata.revision.trim().is_empty(),
            "Local image requires a name, revision and API version"
        );
        let archive = tokio::fs::canonicalize(archive).await?;
        let path = archive.clone();
        let (size, digest) = tokio::task::spawn_blocking(move || -> Result<(u64, String)> {
            use std::io::Read;
            let mut file = std::fs::File::open(path)?;
            let size = file.metadata()?.len();
            let mut hash = sha2::Sha256::new();
            let mut b = vec![0; 1024 * 1024];
            loop {
                let n = file.read(&mut b)?;
                if n == 0 {
                    break;
                }
                hash.update(&b[..n]);
            }
            Ok((size, format!("{:x}", hash.finalize())))
        })
        .await??;
        let package = ImagePackage {
            id: format!("local-{}", &digest[..16]),
            source_id: "local".into(),
            name: metadata.name,
            revision: metadata.revision,
            api: metadata.api,
            abi: metadata.abi,
            url: archive.to_string_lossy().to_string(),
            size,
            checksum: Checksum {
                algorithm: ChecksumAlgorithm::Sha256,
                value: digest,
            },
            license: String::new(),
            license_id: String::new(),
            min_engine_version: None,
            channel: "local".into(),
        };
        self.install_archive(&package, &archive, None).await
    }
    async fn install_archive(
        &self,
        package: &ImagePackage,
        archive: &Path,
        progress: Option<mpsc::UnboundedSender<DownloadProgress>>,
    ) -> Result<InstalledImage> {
        let _guard = self.mutation.lock().await;
        let key = image_key(package);
        let destination = self.paths.images.join(&key);
        if destination.join("installed.json").exists() {
            return read_json(&destination.join("installed.json")).await;
        }
        ensure!(
            !destination.exists(),
            "Image directory exists without a valid installation record"
        );
        let staging = self
            .paths
            .images
            .join(format!(".staging-{}", uuid::Uuid::new_v4()));
        crate::download::progress(
            &progress,
            package.size,
            package.size,
            DownloadStage::Extracting,
        );
        let archive = archive.to_path_buf();
        let output = staging.clone();
        let relative = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
            extract_zip(&archive, &output)?;
            let root = find_image_root(&output)?;
            Ok(root.strip_prefix(&output)?.into())
        })
        .await?;
        let relative = match relative {
            Ok(p) => p,
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(e);
            }
        };
        let installed = InstalledImage {
            key,
            package: package.clone(),
            directory: destination.join(&relative),
        };
        if let Err(e) = verify_image_metadata(&staging.join(&relative), package).await {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(e);
        }
        write_json(&staging.join("installed.json"), &installed).await?;
        tokio::fs::rename(&staging, &destination).await?;
        crate::download::progress(
            &progress,
            package.size,
            package.size,
            DownloadStage::Complete,
        );
        Ok(installed)
    }
    pub async fn create_instance(&self, spec: InstanceSpec) -> Result<Instance> {
        let _guard = self.mutation.lock().await;
        ensure!(
            !spec.name.trim().is_empty() && !spec.name.contains(['\n', '\r', '\0']),
            "Instance name must be nonempty and single-line"
        );
        ensure!(
            (512..=65536).contains(&spec.memory_mb) && (1..=32).contains(&spec.cpu_cores),
            "Memory or CPU count is outside supported range"
        );
        ensure!(
            (320..=3840).contains(&spec.width)
                && (320..=3840).contains(&spec.height)
                && (120..=640).contains(&spec.density),
            "Display configuration is outside supported range"
        );
        ensure!(
            (2048..=262144).contains(&spec.data_disk_mb),
            "Data disk must be between 2 and 256 GiB"
        );
        let image = self
            .list_installed_images()
            .await?
            .into_iter()
            .find(|i| i.key == spec.image_key)
            .context("Selected image is not installed")?;
        let id = uuid::Uuid::new_v4().to_string();
        let directory = self.paths.instances.join(&id);
        let avd_home = directory.join("avd");
        let avd_name = format!("hub_{}", id.replace('-', ""));
        let avd_dir = avd_home.join(format!("{avd_name}.avd"));
        tokio::fs::create_dir_all(&avd_dir).await?;
        let canonical_image = tokio::fs::canonicalize(&image.directory).await?;
        let image_path = ini_path(&canonical_image)?;
        let avd_path = ini_path(&tokio::fs::canonicalize(&avd_dir).await?)?;
        let arch = match image.package.abi {
            Abi::X86_64 => "x86_64",
            Abi::Arm64V8a => "arm64",
            Abi::X86 => "x86",
            Abi::ArmeabiV7a => "arm",
        };
        let config = format!(
            "AvdId={avd_name}\navd.ini.displayname={}\navd.ini.encoding=UTF-8\nabi.type={}\nhw.cpu.arch={arch}\nhw.cpu.ncore={}\nhw.ramSize={}\nhw.lcd.width={}\nhw.lcd.height={}\nhw.lcd.density={}\nhw.gpu.enabled=yes\nhw.gpu.mode=auto\nhw.keyboard=yes\nhw.mainKeys=no\nhw.audioInput=yes\nhw.audioOutput=yes\nhw.battery=yes\nhw.accelerometer=yes\nhw.gps=yes\nhw.dPad=no\nhw.camera.back=none\nhw.camera.front=none\nhw.sdCard=no\ndisk.dataPartition.size={}M\nimage.sysdir.1={image_path}\ntag.id=default\ntag.display=Emulator Hub\nPlayStore.enabled={}\nfastboot.forceColdBoot=no\nfastboot.forceFastBoot=yes\nshowDeviceFrame=no\n",
            spec.name,
            image.package.abi,
            spec.cpu_cores,
            spec.memory_mb,
            spec.width,
            spec.height,
            spec.density,
            spec.data_disk_mb,
            image.package.id.contains("playstore")
        );
        tokio::fs::write(avd_dir.join("config.ini"), config).await?;
        tokio::fs::write(
            avd_home.join(format!("{avd_name}.ini")),
            format!(
                "avd.ini.encoding=UTF-8\npath={avd_path}\ntarget=android-{}\n",
                image.package.api
            ),
        )
        .await?;
        let instance = Instance {
            id,
            spec,
            directory,
            avd_name,
            avd_home,
            engine_version: None,
        };
        write_json(&instance.directory.join("instance.json"), &instance).await?;
        Ok(instance)
    }
    pub async fn rename_instance(&self, id: &str, name: &str) -> Result<Instance> {
        let _guard = self.mutation.lock().await;
        ensure!(
            !name.trim().is_empty() && !name.contains(['\n', '\r', '\0']),
            "Instance name must be nonempty and single-line"
        );
        let directory = self.instance_directory(id)?;
        let mut instance: Instance = read_json(&directory.join("instance.json")).await?;
        instance.spec.name = name.trim().into();
        write_json(&directory.join("instance.json"), &instance).await?;
        Ok(instance)
    }
    /// The caller confirms deletion of user data; a process lock prevents deleting a running VM.
    pub async fn delete_instance(&self, id: &str) -> Result<()> {
        let _guard = self.mutation.lock().await;
        let directory = self.instance_directory(id)?;
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("running.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock).context("Stop this instance before deleting it")?;
        // Rename while locked, then release so Windows permits removing the lock file.
        let trash = self
            .paths
            .instances
            .join(format!(".deleted-{}", uuid::Uuid::new_v4()));
        tokio::fs::rename(&directory, &trash).await?;
        drop(lock);
        tokio::fs::remove_dir_all(trash).await?;
        Ok(())
    }
    fn instance_directory(&self, id: &str) -> Result<PathBuf> {
        let parsed = uuid::Uuid::parse_str(id).context("Invalid instance ID")?;
        ensure!(
            parsed.to_string() == id,
            "Instance ID must use its canonical format"
        );
        Ok(self.paths.instances.join(id))
    }
    pub async fn local_image_metadata(&self, path: &Path) -> Result<LocalImageMetadata> {
        local_image_metadata(path).await
    }
}
/// Inspect only small metadata members; archive extraction still performs its full safety check.
pub async fn local_image_metadata(path: &Path) -> Result<LocalImageMetadata> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<LocalImageMetadata> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path)?)?;
        let mut properties = std::collections::HashMap::new();
        let mut count = 0;
        for index in 0..zip.len() {
            let mut file = zip.by_index(index)?;
            if file.name().rsplit('/').next() != Some("source.properties") { continue; }
            count += 1; ensure!(file.size() <= 1024*1024, "Image metadata exceeds 1 MiB");
            let mut text = String::new();
            (&mut file).take(1024 * 1024 + 1).read_to_string(&mut text)?;
            ensure!(text.len() <= 1024 * 1024, "Image metadata exceeds 1 MiB");
            for line in text.lines() { if let Some((k,v)) = line.split_once('=') { properties.insert(k.trim().to_string(), v.trim().to_string()); } }
        }
        ensure!(count == 1, "Local archive must contain one source.properties file for automatic import; use explicit metadata through the CLI otherwise");
        let api = properties.get("AndroidVersion.ApiLevel").context("Local image metadata is missing AndroidVersion.ApiLevel")?;
        let (major, minor) = api.split_once('.').unwrap_or((api, "0"));
        let api = ApiVersion { major:major.parse()?, minor:properties.get("AndroidVersion.ApiMinor").map(String::as_str).unwrap_or(minor).parse()? };
        let abi = properties.get("SystemImage.Abi").and_then(|v| Abi::from_sdk(v)).context("Local image metadata has an unsupported or missing ABI")?;
        let name = properties.get("Pkg.Desc").cloned().unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().into());
        let revision = properties.get("Pkg.Revision").cloned().unwrap_or_else(|| "1".into());
        Ok(LocalImageMetadata { name, api, abi, revision })
    }).await?
}
pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("emulator-hub/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("Too many redirects")
            } else if sources::validate_url(attempt.url().as_str()).is_err() {
                attempt.error("Unsafe download redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}
pub fn image_key(package: &ImagePackage) -> String {
    let identity = format!(
        "{}\0{}\0{}\0{}",
        package.source_id,
        package.id,
        package.revision,
        package.checksum.value.to_lowercase()
    );
    format!("{:x}", sha2::Sha256::digest(identity.as_bytes()))
}
fn ini_path(path: &Path) -> Result<String> {
    let value = path.to_str().context("AVD paths must be UTF-8")?;
    ensure!(
        !value.contains(['\n', '\r', '\0']),
        "AVD path must be single-line"
    );
    // std::fs::canonicalize returns Win32 verbatim paths. The emulator's INI
    // parser expects ordinary drive/UNC paths, including when slashes normalize.
    if let Some(unc) = value.strip_prefix("\\\\?\\UNC\\") {
        return Ok(format!("//{}", unc.replace('\\', "/")));
    }
    Ok(value
        .strip_prefix("\\\\?\\")
        .unwrap_or(value)
        .replace('\\', "/"))
}
async fn verify_image_metadata(path: &Path, package: &ImagePackage) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let file = match tokio::fs::File::open(path.join("source.properties")).await {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        file.metadata().await?.len() <= 1024 * 1024,
        "Image metadata exceeds 1 MiB"
    );
    let mut properties = String::new();
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut properties)
        .await?;
    ensure!(
        properties.len() <= 1024 * 1024,
        "Image metadata exceeds 1 MiB"
    );
    for line in properties.lines() {
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "SystemImage.Abi"
        {
            ensure!(
                Abi::from_sdk(value.trim()).as_ref() == Some(&package.abi),
                "Archive ABI differs from catalog"
            );
        }
    }
    Ok(())
}
pub async fn write_json(path: &Path, value: &(impl Serialize + ?Sized)) -> Result<()> {
    let parent = path.parent().context("JSON path has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let temp = parent.join(format!(".write-{}.json", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp, serde_json::to_vec_pretty(value)?).await?;
    // On Windows MoveFileEx's replacement semantics are provided by std::fs::rename.
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}
async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &tokio::fs::read(path)
            .await
            .with_context(|| format!("Read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid data in {}", path.display()))
}
async fn read_records<T: DeserializeOwned>(directory: &Path, name: &str) -> Result<Vec<T>> {
    let mut values = Vec::new();
    let mut entries = tokio::fs::read_dir(directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir()
            && !entry.file_name().to_string_lossy().starts_with('.')
        {
            let path = entry.path().join(name);
            if path.exists() {
                values.push(read_json(&path).await?);
            }
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_verbatim_paths_become_portable_avd_ini_paths() {
        assert_eq!(
            ini_path(Path::new(r"\\?\C:\Users\Name\Image")).unwrap(),
            "C:/Users/Name/Image"
        );
        assert_eq!(
            ini_path(Path::new(r"\\?\UNC\server\share\Image")).unwrap(),
            "//server/share/Image"
        );
    }
    #[tokio::test]
    async fn forged_zip_metadata_size_cannot_bypass_read_limit() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("forged.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file(
            "source.properties",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(&vec![b'a'; 1024 * 1024 + 8]).unwrap();
        zip.finish().unwrap();
        let mut bytes = std::fs::read(&archive).unwrap();
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
        bytes[22..26].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&archive, bytes).unwrap();
        let error = local_image_metadata(&archive).await.unwrap_err();
        assert!(error.to_string().contains("exceeds 1 MiB"), "{error:#}");
    }
    #[tokio::test]
    async fn instances_have_private_avds_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::open(HubPaths::new(dir.path())).await.unwrap();
        let metadata = LocalImageMetadata {
            name: "Test".into(),
            api: ApiVersion {
                major: 36,
                minor: 1,
            },
            abi: Abi::Arm64V8a,
            revision: "3".into(),
        };
        let archive = dir.path().join("image.zip");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            for file in ["system.img", "ramdisk.img", "kernel-ranchu"] {
                z.start_file(file, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(b"image").unwrap();
            }
            z.finish().unwrap();
        }
        let image = hub.import_local_zip(&archive, metadata).await.unwrap();
        let one = hub
            .create_instance(InstanceSpec::new("One", &image.key))
            .await
            .unwrap();
        let two = hub
            .create_instance(InstanceSpec::new("Two", &image.key))
            .await
            .unwrap();
        assert_ne!(one.avd_home, two.avd_home);
        assert_eq!(hub.list_instances().await.unwrap().len(), 2);
        let config = tokio::fs::read_to_string(
            one.avd_home
                .join(format!("{}.avd/config.ini", one.avd_name)),
        )
        .await
        .unwrap();
        assert!(config.contains("hw.cpu.arch=arm64"));
        assert!(config.contains("image.sysdir.1="));
        assert_eq!(
            Hub::open(HubPaths::new(dir.path()))
                .await
                .unwrap()
                .list_installed_images()
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
