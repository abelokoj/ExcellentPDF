//! ExcellentPDF on the machine's native backend (`cargo run -p excellent-app`).

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();
    log::debug!("app: main started");
    // A second launch hands its files to the running app and ends, before anything else loads (R.11).
    if excellent_app::start() == excellent_app::Started::Forwarded {
        log::debug!("app: the running instance took the command line");
        std::process::exit(0);
    }
    excellent_app::load_settings();
    excellent_app::open_library();
    let components = platform::Components::default();
    #[cfg(target_os = "windows")]
    excellent_app::components(&components);
    platform::run_app(
        platform::AppSource::Rust(excellent_app::root()),
        platform::AppOptions {
            title: "ExcellentPDF".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: excellent_app::APP_ID.to_owned(),
            size: (1280, 800),
            components,
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!(
    "The desktop binary runs on Windows, macOS and Linux; tablets get their own shells (plan D10)."
);
