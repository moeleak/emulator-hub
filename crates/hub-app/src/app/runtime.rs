use super::*;
use anyhow::{Context, Result};
use hub_engine::{EngineConfig, EngineController, Frame, RunningInstance};
use tokio::sync::{mpsc, watch};

pub(super) async fn load(hub: Arc<Hub>) -> Result<Data> {
    let (instances, mut images, sources) = tokio::try_join!(
        hub.list_instances(),
        hub.list_installed_images(),
        hub.sources()
    )?;
    images.sort_by_key(|image| image.package.source_id != "lineageos-avd");
    let (engine, engine_error) =
        match hub_engine::provision::load_installed_engine(hub.paths()).await {
            Ok(engine) => (engine, None),
            Err(error) => (None, Some(error.to_string())),
        };
    Ok(Data {
        instances,
        images,
        sources,
        engine,
        engine_error,
    })
}

pub(super) fn start_download(app: &mut App, package: ImagePackage) -> Task<Message> {
    let Some(hub) = app.hub.clone() else {
        return Task::none();
    };
    let key = format!("{}:{}:{}", package.source_id, package.id, package.revision);
    if app.downloads.get(&key).is_some_and(|d| !d.finished) {
        return Task::none();
    }
    let (tx, rx) = mpsc::unbounded_channel();
    let (control, receiver) = watch::channel(hub_core::DownloadControl::Running);
    app.downloads.insert(
        key.clone(),
        DownloadState {
            name: package.name.clone(),
            downloaded: 0,
            total: package.size,
            status: "Downloading".into(),
            paused: false,
            finished: false,
            progress: rx,
            control,
        },
    );
    app.navigate(Page::Downloads);
    Task::perform(
        async move {
            hub.install_image_controlled(&package, Some(tx), receiver)
                .await
                .map_err(|e| format!("{e:#}"))
        },
        move |r| Message::DownloadDone(key.clone(), r),
    )
}

pub(super) fn apply_progress(state: &mut DownloadState, progress: hub_core::DownloadProgress) {
    state.downloaded = progress.downloaded;
    state.total = progress.total;
    state.status = format!("{:?}", progress.stage);
}

pub(super) fn save_sources(app: &App) -> Task<Message> {
    let Some(hub) = app.hub.clone() else {
        return Task::none();
    };
    let sources = app.data.sources.clone();
    Task::perform(
        async move {
            hub.save_sources(&sources)
                .await
                .map(|_| "Sources updated".into())
                .map_err(error)
        },
        Message::Action,
    )
}

pub(super) fn open_data(hub: &Hub) -> Result<()> {
    open::that(&hub.paths().root)?;
    Ok(())
}

pub(super) async fn import_image(hub: Arc<Hub>, path: PathBuf) -> Result<InstalledImage> {
    let metadata = hub_core::local_image_metadata(&path).await?;
    hub.import_local_zip(&path, metadata).await
}

pub(super) async fn rename(hub: Arc<Hub>, id: String, name: String) -> Result<()> {
    hub.rename_instance(&id, &name).await.map(|_| ())
}
pub(super) async fn delete(hub: Arc<Hub>, id: String) -> Result<()> {
    hub.delete_instance(&id).await
}

#[derive(Clone)]
pub(super) struct Session {
    engine: RunningInstance,
    frames: watch::Receiver<Option<Frame>>,
    errors: watch::Receiver<Option<String>>,
    input_tx: mpsc::UnboundedSender<screen::Input>,
}

