//! Render our own widget tree offscreen: no desktop capture or OS automation.
use super::*;
use iced::{
    Pixels, Rectangle,
    advanced::{Layout, layout, mouse, renderer, widget::Tree},
};

#[test]
#[ignore = "Generates visual QA PNGs in target/ui-previews without opening a window"]
fn render_pages() {
    {
        let mut fonts = iced_tiny_skia::graphics::text::font_system()
            .write()
            .unwrap();
        for font in material::fonts::all() {
            fonts.load_font(font);
        }
        fonts.load_font(std::borrow::Cow::Borrowed(include_bytes!(
            "../../assets/fonts/NotoSansSC-Core-0a7ff25a.otf"
        )));
        fonts.load_font(std::borrow::Cow::Borrowed(include_bytes!(
            "../../assets/fonts/NotoSansSC-faa6c9df.otf"
        )));
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-previews");
    std::fs::create_dir_all(&output).unwrap();
    for (name, language, appearance, size) in [
        (
            "zh-light",
            Language::Chinese,
            Appearance::Light,
            Size::new(1240.0, 820.0),
        ),
        (
            "zh-dark",
            Language::Chinese,
            Appearance::Dark,
            Size::new(1240.0, 820.0),
        ),
        (
            "en-light",
            Language::English,
            Appearance::Light,
            Size::new(1240.0, 820.0),
        ),
        (
            "en-narrow",
            Language::English,
            Appearance::Dark,
            Size::new(840.0, 620.0),
        ),
    ] {
        let (mut app, _) = boot();
        app.preferences.language = language;
        app.preferences.appearance = appearance;
        app.window_size = size;
        for (page, slug) in [
            (Page::Devices, "devices-empty"),
            (Page::Images, "images"),
            (Page::Downloads, "downloads"),
            (Page::Settings, "settings"),
        ] {
            app.navigation.select_now_for_size(page, size);
            draw(&app, &output.join(format!("{name}-{slug}.png")));
        }
        // Representative local fixtures for visual QA, never used in the application.
        let package = ImagePackage {
            id: "qa-lineage".into(),
            source_id: "lineageos-avd".into(),
            name: "LineageOS 23.2".into(),
            revision: "3".into(),
            api: hub_core::ApiVersion {
                major: 36,
                minor: 1,
            },
            abi: hub_core::Abi::Arm64V8a,
            url: "https://example.org/qa.zip".into(),
            size: 1_072_152_163,
            checksum: hub_core::Checksum {
                algorithm: hub_core::ChecksumAlgorithm::Sha256,
                value: "0".repeat(64),
            },
            license: String::new(),
            license_id: String::new(),
            min_engine_version: Some("36.1.0".into()),
            channel: "preview".into(),
        };
        let image = InstalledImage {
            key: "qa-image".into(),
            package: package.clone(),
            directory: PathBuf::new(),
        };
        app.data.images.push(image);
        app.catalog = vec![package];
        app.data.sources = SourceConfig::defaults();
        for (id, label) in [("qa-one", "LineageOS"), ("qa-two", "Android 工作空间")] {
            app.data.instances.push(Instance {
                id: id.into(),
                spec: InstanceSpec::new(label, "qa-image"),
                directory: PathBuf::new(),
                avd_name: id.into(),
                avd_home: PathBuf::new(),
                engine_version: None,
            });
        }
        app.navigate(Page::Devices);
        draw(&app, &output.join(format!("{name}-devices.png")));
        app.image_key = Some("qa-image".into());
        app.dialog = Some(Dialog::Create);
        draw(&app, &output.join(format!("{name}-create.png")));
        app.dialog = None;
        app.navigate(Page::Images);
        draw(&app, &output.join(format!("{name}-catalog.png")));
        app.selected = Some("qa-one".into());
        app.frame = Some((
            iced::widget::image::Handle::from_rgba(
                1080,
                1920,
                [24u8, 24, 24, 255].repeat(1080 * 1920),
            ),
            1080,
            1920,
        ));
        draw(&app, &output.join(format!("{name}-running.png")));
    }
    println!("Offscreen widget renders: {}", output.display());
}

fn draw(app: &App, path: &std::path::Path) {
    let size = app.window_size;
    let theme = theme(app).unwrap_or(material::Theme::Light);
    let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
        material::fonts::ROBOTO,
        Pixels(16.0),
    ));
    let mut view = views::view(app);
    let mut tree = Tree::new(view.as_widget());
    let node =
        view.as_widget_mut()
            .layout(&mut tree, &renderer, &layout::Limits::new(Size::ZERO, size));
    let bounds = Rectangle::with_size(size);
    let mut messages = Vec::new();
    let mut clipboard = iced::advanced::clipboard::Null;
    let started = Instant::now();
    for elapsed in [0, 500] {
        let mut shell = iced::advanced::Shell::new(&mut messages);
        view.as_widget_mut().update(
            &mut tree,
            &iced::Event::Window(iced::window::Event::RedrawRequested(
                started + Duration::from_millis(elapsed),
            )),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &renderer,
            &mut clipboard,
            &mut shell,
            &bounds,
        );
    }
    view.as_widget().draw(
        &tree,
        &mut renderer,
        &theme,
        &renderer::Style {
            text_color: theme.colors().surface.text,
        },
        Layout::new(&node),
        mouse::Cursor::Unavailable,
        &bounds,
    );
    let surface = Size::new(size.width as u32, size.height as u32);
    let mut pixels = tiny_skia::Pixmap::new(surface.width, surface.height).unwrap();
    let mut mask = tiny_skia::Mask::new(surface.width, surface.height).unwrap();
    if let iced::Renderer::Secondary(renderer) = &mut renderer {
        renderer.draw(
            &mut pixels.as_mut(),
            &mut mask,
            &iced_tiny_skia::graphics::Viewport::with_physical_size(surface, 1.0),
            &[bounds],
            theme.colors().surface.color,
        );
    } else {
        panic!("Expected offscreen software renderer");
    }
    assert!(
        pixels.pixels().iter().any(|p| p.alpha() != 0),
        "Empty render"
    );
    pixels.save_png(path).unwrap();
}
