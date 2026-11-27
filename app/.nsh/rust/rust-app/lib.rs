//! The Rust app's UI sets and root, on no platform: what the runner of a
//! platform implemented outside NSH starts (`docs/dev/platforms.md`).

/// The UI sets the app serves: `nsh/base`, then the app's crates'.
pub fn ui_sets() -> Vec<nsh_core::element::Registry> {
    vec![nsh_base::registry(), set_app_ecclesia_app()]
}

/// The app's crates' UI sets without `nsh/base`'s: what a platform that runs
/// no TSX serves (their files, such as fonts), so that its module links only
/// the base components the app's code uses.
pub fn app_ui_sets() -> Vec<nsh_core::element::Registry> {
    vec![set_app_ecclesia_app()]
}

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and status bar.
pub fn root() -> nsh_core::element::Root {
    app_root()
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
