#[cfg(test)]
mod render_tests;
mod runtime;
mod views;

use crate::{
    i18n::tr,
    preferences::{Appearance, Graphics, Language, Preferences},
    screen,
};
use hub_core::{
    Hub, HubPaths, ImagePackage, InstalledImage, Instance, InstanceSpec, SourceConfig, SourceKind,
};
use iced::{Size, Subscription, Task, time::Instant};
use material_ui_rs::{self as material, widget::navigation};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

pub fn run() -> iced::Result {
    let mut window =
        material::window_with_min_size(Size::new(1240.0, 820.0), Size::new(840.0, 620.0));
    window.exit_on_close_request = false;
    material::application(boot, update, views::view)
        .title("Emulator Hub")
        .theme(theme)
        .subscription(subscription)
        .font(include_bytes!("../../assets/fonts/NotoSansSC-Core-0a7ff25a.otf").as_slice())
        .font(include_bytes!("../../assets/fonts/NotoSansSC-faa6c9df.otf").as_slice())
        .window(window)
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Devices,
    Images,
    Downloads,
    Settings,
}

const NAV_ZH: [navigation::Destination<Page>; 4] = [
    navigation::Destination::new(Page::Devices, "devices", "设备"),
    navigation::Destination::new(Page::Images, "android", "镜像库"),
    navigation::Destination::new(Page::Downloads, "download", "下载"),
    navigation::Destination::new(Page::Settings, "settings", "设置"),
];
const NAV_EN: [navigation::Destination<Page>; 4] = [
    navigation::Destination::new(Page::Devices, "devices", "Devices"),
    navigation::Destination::new(Page::Images, "android", "Images"),
    navigation::Destination::new(Page::Downloads, "download", "Downloads"),
    navigation::Destination::new(Page::Settings, "settings", "Settings"),
];

#[derive(Clone)]
enum Message {
    Booted(Result<Arc<Hub>, String>),
    Loaded(Result<Data, String>),
    Catalog(Result<hub_core::CatalogRefresh, String>),
    Navigate(Page),
    Menu,
    Tick(Instant),
    Resize(Size),
    Refresh,
    Search(String),
    CompatibleOnly(bool),
    NewInstance,
    NewFromImage(String),
    InstanceName(String),
    InstanceImage(ImageChoice),
    Memory(String),
    Cpus(String),
    Width(String),
    Height(String),
    Create,
    Created(Result<Instance, String>),
    Download(ImagePackage),
    ConfirmDownload,
    PauseDownload(String),
    CancelDownload(String),
    DownloadDone(String, Result<InstalledImage, String>),
    Import,
    ImportPicked(Option<PathBuf>),
    Imported(Result<InstalledImage, String>),
    OpenInstance(String),
    Launch(String),
    Launched(String, Result<runtime::Session, String>),
    BackToDevices,
    Stop(String),
    Stopped(String, Result<(), String>),
    InstallApk(String),
    ApkPicked(String, Option<PathBuf>),
    ScreenInput(screen::Input),
    AndroidKey(&'static str),
    Snapshot(bool),
    Capture,
    ClipboardToDevice,
    ClipboardFromDevice,
    HostClipboard(Option<String>),
    DeviceClipboard(Result<String, String>),
    Rename(String),
    RenameValue(String),
    ConfirmRename,
    Delete(String),
    ConfirmDelete,
    Action(Result<String, String>),
    Appearance(Appearance),
    Language(Language),
    Audio(bool),
    Graphics(Graphics),
    PickEngine,
    PickAdb,
    EnginePicked(Option<PathBuf>),
    AdbPicked(Option<PathBuf>),
    InstallEngine,
    InstallOfficialEngine,
    ToolsDiscovered(Result<Vec<hub_engine::provision::ToolPackage>, String>),
    ConfirmTools,
    EngineInstalled(Result<(PathBuf, PathBuf), String>),
    SourceName(String),
    SourceUrl(String),
    SourceKind(SourceKind),
    AddSource,
    ToggleSource(String),
    RemoveSource(String),
    Dismiss,
    OpenFolder,
    OpenProject,
    Quit(iced::window::Id),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Message")
            .field(&std::mem::discriminant(self))
            .finish()
    }
}

#[derive(Clone, Default)]
struct Data {
    instances: Vec<Instance>,
    images: Vec<InstalledImage>,
    sources: Vec<SourceConfig>,
    engine: Option<hub_engine::EngineConfig>,
    engine_error: Option<String>,
}

