//! The app's headless runner (`nsh test`): its test files and the apps
//! they open, with `nsh/base` and the app's Rust components.

fn main() -> std::process::ExitCode {
    nsh_testing::runner(vec![nsh_base::registry(), set_app_ecclesia_app()], Some(app_root))
}

/// The UI set of the crate ecclesia-app: the entry `#[native_module("…")]` makes in it.
fn set_app_ecclesia_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_ecclesia_app::*;
    __nsh_native_module() // the crate ecclesia-app (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_ecclesia_app::App>()
        .app_id("com.ecclesiapdf.app")
        .app_info_json("{\"id\":\"com.ecclesiapdf.app\",\"name\":\"EcclesiaPDF\",\"version\":\"0.1.0\",\"build\":1,\"development\":true}")
        .translations_json(include_str!("C:\\dev\\EcclesiaPDF\\app\\.nsh/rust\\translations.json"))
        .capabilities_json("{\"network\":{\"hosts\":[],\"http\":[]},\"process\":{\"execute\":[],\"exit\":false,\"env\":[]},\"fs\":{\"read\":[],\"write\":[]},\"statusBar\":false,\"clipboard\":true,\"picker\":false,\"openUrl\":true,\"windows\":{\"kinds\":[\"normal\"]}}")
}

// The features the app's programs share (`.nsh/rust/hack`).
use workspace_hack as _;
