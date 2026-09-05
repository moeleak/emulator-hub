#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod i18n;
mod preferences;
mod screen;
mod typography;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    app::run()
}
