//! The Rust app's executable: its root with `nsh/base` and the app's
//! components, on nsh_macos_app's runner; no JS runtime.

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    {
        nsh_platform_macos::set_ui_sets(vec![nsh_base::registry_of([nsh_base::parts::core as nsh_base::parts::Part]), set_app_excellent_app()]);
        nsh_platform_macos::set_components(app_components);
        nsh_macos_app::run_rust(app_root())
    }
    #[cfg(not(target_os = "macos"))]
    nsh_macos_app::main()
}

/// The UI set of the crate excellent-app: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "macos")]
fn set_app_excellent_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_excellent_app::*;
    __nsh_native_module() // the crate excellent-app (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's native components on this platform: what `#[native_components(macos)]` registers in its crates.
#[cfg(target_os = "macos")]
fn app_components(__nsh_components: &nsh_platform_macos::Components) {
    {
        #[allow(unused_imports)]
        use app_excellent_app::*;
        __nsh_native_components_macos(__nsh_components); // the crate excellent-app (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_macos;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "macos")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_excellent_app::App>()
        .app_id("com.excellentpdf.app")
        .app_info_json("{\"id\":\"com.excellentpdf.app\",\"name\":\"ExcellentPDF\",\"version\":\"0.1.0\",\"build\":1,\"development\":true}")
        .translations_json(include_str!("C:\\dev\\ExcellentPDF\\app\\.nsh/rust\\translations.json"))
        .capabilities_json("{\"network\":{\"hosts\":[],\"http\":[]},\"process\":{\"execute\":[],\"exit\":false,\"env\":[]},\"fs\":{\"read\":[],\"write\":[]},\"statusBar\":false,\"clipboard\":true,\"picker\":false,\"openUrl\":true,\"windows\":{\"kinds\":[\"normal\"]}}")
}
