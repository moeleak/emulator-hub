use super::*;
use crate::typography as text;
use iced::{
    Alignment, Length, border,
    widget::{Column, Row, Space, column, container, row, scrollable},
};
use material::{
    Element,
    widget::{
        button::{self, ButtonVariant as B, IconButtonVariant as I},
        card, dialog, progress_bar, select, text_input, toggler,
    },
};

fn icon<'a>(
    name: impl iced::widget::text::IntoFragment<'a>,
    size: u16,
) -> iced::widget::Text<'a, material::Theme> {
    material::fonts::icon(name, f32::from(size))
}

pub(super) fn view(app: &App) -> Element<'_, Message> {
    let page = if app.selected.is_some() {
        running(app)
    } else {
        match app.navigation.selected() {
            Page::Devices => devices(app),
            Page::Images => images(app),
            Page::Downloads => downloads(app),
            Page::Settings => settings(app),
        }
    };
    let content = column![
        row![
            text::label_large("EMULATOR HUB"),
            Space::new().width(Length::Fill),
            pill("science", "Preview 0.1")
        ]
        .align_y(Alignment::Center)
        .padding([0, 4]),
        page,
        notice(app),
    ]
    .spacing(16)
    .padding(24)
    .width(Length::Fill)
    .height(Length::Fill);
    let destinations = if app.preferences.language == Language::Chinese {
        &NAV_ZH
    } else {
        &NAV_EN
    };
    let base = navigation::suite(destinations, &app.navigation)
        .window_size(app.window_size)
        .with_menu("Emulator Hub", Message::Menu)
        .view(Message::Navigate, content);
    if let Some(current) = &app.dialog {
        dialog::modal(base, modal(app, current))
    } else {
        base
    }
}

fn action<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    variant: B,
    message: Message,
) -> Element<'a, Message> {
    button::button(label, variant).on_press(message).into()
}

fn icon_action(
    icon_name: &'static str,
    label: &'static str,
    message: Message,
) -> Element<'static, Message> {
    material::widget::tooltip::plain(
        button::icon_button(icon_name, I::Standard).on_press(message),
        label,
        iced::widget::tooltip::Position::Bottom,
    )
    .into()
}

fn pill<'a>(
    symbol: &'static str,
    label: impl iced::widget::text::IntoFragment<'a>,
) -> Element<'a, Message> {
    container(
        row![icon(symbol, 16), text::label_medium(label)]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .padding([6, 10])
    .style(|theme: &material::Theme| {
        let colors = theme.colors();
        iced::widget::container::Style {
            background: Some(colors.secondary.container.into()),
            text_color: Some(colors.secondary.container_text),
            border: border::rounded(999),
            ..Default::default()
        }
    })
    .into()
}

fn header<'a>(
    title: &'static str,
    subtitle: &'static str,
    actions: Element<'a, Message>,
) -> Element<'a, Message> {
    row![
        column![text::headline_large(title), text::body_medium(subtitle)].spacing(8),
        Space::new().width(Length::Fill),
        actions
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

fn pane<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding(24)
        .style(|theme: &material::Theme| iced::widget::container::Style {
            background: Some(theme.colors().surface.container.lowest.into()),
            border: border::rounded(material::tokens::shape::CORNER_EXTRA_LARGE),
            ..Default::default()
        })
        .into()
}

