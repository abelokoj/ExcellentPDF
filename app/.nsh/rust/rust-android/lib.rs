//! The Rust app's `libnsh.so`: its root, which `AppActivity` runs, with
//! `nsh/base` and the app's components; no JS runtime.

#[cfg(target_os = "android")]
nsh::ui_sets!(vec![nsh_base::registry(), set_app_excellent_app()]);
#[cfg(target_os = "android")]
nsh::components!(app_components);
#[cfg(target_os = "android")]
nsh::main!(app_root());

// Off Android it is empty, as nsh-platform-android is.
#[cfg(not(target_os = "android"))]
use nsh_core as _;
#[cfg(not(target_os = "android"))]
use nsh_base as _;

/// The UI set of the crate excellent-app: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "android")]
fn set_app_excellent_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_excellent_app::*;
    __nsh_native_module() // the crate excellent-app (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "android")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's native components on this platform: what `#[native_components(android)]` registers in its crates.
#[cfg(target_os = "android")]
fn app_components(__nsh_components: &nsh::Components) {
    {
        #[allow(unused_imports)]
        use app_excellent_app::*;
        __nsh_native_components_android(__nsh_components); // the crate excellent-app (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "android")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_android;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "android")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_excellent_app::App>()
        .app_id("com.excellentpdf.app")
        .app_info_json("{\"id\":\"com.excellentpdf.app\",\"name\":\"ExcellentPDF\",\"version\":\"0.1.0\",\"build\":1,\"development\":true}")
        .translations_json(include_str!("C:\\dev\\ExcellentPDF\\app\\.nsh/rust\\translations.json"))
        .capabilities_json("{\"network\":{\"hosts\":[],\"http\":[]},\"process\":{\"execute\":[],\"exit\":false,\"env\":[]},\"fs\":{\"read\":[],\"write\":[]},\"statusBar\":false,\"clipboard\":true,\"picker\":false,\"openUrl\":true,\"windows\":{\"kinds\":[\"normal\"]}}")
}
