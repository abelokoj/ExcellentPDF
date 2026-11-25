//! The Windows development host with `nsh/base` and the app's Rust components.

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "windows")]
    nsh_platform_windows::set_ui_sets(vec![nsh_base::registry(), set_app_excellent_app()]);
    #[cfg(target_os = "windows")]
    nsh_platform_windows::set_components(app_components);
    #[cfg(target_os = "windows")]
    nsh_platform_windows::set_root(app_root);
    nsh_windows_host::main()
}

/// The UI set of the crate excellent-app: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "windows")]
fn set_app_excellent_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_excellent_app::*;
    __nsh_native_module() // the crate excellent-app (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's native components on this platform: what `#[native_components(windows)]` registers in its crates.
#[cfg(target_os = "windows")]
fn app_components(__nsh_components: &nsh_platform_windows::Components) {
    {
        #[allow(unused_imports)]
        use app_excellent_app::*;
        __nsh_native_components_windows(__nsh_components); // the crate excellent-app (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_windows;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "windows")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_excellent_app::App>()
        .app_id("com.excellentpdf.app")
        .app_info_json("{\"id\":\"com.excellentpdf.app\",\"name\":\"ExcellentPDF\",\"version\":\"0.1.0\",\"build\":1,\"development\":true}")
        .translations_json(include_str!("C:\\dev\\ExcellentPDF\\app\\.nsh/rust\\translations.json"))
        .capabilities_json("{\"network\":{\"hosts\":[],\"http\":[]},\"process\":{\"execute\":[],\"exit\":false,\"env\":[]},\"fs\":{\"read\":[],\"write\":[]},\"statusBar\":false,\"clipboard\":true,\"picker\":false,\"openUrl\":true,\"windows\":{\"kinds\":[\"normal\"]}}")
}

// The features the app's programs share (`.nsh/rust/hack`).
use workspace_hack as _;