fn notice(app: &App) -> Element<'_, Message> {
    if let Some((message, is_error)) = &app.notification {
        let is_error = *is_error;
        container(
            row![
                icon(if is_error { "error" } else { "check_circle" }, 22),
                text::body_medium(message).width(Length::Fill),
                icon_action("close", app.t("关闭", "Dismiss"), Message::Dismiss)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([8, 16])
        .style(move |theme: &material::Theme| {
            let colors = theme.colors();
            iced::widget::container::Style {
                background: Some(
                    if is_error {
                        colors.error.container
                    } else {
                        colors.inverse.inverse_surface
                    }
                    .into(),
                ),
                text_color: Some(if is_error {
                    colors.error.container_text
                } else {
                    colors.inverse.inverse_surface_text
                }),
                border: border::rounded(16),
                ..Default::default()
            }
        })
        .into()
    } else {
        Space::new().height(0).into()
    }
}

fn devices(app: &App) -> Element<'_, Message> {
    let heading = header(
        app.t("你的设备", "Your devices"),
        app.t(
            "在桌面上，打开你的 Android。",
            "Your Android, at home on your desktop.",
        ),
        action(
            app.t("创建设备", "Create device"),
            B::Filled,
            Message::NewInstance,
        ),
    );
    let mut body = column![heading].spacing(24);
    if app.data.instances.is_empty() {
        let welcome = column![
            container(icon("devices_other", 88))
                .padding(28)
                .style(|theme: &material::Theme| {
                    iced::widget::container::Style {
                        background: Some(theme.colors().primary.container.into()),
                        text_color: Some(theme.colors().primary.container_text),
                        border: border::rounded(40),
                        ..Default::default()
                    }
                }),
            text::headline_medium(app.t(
                "一个桌面，无限可能",
                "A little Android. A lot of possibilities."
            )),
            text::body_large(app.t(
                "选择 LineageOS 或 Google 系统镜像，\n创建属于你的第一台 Android 设备。",
                "Choose a LineageOS or Google system image\nand make your first Android device."
            ))
            .center(),
            row![
                action(
                    app.t("浏览镜像库", "Explore images"),
                    B::Filled,
                    Message::Navigate(Page::Images)
                ),
                action(
                    app.t("导入本地镜像", "Import image"),
                    B::Outlined,
                    Message::Import
                )
            ]
            .spacing(12),
        ]
        .spacing(24)
        .align_x(Alignment::Center);
        body = body.push(
            container(welcome)
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        );
    } else {
        let columns = if app.window_size.width >= 1450.0 {
            3
        } else {
            2
        };
        let mut cards = Column::new().spacing(16);
        for chunk in app.data.instances.chunks(columns) {
            let mut cards_row = Row::new().spacing(16);
            for instance in chunk {
                cards_row = cards_row.push(device_card(app, instance));
            }
            for _ in chunk.len()..columns {
                cards_row = cards_row.push(Space::new().width(Length::Fill));
            }
            cards = cards.push(cards_row);
        }
        body = body.push(scrollable(cards).height(Length::Fill));
    }
    body = body.push(
        row![
            pill(
                "devices",
                format!(
                    "{} {}",
                    app.data.instances.len(),
                    app.t("台设备", "devices")
                )
            ),
            pill(
                "play_circle",
                format!("{} {}", app.sessions.len(), app.t("正在运行", "running"))
            ),
            Space::new().width(Length::Fill),
            text::body_small(app.t("由 Google Emulator 驱动", "Powered by Google Emulator"))
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    );
    body.height(Length::Fill).into()
}

fn device_card<'a>(app: &'a App, instance: &'a Instance) -> Element<'a, Message> {
    let running = app.sessions.contains_key(&instance.id);
    let launching = app.launching.contains(&instance.id);
    let image = app
        .data
        .images
        .iter()
        .find(|i| i.key == instance.spec.image_key);
    let content = column![
        row![
            container(icon("phone_android", 36))
                .padding(12)
                .style(|theme: &material::Theme| {
                    iced::widget::container::Style {
                        background: Some(theme.colors().primary.container.into()),
                        text_color: Some(theme.colors().primary.container_text),
                        border: border::rounded(20),
                        ..Default::default()
                    }
                }),
            Space::new().width(Length::Fill),
            pill(
                if running {
                    "play_circle"
                } else {
                    "power_settings_new"
                },
                if running {
                    app.t("运行中", "Running")
                } else if launching {
                    app.t("启动中", "Starting")
                } else {
                    app.t("已关闭", "Stopped")
                }
            )
        ]
        .align_y(Alignment::Center),
        column![
            text::title_large(&instance.spec.name),
            text::body_medium(
                image
                    .map(|i| format!(
                        "{} · API {} · {}",
                        i.package.name, i.package.api, i.package.abi
                    ))
                    .unwrap_or_else(|| app.t("镜像不可用", "Image unavailable").into())
            )
        ]
        .spacing(6),
        row![
            pill("memory", format!("{} GB", instance.spec.memory_mb / 1024)),
            pill(
                "developer_board",
                format!("{} CPU", instance.spec.cpu_cores)
            ),
            text::label_medium(format!(
                "{} × {}",
                instance.spec.width, instance.spec.height
            ))
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            button::button(
                if running {
                    app.t("打开", "Open")
                } else {
                    app.t("启动", "Start")
                },
                B::Filled
            )
            .on_press_maybe((!launching).then(|| if running {
                Message::OpenInstance(instance.id.clone())
            } else {
                Message::Launch(instance.id.clone())
            })),
            Space::new().width(Length::Fill),
            icon_action(
                "edit",
                app.t("重命名", "Rename"),
                Message::Rename(instance.id.clone())
            ),
            icon_action(
                if running { "stop_circle" } else { "delete" },
                if running {
                    app.t("关闭设备", "Stop device")
                } else {
                    app.t("删除设备", "Delete device")
                },
                if running {
                    Message::Stop(instance.id.clone())
                } else {
                    Message::Delete(instance.id.clone())
                }
            )
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(20);
    card::filled(content).padding(24).width(Length::Fill).into()
}

fn images(app: &App) -> Element<'_, Message> {
    let heading = header(
        app.t("镜像库", "Image library"),
        app.t(
            "选择你喜欢的 Android 系统。",
            "Find the Android that fits you.",
        ),
        row![
            action(app.t("导入", "Import"), B::Outlined, Message::Import),
            button::button(app.t("刷新", "Refresh"), B::FilledTonal)
                .on_press_maybe((!app.refreshing).then_some(Message::Refresh))
        ]
        .spacing(12)
        .into(),
    );
    let filters = row![
        text_input::outlined(
            app.t("搜索系统、版本或来源", "Search images, versions or sources"),
            &app.search
        )
        .on_input(Message::Search)
        .width(Length::Fill),
        toggler::standard(
            app.compatible_only,
            app.t("兼容此电脑", "Compatible"),
            Message::CompatibleOnly
        )
    ]
    .spacing(20)
    .align_y(Alignment::Center);
    let query = app.search.to_lowercase();
    let mut packages: Vec<_> = app
        .catalog
        .iter()
        .filter(|i| {
            (!app.compatible_only || i.abi.compatible_with_host())
                && format!("{} {} {} {}", i.name, i.source_id, i.api, i.abi)
                    .to_lowercase()
                    .contains(&query)
        })
        .collect();
    packages.sort_by_key(|package| package.source_id != "lineageos-avd");
    let mut entries = Column::new().spacing(12);
    for package in packages {
        let installed = app.data.images.iter().find(|i| {
            i.package.source_id == package.source_id
                && i.package.id == package.id
                && i.package.revision == package.revision
                && i.package.checksum == package.checksum
        });
        let download_key = format!("{}:{}:{}", package.source_id, package.id, package.revision);
        let downloading = app
            .downloads
            .get(&download_key)
            .is_some_and(|d| !d.finished);
        let action = if let Some(installed) = installed {
            action(
                app.t("创建设备", "Create device"),
                B::FilledTonal,
                Message::NewFromImage(installed.key.clone()),
            )
        } else {
            button::button(
                if downloading {
                    app.t("下载中", "Downloading")
                } else {
                    app.t("下载", "Download")
                },
                B::Outlined,
            )
            .on_press_maybe(
                (!downloading && package.abi.compatible_with_host())
                    .then(|| Message::Download(package.clone())),
            )
            .into()
        };
        entries = entries.push(
            card::filled(
                row![
                    icon("android", 32),
                    column![
                        text::title_medium(&package.name),
                        text::body_small(format!(
                            "{} · API {} · {} · {}",
                            package.source_id,
                            package.api,
                            package.abi,
                            bytes(package.size)
                        ))
                    ]
                    .spacing(6)
                    .width(Length::Fill),
                    action
                ]
                .spacing(20)
                .align_y(Alignment::Center),
            )
            .padding(20),
        );
    }
    if app.catalog.is_empty() && !app.refreshing {
        entries = entries.push(pane(
            column![
                text::title_large(app.t("还没有可用镜像", "No images available yet")),
                text::body_medium(app.t(
                    "刷新镜像列表，或在设置中添加镜像源。你也可以导入本地 ZIP。",
                    "Refresh the catalog, add a source in Settings, or import a local ZIP."
                ))
            ]
            .spacing(12),
        ));
    }
    if !app.data.images.is_empty() {
        entries = entries.push(text::title_large(app.t("已安装", "Installed")));
        for image in &app.data.images {
            entries = entries.push(
                card::outlined(
                    row![
                        icon("offline_pin", 28),
                        column![
                            text::title_medium(&image.package.name),
                            text::body_small(format!(
                                "{} · API {} · {}",
                                image.package.source_id, image.package.api, image.package.abi
                            ))
                        ]
                        .spacing(4)
                        .width(Length::Fill),
                        action(
                            app.t("创建设备", "Create device"),
                            B::FilledTonal,
                            Message::NewFromImage(image.key.clone())
                        )
                    ]
                    .spacing(16)
                    .align_y(Alignment::Center),
                )
                .padding(20),
            );
        }
    }
    for error in &app.catalog_errors {
        entries = entries.push(text::body_small(error));
    }
    let mut body = column![heading, filters].spacing(24);
    if app.refreshing {
        body = body.push(progress_bar::linear(
            progress_bar::LinearProgressMode::indeterminate(app.phase),
        ));
    }
    body.push(scrollable(entries).height(Length::Fill))
        .height(Length::Fill)
        .into()
}

fn downloads(app: &App) -> Element<'_, Message> {
    let mut entries = Column::new().spacing(16);
    if app.downloads.is_empty() {
        entries = entries.push(pane(
            column![
                icon("download_done", 48),
                text::headline_small(app.t("下载队列是空的", "All quiet here")),
                text::body_large(app.t(
                    "从镜像库选择一个系统，下载进度会显示在这里。",
                    "Choose an image from the library to start a download."
                )),
                action(
                    app.t("浏览镜像", "Explore images"),
                    B::FilledTonal,
                    Message::Navigate(Page::Images)
                )
            ]
            .spacing(20),
        ));
    }
    let mut keys: Vec<_> = app.downloads.keys().collect();
    keys.sort();
    for key in keys {
        let item = &app.downloads[key];
        let progress = if item.total > 0 {
            item.downloaded as f32 / item.total as f32
        } else {
            0.0
        };
        let mut content = column![
            row![
                icon(
                    if item.finished {
                        "task_alt"
                    } else {
                        "download"
                    },
                    28
                ),
                text::title_medium(&item.name).width(Length::Fill),
                text::label_large(format!("{:.0}%", progress * 100.0))
            ]
            .spacing(12)
            .align_y(Alignment::Center),
            progress_bar::linear(progress_bar::LinearProgressMode::determinate(
                progress, app.phase
            )),
            row![
                text::body_small(format!(
                    "{} / {}",
                    bytes(item.downloaded),
                    bytes(item.total)
                )),
                Space::new().width(Length::Fill),
                text::body_small(if item.paused {
                    app.t("已暂停", "Paused").into()
                } else {
                    item.status.clone()
                })
            ]
            .spacing(12)
        ]
        .spacing(16);
        if !item.finished {
            content = content.push(
                row![
                    action(
                        if item.paused {
                            app.t("继续", "Resume")
                        } else {
                            app.t("暂停", "Pause")
                        },
                        B::FilledTonal,
                        Message::PauseDownload(key.clone())
                    ),
                    action(
                        app.t("取消", "Cancel"),
                        B::Text,
                        Message::CancelDownload(key.clone())
                    )
                ]
                .spacing(12),
            );
        }
        entries = entries.push(card::filled(content).padding(24));
    }
    column![
        header(
            app.t("下载", "Downloads"),
            app.t(
                "下载、校验并安装你的系统镜像。",
                "Download, verify and install your images."
            ),
            Space::new().into()
        ),
        scrollable(entries).height(Length::Fill)
    ]
    .spacing(24)
    .height(Length::Fill)
    .into()
}

fn settings(app: &App) -> Element<'_, Message> {
    let theme_buttons = row![
        action(
            app.t("跟随系统", "System"),
            if app.preferences.appearance == Appearance::System {
                B::Filled
            } else {
                B::Outlined
            },
            Message::Appearance(Appearance::System)
        ),
        action(
            app.t("浅色", "Light"),
            if app.preferences.appearance == Appearance::Light {
                B::Filled
            } else {
                B::Outlined
            },
            Message::Appearance(Appearance::Light)
        ),
        action(
            app.t("深色", "Dark"),
            if app.preferences.appearance == Appearance::Dark {
                B::Filled
            } else {
                B::Outlined
            },
            Message::Appearance(Appearance::Dark)
        ),
    ]
    .spacing(8);
    let appearance = pane(
        column![
            text::title_large(app.t("外观", "Appearance")),
            theme_buttons,
            row![
                action(
                    "简体中文",
                    if app.preferences.language == Language::Chinese {
                        B::FilledTonal
                    } else {
                        B::Outlined
                    },
                    Message::Language(Language::Chinese)
                ),
                action(
                    "English",
                    if app.preferences.language == Language::English {
                        B::FilledTonal
                    } else {
                        B::Outlined
                    },
                    Message::Language(Language::English)
                )
            ]
            .spacing(8)
        ]
        .spacing(20),
    );
    let engine = pane(column![
        row![icon("memory", 28), text::title_large(app.t("模拟器引擎", "Emulator engine")), Space::new().width(Length::Fill), button::button(if app.busy { app.t("安装中…", "Installing…") } else { app.t("安装自编引擎", "Install engine") }, B::Filled).on_press_maybe((!app.busy).then_some(Message::InstallEngine))].spacing(12).align_y(Alignment::Center),
        text::body_medium(app.t("自动下载 LineageOS AVD 发布的源码构建引擎。也可以选择现有的 Emulator 和 ADB。", "Download the source-built engine from LineageOS AVD, or select an existing Emulator and ADB.")),
        button::button(app.t("使用 Google 官方引擎…", "Use Google official tools…"), B::Text).on_press_maybe((!app.busy).then_some(Message::InstallOfficialEngine)),
        row![column![text::label_large("Emulator"), text::body_small(display_path(&app.preferences.emulator, app.t("尚未配置", "Not configured")))].spacing(4).width(Length::Fill), action(app.t("选择文件", "Choose file"), B::Outlined, Message::PickEngine)].spacing(16).align_y(Alignment::Center),
        row![column![text::label_large("ADB"), text::body_small(display_path(&app.preferences.adb, app.t("尚未配置", "Not configured")))].spacing(4).width(Length::Fill), action(app.t("选择文件", "Choose file"), B::Outlined, Message::PickAdb)].spacing(16).align_y(Alignment::Center),
        toggler::standard(app.preferences.audio, app.t("播放设备声音（下次启动时生效）", "Play audio on next device start"), Message::Audio),
        text::label_large(app.t("图形渲染（下次启动时生效）", "Graphics on next device start")),
        row![
            action(app.t("自动", "Automatic"), if app.preferences.graphics == Graphics::Auto { B::FilledTonal } else { B::Outlined }, Message::Graphics(Graphics::Auto)),
            action(app.t("硬件", "Hardware"), if app.preferences.graphics == Graphics::Host { B::FilledTonal } else { B::Outlined }, Message::Graphics(Graphics::Host)),
            action(app.t("软件", "Software"), if app.preferences.graphics == Graphics::Software { B::FilledTonal } else { B::Outlined }, Message::Graphics(Graphics::Software)),
        ].spacing(8),
        text::body_small(app.t("遇到图形驱动问题时可尝试软件渲染。", "Try software rendering if a graphics driver prevents startup.")),
    ].spacing(20));
    let mut sources = column![
        text::title_large(app.t("镜像源", "Image sources")),
        text::body_medium(app.t(
            "启用多个来源，或添加你自己的镜像目录。",
            "Use multiple sources or add your own image catalog."
        ))
    ]
    .spacing(16);
    for source in &app.data.sources {
        let id = source.id.clone();
        sources = sources.push(
            row![
                toggler::control(source.enabled)
                    .on_toggle(move |_| Message::ToggleSource(id.clone())),
                column![
                    text::title_medium(&source.name),
                    text::body_small(&source.url)
                ]
                .spacing(4)
                .width(Length::Fill),
                icon_action(
                    "delete",
                    app.t("移除源", "Remove source"),
                    Message::RemoveSource(source.id.clone())
                )
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        );
    }
    sources = sources
        .push(material::widget::rule::horizontal_full_width())
        .push(
            row![
                text_input::outlined(app.t("名称", "Name"), &app.source_name)
                    .on_input(Message::SourceName),
                text_input::outlined("https://…", &app.source_url)
                    .on_input(Message::SourceUrl)
                    .width(Length::Fill)
            ]
            .spacing(12),
        );
    sources = sources.push(
        row![
            action(
                "Hub JSON",
                if app.source_kind == SourceKind::HubJson {
                    B::FilledTonal
                } else {
                    B::Outlined
                },
                Message::SourceKind(SourceKind::HubJson)
            ),
            action(
                "SDK XML",
                if app.source_kind == SourceKind::SdkXml {
                    B::FilledTonal
                } else {
                    B::Outlined
                },
                Message::SourceKind(SourceKind::SdkXml)
            ),
            Space::new().width(Length::Fill),
            action(app.t("添加源", "Add source"), B::Filled, Message::AddSource)
        ]
        .spacing(8),
    );
    let body = column![
        appearance,
        engine,
        pane(sources),
        pane(
            row![
                text::body_medium("Emulator Hub 0.1.0 Preview").width(Length::Fill),
                action(
                    app.t("数据目录", "Data folder"),
                    B::Text,
                    Message::OpenFolder
                ),
                action("GitHub", B::Text, Message::OpenProject)
            ]
            .spacing(12)
            .align_y(Alignment::Center)
        )
    ]
    .spacing(20);
    column![
        header(
            app.t("设置", "Settings"),
            app.t("让工作空间适合你。", "Make this space yours."),
            Space::new().into()
        ),
        scrollable(body).height(Length::Fill)
    ]
    .spacing(24)
    .height(Length::Fill)
    .into()
}

fn running(app: &App) -> Element<'_, Message> {
    let id = app.selected.as_deref().unwrap_or_default();
    let instance = app.data.instances.iter().find(|i| i.id == id);
    let name = instance.map(|i| i.spec.name.as_str()).unwrap_or("Android");
    let toolbar = row![
        icon_action(
            "arrow_back",
            app.t("设备列表", "Devices"),
            Message::BackToDevices
        ),
        text::title_large(name),
        Space::new().width(Length::Fill),
        action(
            app.t("安装 APK", "Install APK"),
            B::FilledTonal,
            Message::InstallApk(id.into())
        ),
        icon_action(
            "stop_circle",
            app.t("关闭", "Stop"),
            Message::Stop(id.into())
        )
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let display: Element<'_, Message> = if let Some((handle, width, height)) = &app.frame {
        screen::view(handle.clone(), *width, *height, Message::ScreenInput)
    } else {
        container(
            column![
                icon("android", 64),
                text::title_large(if app.launching.iter().any(|i| i == id) {
                    app.t("正在启动 Android…", "Starting Android…")
                } else {
                    app.t("等待设备画面", "Waiting for display")
                }),
                text::body_medium(app.t(
                    "首次启动可能需要几分钟。屏幕休眠时可用右侧电源键唤醒。",
                    "The first boot may take a few minutes. Use Power to wake a sleeping display."
                ))
            ]
            .spacing(20)
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    };
    let controls = column![
        icon_action(
            "power_settings_new",
            app.t("电源 / 唤醒", "Power / wake"),
            Message::AndroidKey("Power")
        ),
        icon_action(
            "arrow_back",
            app.t("返回", "Back"),
            Message::AndroidKey("GoBack")
        ),
        icon_action("home", app.t("主页", "Home"), Message::AndroidKey("GoHome")),
        icon_action(
            "recent_actors",
            app.t("最近任务", "Recent apps"),
            Message::AndroidKey("AppSwitch")
        ),
        material::widget::rule::horizontal_full_width(),
        icon_action(
            "volume_up",
            app.t("提高音量", "Volume up"),
            Message::AndroidKey("AudioVolumeUp")
        ),
        icon_action(
            "volume_down",
            app.t("降低音量", "Volume down"),
            Message::AndroidKey("AudioVolumeDown")
        ),
        icon_action("screenshot", app.t("截图", "Screenshot"), Message::Capture),
        icon_action(
            "save",
            app.t("保存快照", "Save snapshot"),
            Message::Snapshot(true)
        ),
        icon_action(
            "restore",
            app.t("恢复快照", "Restore snapshot"),
            Message::Snapshot(false)
        ),
        material::widget::rule::horizontal_full_width(),
        icon_action(
            "content_paste",
            app.t("粘贴到设备", "Paste to device"),
            Message::ClipboardToDevice
        ),
        icon_action(
            "content_copy",
            app.t("复制设备剪贴板", "Copy device clipboard"),
            Message::ClipboardFromDevice
        ),
    ]
    .spacing(8)
    .align_x(Alignment::Center)
    .width(56);
    column![
        toolbar,
        row![display, scrollable(controls).width(64).height(Length::Fill)].spacing(16).height(Length::Fill),
        text::body_small(app.t(
            "点击画面后可使用键盘和鼠标。切换页面或窗口会释放按键。",
            "Click the display for keyboard and mouse input. Switching pages or windows releases held input."
        ))
    ]
    .spacing(16)
    .height(Length::Fill)
    .into()
}

fn modal<'a>(app: &'a App, current: &'a Dialog) -> Element<'a, Message> {
    let cancel = action(app.t("取消", "Cancel"), B::Text, Message::Dismiss);
    match current {
        Dialog::Create => {
            let options: Vec<ImageChoice> = app.data.images.iter().map(ImageChoice::from).collect();
            let selected = app
                .image_key
                .as_ref()
                .and_then(|key| app.data.images.iter().find(|i| &i.key == key))
                .map(ImageChoice::from);
            let body = column![
                text_input::outlined(app.t("设备名称", "Device name"), &app.name)
                    .on_input(Message::InstanceName),
                select::outlined(options, selected, Message::InstanceImage)
                    .label(app.t("系统镜像", "System image"))
                    .width(Length::Fill),
                row![
                    text_input::outlined(app.t("内存 (MB)", "Memory (MB)"), &app.memory)
                        .on_input(Message::Memory),
                    text_input::outlined(app.t("CPU 核心", "CPU cores"), &app.cpus)
                        .on_input(Message::Cpus)
                ]
                .spacing(12),
                row![
                    text_input::outlined(app.t("宽度", "Width"), &app.width)
                        .on_input(Message::Width),
                    text_input::outlined(app.t("高度", "Height"), &app.height)
                        .on_input(Message::Height)
                ]
                .spacing(12),
            ]
            .spacing(20);
            dialog::content(
                app.t("创建设备", "Create device"),
                body,
                row![
                    cancel,
                    button::button(app.t("创建", "Create"), B::Filled)
                        .on_press_maybe((!app.busy).then_some(Message::Create))
                ]
                .spacing(12),
            )
            .into()
        }
        Dialog::License(package) => {
            let license = if package.license.is_empty() {
                app.t("此镜像由所选来源发布。下载后会验证完整性，并保留包中的许可证文件。", "This image is published by the selected source. Its integrity will be verified and included licenses preserved.")
            } else {
                &package.license
            };
            dialog::content(
                app.t("下载系统镜像", "Download system image"),
                column![
                    text::title_medium(&package.name),
                    text::body_small(format!(
                        "{} · {} · API {}",
                        bytes(package.size),
                        package.abi,
                        package.api
                    )),
                    scrollable(text::body_small(license)).height(240)
                ]
                .spacing(16),
                row![
                    cancel,
                    action(
                        app.t("同意并下载", "Accept and download"),
                        B::Filled,
                        Message::ConfirmDownload
                    )
                ]
                .spacing(12),
            )
            .into()
        }
        Dialog::Tools(tools) => {
            let names = tools
                .iter()
                .map(|p| format!("{} {} · {}", p.name, p.version, bytes(p.size)))
                .collect::<Vec<_>>()
                .join("\n");
            let licenses = tools
                .iter()
                .filter(|p| !p.license.is_empty())
                .map(|p| p.license.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            dialog::content(
                app.t("安装模拟器工具", "Install emulator tools"),
                column![
                    text::body_medium(names),
                    scrollable(text::body_small(licenses)).height(240)
                ]
                .spacing(16),
                row![
                    cancel,
                    action(
                        app.t("同意并安装", "Accept and install"),
                        B::Filled,
                        Message::ConfirmTools
                    )
                ]
                .spacing(12),
            )
            .into()
        }
        Dialog::Rename(_) => dialog::content(
            app.t("重命名设备", "Rename device"),
            text_input::outlined(app.t("名称", "Name"), &app.name).on_input(Message::RenameValue),
            row![
                cancel,
                action(app.t("保存", "Save"), B::Filled, Message::ConfirmRename)
            ]
            .spacing(12),
        )
        .into(),
        Dialog::Delete(id) => {
            let name = app
                .data
                .instances
                .iter()
                .find(|i| &i.id == id)
                .map(|i| i.spec.name.as_str())
                .unwrap_or(id);
            dialog::content(app.t("删除设备？", "Delete device?"), column![text::title_medium(name), text::body_medium(app.t("此设备的应用、用户数据和快照将被删除。已下载的系统镜像会保留。", "This removes the device's apps, user data and snapshots. Downloaded system images are retained."))].spacing(12), row![cancel, action(app.t("删除", "Delete"), B::Filled, Message::ConfirmDelete)].spacing(12)).into()
        }
    }
}

fn display_path(path: &std::path::Path, empty: &str) -> String {
    if path.as_os_str().is_empty() {
        empty.into()
    } else {
        path.display().to_string()
    }
}
fn bytes(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.2} GB", value as f64 / 1_000_000_000.0)
    } else {
        format!("{:.1} MB", value as f64 / 1_000_000.0)
    }
}