pub(super) async fn launch(
    preferences: Preferences,
    instance: Instance,
    image: InstalledImage,
) -> Result<Session> {
    anyhow::ensure!(
        !preferences.adb.as_os_str().is_empty(),
        "Install or select ADB in Settings first."
    );
    let config = EngineConfig {
        emulator: preferences.emulator.clone(),
        adb: preferences.adb,
        sdk_root: preferences
            .emulator
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_owned()),
        startup_timeout_secs: 180,
        audio: preferences.audio,
        gpu: preferences.graphics.emulator_mode().into(),
        ..Default::default()
    };
    let controller = EngineController::for_instance(config, &instance).await?;
    anyhow::ensure!(
        !controller.config().emulator.as_os_str().is_empty(),
        "Install or select an Emulator engine in Settings first."
    );
    let engine = controller.launch(&instance, &image).await?;
    let frames = engine.frames();
    let errors = engine.stream_errors();
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let input_engine = engine.clone();
    tokio::spawn(async move {
        while let Some(input) = input_rx.recv().await {
            let result = match input {
                screen::Input::Touch { x, y, down } => input_engine.send_touch(x, y, down).await,
                screen::Input::Key { key, down } if !key.is_ascii() => {
                    if down {
                        paste_text(&input_engine, &key).await
                    } else {
                        Ok(())
                    }
                }
                screen::Input::Key { key, down } => input_engine.send_key(&key, down).await,
                screen::Input::Text(text) if text.is_ascii() => input_engine.send_text(&text).await,
                screen::Input::Text(text) => paste_text(&input_engine, &text).await,
                screen::Input::Wheel { dx, dy, .. } => input_engine.send_wheel(dx, dy).await,
                screen::Input::ReleaseAll => input_engine.release_inputs().await,
            };
            if let Err(e) = result {
                tracing::warn!(error=%e, "Android input rejected");
            }
        }
        let _ = input_engine.release_inputs().await;
    });
    Ok(Session {
        engine,
        frames,
        errors,
        input_tx,
    })
}

async fn paste_text(engine: &RunningInstance, text: &str) -> Result<()> {
    engine.set_clipboard(text).await?;
    engine.press_key("Paste").await
}

impl Session {
    pub(super) fn current_frame(&self) -> Option<(iced::widget::image::Handle, u32, u32)> {
        let frame = self.frames.borrow().clone()?;
        Some((
            iced::widget::image::Handle::from_rgba(
                frame.width,
                frame.height,
                frame.rgba.as_ref().clone(),
            ),
            frame.width,
            frame.height,
        ))
    }
    pub(super) fn latest_frame(
        &mut self,
    ) -> Option<Option<(iced::widget::image::Handle, u32, u32)>> {
        if !self.frames.has_changed().unwrap_or(false) {
            return None;
        }
        let frame = self.frames.borrow_and_update().clone();
        Some(frame.map(|frame| {
            (
                iced::widget::image::Handle::from_rgba(
                    frame.width,
                    frame.height,
                    frame.rgba.as_ref().clone(),
                ),
                frame.width,
                frame.height,
            )
        }))
    }
    pub(super) fn stream_error(&mut self) -> Option<String> {
        if !self.errors.has_changed().unwrap_or(false) {
            return None;
        }
        self.errors.borrow_and_update().clone()
    }
    pub(super) fn release_input(&self) {
        self.input(screen::Input::ReleaseAll);
    }
    pub(super) fn input(&self, input: screen::Input) {
        let _ = self.input_tx.send(input);
    }
    pub(super) async fn stop(&self) -> Result<()> {
        self.engine.stop().await
    }
    pub(super) async fn install_apk(&self, path: PathBuf) -> Result<()> {
        self.engine.install_apk(&path).await?;
        Ok(())
    }
    pub(super) async fn snapshot(&self, save: bool) -> Result<()> {
        if save {
            self.engine.save_snapshot("hub_manual").await
        } else {
            self.engine.load_snapshot("hub_manual").await
        }
    }
    pub(super) async fn capture(&self) -> Result<String> {
        let bytes = self.engine.screenshot_png().await?;
        let filename = format!(
            "Android-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
        );
        if let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(filename)
            .save_file()
            .await
        {
            tokio::fs::write(file.path(), bytes).await?;
            Ok(format!("Saved {}", file.path().display()))
        } else {
            Ok("Screenshot cancelled".into())
        }
    }
    pub(super) async fn set_clipboard(&self, text: String) -> Result<()> {
        self.engine.set_clipboard(&text).await
    }
    pub(super) async fn clipboard(&self) -> Result<String> {
        self.engine.get_clipboard().await
    }
}

pub(super) async fn install_engine(
    tools: Vec<hub_engine::provision::ToolPackage>,
) -> Result<(PathBuf, PathBuf)> {
    let paths = HubPaths::discover()?;
    let config = hub_engine::provision::install_tools(&paths, &tools, None)
        .await
        .context("Install emulator tools")?;
    Ok((config.emulator, config.adb))
}