enum Dialog {
    Create,
    License(Box<ImagePackage>),
    Tools(Vec<hub_engine::provision::ToolPackage>),
    Rename(String),
    Delete(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageChoice {
    key: String,
    label: String,
}
impl From<&InstalledImage> for ImageChoice {
    fn from(image: &InstalledImage) -> Self {
        Self {
            key: image.key.clone(),
            label: format!(
                "{} · {} · r{} · {}",
                image.package.name,
                image.package.abi,
                image.package.revision,
                image.package.source_id
            ),
        }
    }
}
impl std::fmt::Display for ImageChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

struct DownloadState {
    name: String,
    downloaded: u64,
    total: u64,
    status: String,
    paused: bool,
    finished: bool,
    progress: tokio::sync::mpsc::UnboundedReceiver<hub_core::DownloadProgress>,
    control: tokio::sync::watch::Sender<hub_core::DownloadControl>,
}

struct App {
    hub: Option<Arc<Hub>>,
    preferences: Preferences,
    navigation: navigation::NavigationState<Page>,
    window_size: Size,
    data: Data,
    catalog: Vec<ImagePackage>,
    catalog_errors: Vec<String>,
    search: String,
    compatible_only: bool,
    refreshing: bool,
    busy: bool,
    phase: f32,
    notification: Option<(String, bool)>,
    dialog: Option<Dialog>,
    name: String,
    image_key: Option<String>,
    memory: String,
    cpus: String,
    width: String,
    height: String,
    source_name: String,
    source_url: String,
    source_kind: SourceKind,
    downloads: HashMap<String, DownloadState>,
    sessions: HashMap<String, runtime::Session>,
    selected: Option<String>,
    launching: Vec<String>,
    frame: Option<(iced::widget::image::Handle, u32, u32)>,
}

fn boot() -> (App, Task<Message>) {
    let preferences = Preferences::load();
    let app = App {
        hub: None,
        preferences,
        navigation: navigation::NavigationState::new(Page::Devices),
        window_size: Size::new(1240.0, 820.0),
        data: Data::default(),
        catalog: vec![],
        catalog_errors: vec![],
        search: String::new(),
        compatible_only: true,
        refreshing: false,
        busy: false,
        phase: 0.0,
        notification: None,
        dialog: None,
        name: "LineageOS".into(),
        image_key: None,
        memory: "4096".into(),
        cpus: "4".into(),
        width: "1080".into(),
        height: "1920".into(),
        source_name: String::new(),
        source_url: String::new(),
        source_kind: SourceKind::HubJson,
        downloads: HashMap::new(),
        sessions: HashMap::new(),
        selected: None,
        launching: vec![],
        frame: None,
    };
    (
        app,
        Task::perform(
            async { Hub::open(HubPaths::discover()?).await.map(Arc::new) },
            |result: anyhow::Result<Arc<Hub>>| Message::Booted(result.map_err(error)),
        ),
    )
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn theme(app: &App) -> Option<material::Theme> {
    let dark = match app.preferences.appearance {
        Appearance::System => return None,
        Appearance::Dark => true,
        Appearance::Light => false,
    };
    Some(material::Theme::new(
        "Emulator Hub",
        material::widget::theme_picker::MaterialColor::Teal.color_scheme(dark),
    ))
}

fn subscription(app: &App) -> Subscription<Message> {
    Subscription::batch([
        iced::time::every(Duration::from_millis(if app.selected.is_some() {
            16
        } else {
            100
        }))
        .map(Message::Tick),
        app.navigation.subscription(Message::Tick),
        iced::event::listen_with(|event, _, id| match event {
            iced::Event::Window(iced::window::Event::Resized(size)) => Some(Message::Resize(size)),
            iced::Event::Window(iced::window::Event::CloseRequested) => Some(Message::Quit(id)),
            _ => None,
        }),
    ])
}

impl App {
    fn t(&self, zh: &'static str, en: &'static str) -> &'static str {
        tr(self.preferences.language, zh, en)
    }
    fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.notification = Some((text.into(), error));
    }
    fn save_preferences(&mut self) {
        if let Err(e) = self.preferences.save() {
            self.notice(e.to_string(), true);
        }
    }
    fn navigate(&mut self, page: Page) {
        self.navigation.select_now_for_size(page, self.window_size);
        if let Some(id) = self.selected.take()
            && let Some(session) = self.sessions.get(&id)
        {
            session.release_input();
        }
        self.frame = None;
    }
    fn reload(&self) -> Task<Message> {
        let Some(hub) = self.hub.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { runtime::load(hub).await.map_err(error) },
            Message::Loaded,
        )
    }
    fn current(&self) -> Option<&runtime::Session> {
        self.selected.as_ref().and_then(|id| self.sessions.get(id))
    }
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Booted(result) => match result {
            Ok(hub) => {
                app.hub = Some(hub);
                return Task::batch([app.reload(), Task::done(Message::Refresh)]);
            }
            Err(e) => app.notice(e, true),
        },
        Message::Loaded(result) => match result {
            Ok(data) => {
                if app.preferences.emulator.as_os_str().is_empty()
                    && let Some(engine) = &data.engine
                {
                    app.preferences.emulator = engine.emulator.clone();
                    app.preferences.adb = engine.adb.clone();
                }
                if let Some(error) = &data.engine_error {
                    app.notice(error.clone(), true);
                }
                app.data = data;
            }
            Err(e) => app.notice(e, true),
        },
        Message::Refresh => {
            if app.refreshing {
                return Task::none();
            }
            if let Some(hub) = app.hub.clone() {
                app.refreshing = true;
                return Task::perform(
                    async move { hub.refresh_catalog().await.map_err(error) },
                    Message::Catalog,
                );
            }
        }
        Message::Catalog(result) => {
            app.refreshing = false;
            match result {
                Ok(catalog) => {
                    app.catalog = catalog.images;
                    app.catalog_errors = catalog
                        .errors
                        .iter()
                        .map(|e| format!("{}: {}", e.source_id, e.message))
                        .collect();
                }
                Err(e) => app.notice(e, true),
            }
        }
        Message::Navigate(page) => app.navigate(page),
        Message::Menu => app.navigation.toggle_menu_now(),
        Message::Resize(size) => app.window_size = size,
        Message::Tick(now) => {
            app.navigation.advance_frame(now);
            app.phase = (app.phase + 0.016) % 1.0;
            for download in app.downloads.values_mut() {
                while let Ok(progress) = download.progress.try_recv() {
                    runtime::apply_progress(download, progress);
                }
            }
            if let Some(id) = &app.selected
                && let Some(session) = app.sessions.get_mut(id)
                && let Some(frame) = session.latest_frame()
            {
                if frame.is_none() {
                    session.release_input();
                }
                app.frame = frame;
            }
            let errors: Vec<_> = app
                .sessions
                .iter_mut()
                .filter_map(|(id, session)| session.stream_error().map(|e| (id.clone(), e)))
                .collect();
            let mut cleanup = Vec::new();
            for (id, error) in errors {
                app.notice(error, true);
                if let Some(session) = app.sessions.remove(&id) {
                    cleanup.push(Task::perform(
                        async move { session.stop().await.map_err(|e| e.to_string()) },
                        move |r| Message::Stopped(id.clone(), r),
                    ));
                }
            }
            if !cleanup.is_empty() {
                return Task::batch(cleanup);
            }
        }
        Message::Search(value) => app.search = value,
        Message::CompatibleOnly(value) => app.compatible_only = value,
        Message::NewInstance => {
            if app.data.images.is_empty() {
                app.navigate(Page::Images);
                app.notice(
                    app.t(
                        "请先下载或导入一个系统镜像。",
                        "Download or import a system image first.",
                    ),
                    false,
                );
            } else {
                app.image_key = app.data.images.first().map(|i| i.key.clone());
                app.dialog = Some(Dialog::Create);
            }
        }
        Message::NewFromImage(key) => {
            app.image_key = Some(key);
            app.dialog = Some(Dialog::Create);
        }
        Message::InstanceName(value) | Message::RenameValue(value) => app.name = value,
        Message::InstanceImage(choice) => app.image_key = Some(choice.key),
        Message::Memory(value) => app.memory = value,
        Message::Cpus(value) => app.cpus = value,
        Message::Width(value) => app.width = value,
        Message::Height(value) => app.height = value,
        Message::Create => {
            let Some(hub) = app.hub.clone() else {
                return Task::none();
            };
            let Some(key) = app.image_key.clone() else {
                return Task::none();
            };
            let parsed = (|| -> anyhow::Result<InstanceSpec> {
                let mut spec = InstanceSpec::new(app.name.trim(), key);
                spec.memory_mb = app.memory.parse()?;
                spec.cpu_cores = app.cpus.parse()?;
                spec.width = app.width.parse()?;
                spec.height = app.height.parse()?;
                Ok(spec)
            })();
            match parsed {
                Ok(spec) => {
                    app.busy = true;
                    return Task::perform(
                        async move { hub.create_instance(spec).await.map_err(error) },
                        Message::Created,
                    );
                }
                Err(_) => app.notice(
                    app.t(
                        "请输入有效的内存、核心数和分辨率。",
                        "Enter valid memory, CPU and resolution values.",
                    ),
                    true,
                ),
            }
        }
        Message::Created(result) => {
            app.busy = false;
            match result {
                Ok(_) => {
                    app.dialog = None;
                    app.navigate(Page::Devices);
                    return app.reload();
                }
                Err(e) => app.notice(e, true),
            }
        }
        Message::Download(package) => app.dialog = Some(Dialog::License(Box::new(package))),
        Message::ConfirmDownload => {
            if let Some(Dialog::License(package)) = app.dialog.take() {
                return runtime::start_download(app, *package);
            }
        }
        Message::PauseDownload(key) => {
            if let Some(d) = app.downloads.get_mut(&key) {
                d.paused = !d.paused;
                let _ = d.control.send(if d.paused {
                    hub_core::DownloadControl::Paused
                } else {
                    hub_core::DownloadControl::Running
                });
            }
        }
        Message::CancelDownload(key) => {
            if let Some(d) = app.downloads.get_mut(&key) {
                let _ = d.control.send(hub_core::DownloadControl::Cancelled);
            }
        }
        Message::DownloadDone(key, result) => {
            if let Some(d) = app.downloads.get_mut(&key) {
                d.finished = true;
                d.status = match &result {
                    Ok(_) => "Installed".into(),
                    Err(e) => e.clone(),
                };
            }
            match result {
                Ok(_) => return app.reload(),
                Err(e) => app.notice(e, true),
            }
        }
        Message::Import => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("Android system image", &["zip"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_owned())
                },
                Message::ImportPicked,
            );
        }
        Message::ImportPicked(Some(path)) => {
            if let Some(hub) = app.hub.clone() {
                app.busy = true;
                return Task::perform(
                    async move { runtime::import_image(hub, path).await.map_err(error) },
                    Message::Imported,
                );
            }
        }
        Message::ImportPicked(None) => {}
        Message::Imported(result) => {
            app.busy = false;
            match result {
                Ok(_) => return app.reload(),
                Err(e) => app.notice(e, true),
            }
        }
        Message::OpenInstance(id) => {
            app.navigate(Page::Devices);
            app.frame = app
                .sessions
                .get(&id)
                .and_then(runtime::Session::current_frame);
            app.selected = Some(id);
        }
        Message::BackToDevices => app.navigate(Page::Devices),
        Message::Launch(id) => {
            if app.launching.contains(&id) {
                return Task::none();
            }
            if app.sessions.contains_key(&id) {
                app.navigate(Page::Devices);
                app.frame = app
                    .sessions
                    .get(&id)
                    .and_then(runtime::Session::current_frame);
                app.selected = Some(id);
                return Task::none();
            }
            let Some(instance) = app.data.instances.iter().find(|i| i.id == id).cloned() else {
                return Task::none();
            };
            let Some(image) = app
                .data
                .images
                .iter()
                .find(|i| i.key == instance.spec.image_key)
                .cloned()
            else {
                return Task::none();
            };
            app.launching.push(id.clone());
            app.navigate(Page::Devices);
            app.selected = Some(id.clone());
            let prefs = app.preferences.clone();
            return Task::perform(
                async move { runtime::launch(prefs, instance, image).await.map_err(error) },
                move |r| Message::Launched(id.clone(), r),
            );
        }
        Message::Launched(id, result) => {
            app.launching.retain(|i| i != &id);
            match result {
                Ok(session) => {
                    app.sessions.insert(id, session);
                }
                Err(e) => {
                    if app.selected.as_deref() == Some(&id) {
                        app.selected = None;
                        app.frame = None;
                    }
                    app.notice(e, true);
                }
            }
        }
        Message::Stop(id) => {
            if let Some(session) = app.sessions.get(&id).cloned() {
                return Task::perform(
                    async move { session.stop().await.map_err(error) },
                    move |r| Message::Stopped(id.clone(), r),
                );
            }
        }
        Message::Stopped(id, result) => {
            app.sessions.remove(&id);
            if app.selected.as_deref() == Some(&id) {
                app.selected = None;
                app.frame = None;
            }
            if let Err(e) = result {
                app.notice(e, true);
            }
        }
        Message::InstallApk(id) => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("Android application", &["apk"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_owned())
                },
                move |p| Message::ApkPicked(id.clone(), p),
            );
        }
        Message::ApkPicked(id, Some(path)) => {
            if let Some(session) = app.sessions.get(&id).cloned() {
                return Task::perform(
                    async move {
                        session
                            .install_apk(path)
                            .await
                            .map(|_| "APK installed".into())
                            .map_err(error)
                    },
                    Message::Action,
                );
            }
        }
        Message::ApkPicked(_, None) => {}
        Message::ScreenInput(input) => {
            if let Some(session) = app.current() {
                session.input(input);
            }
        }
        Message::AndroidKey(key) => {
            if let Some(session) = app.current() {
                session.input(screen::Input::Key {
                    key: key.into(),
                    down: true,
                });
                session.input(screen::Input::Key {
                    key: key.into(),
                    down: false,
                });
            }
        }
        Message::Snapshot(save) => {
            if let Some(session) = app.current().cloned() {
                return Task::perform(
                    async move {
                        session
                            .snapshot(save)
                            .await
                            .map(|_| {
                                if save {
                                    "Snapshot saved"
                                } else {
                                    "Snapshot restored"
                                }
                                .into()
                            })
                            .map_err(error)
                    },
                    Message::Action,
                );
            }
        }
        Message::Capture => {
            if let Some(session) = app.current().cloned() {
                return Task::perform(
                    async move { session.capture().await.map_err(error) },
                    Message::Action,
                );
            }
        }
        Message::ClipboardToDevice => return iced::clipboard::read().map(Message::HostClipboard),
        Message::HostClipboard(Some(text)) => {
            if let Some(session) = app.current().cloned() {
                return Task::perform(
                    async move {
                        session
                            .set_clipboard(text)
                            .await
                            .map(|_| "Clipboard sent".into())
                            .map_err(error)
                    },
                    Message::Action,
                );
            }
        }
        Message::HostClipboard(None) => {}
        Message::ClipboardFromDevice => {
            if let Some(session) = app.current().cloned() {
                return Task::perform(
                    async move { session.clipboard().await.map_err(error) },
                    Message::DeviceClipboard,
                );
            }
        }
        Message::DeviceClipboard(Ok(text)) => return iced::clipboard::write(text),
        Message::DeviceClipboard(Err(e)) => app.notice(e, true),
        Message::Rename(id) => {
            if let Some(i) = app.data.instances.iter().find(|i| i.id == id) {
                app.name = i.spec.name.clone();
                app.dialog = Some(Dialog::Rename(id));
            }
        }
        Message::ConfirmRename => {
            if let (Some(hub), Some(Dialog::Rename(id))) = (app.hub.clone(), app.dialog.take()) {
                let name = app.name.clone();
                return Task::perform(
                    async move {
                        runtime::rename(hub, id, name)
                            .await
                            .map(|_| "Device renamed".into())
                            .map_err(error)
                    },
                    Message::Action,
                );
            }
        }
        Message::Delete(id) => app.dialog = Some(Dialog::Delete(id)),
        Message::ConfirmDelete => {
            if let (Some(hub), Some(Dialog::Delete(id))) = (app.hub.clone(), app.dialog.take()) {
                if app.sessions.contains_key(&id) {
                    app.notice(app.t("请先关闭设备。", "Stop the device first."), true);
                } else {
                    return Task::perform(
                        async move {
                            runtime::delete(hub, id)
                                .await
                                .map(|_| "Device deleted".into())
                                .map_err(error)
                        },
                        Message::Action,
                    );
                }
            }
        }
        Message::Action(result) => {
            match result {
                Ok(text) => app.notice(text, false),
                Err(e) => app.notice(e, true),
            };
            return app.reload();
        }
        Message::Appearance(value) => {
            app.preferences.appearance = value;
            app.save_preferences();
        }
        Message::Language(value) => {
            app.preferences.language = value;
            app.save_preferences();
        }
        Message::Audio(value) => {
            app.preferences.audio = value;
            app.save_preferences();
        }
        Message::Graphics(value) => {
            app.preferences.graphics = value;
            app.save_preferences();
        }
        Message::PickEngine => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select emulator executable")
                        .pick_file()
                        .await
                        .map(|f| f.path().to_owned())
                },
                Message::EnginePicked,
            );
        }
        Message::PickAdb => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select adb executable")
                        .pick_file()
                        .await
                        .map(|f| f.path().to_owned())
                },
                Message::AdbPicked,
            );
        }
        Message::EnginePicked(Some(path)) => {
            app.preferences.emulator = path;
            app.save_preferences();
        }
        Message::AdbPicked(Some(path)) => {
            app.preferences.adb = path;
            app.save_preferences();
        }
        Message::EnginePicked(None) | Message::AdbPicked(None) => {}
        Message::InstallEngine => {
            app.busy = true;
            return Task::perform(
                async {
                    hub_engine::provision::discover_default_tools()
                        .await
                        .map_err(error)
                },
                Message::ToolsDiscovered,
            );
        }
        Message::InstallOfficialEngine => {
            app.busy = true;
            return Task::perform(
                async {
                    hub_engine::provision::discover_official_tools()
                        .await
                        .map_err(error)
                },
                Message::ToolsDiscovered,
            );
        }
        Message::ToolsDiscovered(result) => {
            app.busy = false;
            match result {
                Ok(tools) => app.dialog = Some(Dialog::Tools(tools)),
                Err(e) => app.notice(e, true),
            }
        }
        Message::ConfirmTools => {
            if let Some(Dialog::Tools(tools)) = app.dialog.take() {
                app.busy = true;
                return Task::perform(
                    async move { runtime::install_engine(tools).await.map_err(error) },
                    Message::EngineInstalled,
                );
            }
        }
        Message::EngineInstalled(result) => {
            app.busy = false;
            match result {
                Ok((engine, adb)) => {
                    app.preferences.emulator = engine;
                    app.preferences.adb = adb;
                    app.save_preferences();
                    app.notice(app.t("引擎已安装。", "Engine installed."), false);
                }
                Err(e) => app.notice(e, true),
            }
        }
        Message::SourceName(value) => app.source_name = value,
        Message::SourceUrl(value) => app.source_url = value,
        Message::SourceKind(kind) => app.source_kind = kind,
        Message::AddSource => {
            if app.source_name.trim().is_empty() || app.source_url.trim().is_empty() {
                return Task::none();
            }
            if let Err(error) = hub_core::sources::validate_url(app.source_url.trim()) {
                app.notice(error.to_string(), true);
                return Task::none();
            }
            app.data.sources.push(SourceConfig {
                id: uuid::Uuid::new_v4().to_string(),
                name: app.source_name.trim().into(),
                url: app.source_url.trim().into(),
                kind: app.source_kind.clone(),
                enabled: true,
            });
            app.source_name.clear();
            app.source_url.clear();
            return runtime::save_sources(app);
        }
        Message::ToggleSource(id) => {
            if let Some(source) = app.data.sources.iter_mut().find(|s| s.id == id) {
                source.enabled = !source.enabled;
            }
            return runtime::save_sources(app);
        }
        Message::RemoveSource(id) => {
            app.data.sources.retain(|s| s.id != id);
            return runtime::save_sources(app);
        }
        Message::Dismiss => {
            app.dialog = None;
            app.notification = None;
        }
        Message::OpenFolder => {
            if let Some(hub) = &app.hub
                && let Err(e) = runtime::open_data(hub)
            {
                app.notice(e.to_string(), true);
            }
        }
        Message::OpenProject => {
            if let Err(e) = open::that("https://github.com/moeleak/emulator-hub") {
                app.notice(e.to_string(), true);
            }
        }
        Message::Quit(id) => {
            let sessions: Vec<_> = app.sessions.values().cloned().collect();
            return Task::perform(
                async move {
                    for session in sessions {
                        let _ = session.stop().await;
                    }
                    id
                },
                |id| id,
            )
            .then(iced::window::close);
        }
    }
    Task::none()
}
