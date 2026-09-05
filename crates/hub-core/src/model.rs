use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Abi {
    #[serde(rename = "x86_64", alias = "x86-64")]
    X86_64,
    Arm64V8a,
    X86,
    ArmeabiV7a,
}
impl Abi {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Arm64V8a => "arm64-v8a",
            Self::X86 => "x86",
            Self::ArmeabiV7a => "armeabi-v7a",
        }
    }
    pub fn from_sdk(value: &str) -> Option<Self> {
        match value {
            "x86_64" | "x86-64" => Some(Self::X86_64),
            "arm64-v8a" | "aarch64" => Some(Self::Arm64V8a),
            "x86" => Some(Self::X86),
            "armeabi-v7a" => Some(Self::ArmeabiV7a),
            _ => None,
        }
    }
    /// v1 supports hardware acceleration on matching 64-bit hosts only.
    pub fn compatible_with_host(&self) -> bool {
        matches!(
            (std::env::consts::ARCH, self),
            ("x86_64", Self::X86_64) | ("aarch64", Self::Arm64V8a)
        )
    }
}
impl fmt::Display for Abi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ApiVersion {
    pub major: u32,
    #[serde(default)]
    pub minor: u32,
}
impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.minor == 0 {
            write!(f, "{}", self.major)
        } else {
            write!(f, "{}.{}", self.major, self.minor)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChecksumAlgorithm {
    Sha256,
    Sha1,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checksum {
    pub algorithm: ChecksumAlgorithm,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePackage {
    /// Unique within the source. Installation keys include source, revision and digest.
    pub id: String,
    #[serde(default)]
    pub source_id: String,
    pub name: String,
    pub revision: String,
    pub api: ApiVersion,
    pub abi: Abi,
    pub url: String,
    pub size: u64,
    pub checksum: Checksum,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub license_id: String,
    #[serde(default)]
    pub min_engine_version: Option<String>,
    #[serde(default)]
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    #[serde(default)]
    pub images: Vec<ImagePackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    HubJson,
    SdkXml,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub enabled: bool,
}
impl SourceConfig {
    pub fn defaults() -> Vec<Self> {
        vec![
            Self { id: "lineageos-avd".into(), name: "LineageOS AVD".into(), kind: SourceKind::HubJson, url: "https://raw.githubusercontent.com/lineageos-avd/android/avd-main/images/catalog-v1.json".into(), enabled: true },
            Self { id: "google-android".into(), name: "Google Android".into(), kind: SourceKind::SdkXml, url: "https://dl.google.com/android/repository/sys-img/android/sys-img2-4.xml".into(), enabled: true },
            Self { id: "google-apis".into(), name: "Google APIs".into(), kind: SourceKind::SdkXml, url: "https://dl.google.com/android/repository/sys-img/google_apis/sys-img2-4.xml".into(), enabled: true },
            Self { id: "google-play".into(), name: "Google Play".into(), kind: SourceKind::SdkXml, url: "https://dl.google.com/android/repository/sys-img/google_apis_playstore/sys-img2-4.xml".into(), enabled: true },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledImage {
    pub key: String,
    pub package: ImagePackage,
    pub directory: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceSpec {
    pub name: String,
    pub image_key: String,
    pub memory_mb: u32,
    pub cpu_cores: u32,
    pub width: u32,
    pub height: u32,
    pub density: u32,
    pub data_disk_mb: u32,
}
impl InstanceSpec {
    pub fn new(name: impl Into<String>, image_key: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image_key: image_key.into(),
            memory_mb: 4096,
            cpu_cores: 4,
            width: 1080,
            height: 1920,
            density: 420,
            data_disk_mb: 8192,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub spec: InstanceSpec,
    pub directory: PathBuf,
    pub avd_name: String,
    pub avd_home: PathBuf,
    /// Filled by the engine after the first launch to keep snapshots compatible.
    #[serde(default)]
    pub engine_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalImageMetadata {
    pub name: String,
    pub api: ApiVersion,
    pub abi: Abi,
    pub revision: String,
}

#[derive(Debug, Clone, Default)]
pub struct CatalogRefresh {
    pub images: Vec<ImagePackage>,
    pub errors: Vec<SourceFailure>,
}
#[derive(Debug, Clone)]
pub struct SourceFailure {
    pub source_id: String,
    pub message: String,
}
