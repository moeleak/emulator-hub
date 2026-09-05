use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    #[default]
    Chinese,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Graphics {
    #[default]
    Auto,
    Host,
    Software,
}

impl Graphics {
    pub fn emulator_mode(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Host => "host",
            Self::Software => "swiftshader_indirect",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub appearance: Appearance,
    pub language: Language,
    pub emulator: PathBuf,
    pub adb: PathBuf,
    pub audio: bool,
    pub graphics: Graphics,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            language: Language::Chinese,
            emulator: PathBuf::new(),
            adb: PathBuf::new(),
            audio: true,
            graphics: Graphics::Auto,
        }
    }
}

impl Preferences {
    fn path() -> PathBuf {
        hub_core::HubPaths::discover()
            .map(|p| p.root.join("preferences.json"))
            .unwrap_or_else(|_| PathBuf::from(".emulator-hub/appearance.json"))
    }

    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().expect("preferences parent"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        // Persist through the same directory, keeping the original until replacement.
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
