//! The app's `libnsh.so`: the Android runtime with `nsh/base` and the
//! app's Rust components.

#[cfg(target_os = "android")]
nsh::ui_sets!(vec![nsh_base::registry(), set_app_ecclesia_app()]);
#[cfg(target_os = "android")]
nsh::components!(app_components);
#[cfg(target_os = "android")]
nsh::root!(app_root());

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "android")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_ecclesia_app::App>()
        .app_id("com.ecclesiapdf.app")
        .app_info_json("{\"id\":\"com.ecclesiapdf.app\",\"name\":\"EcclesiaPDF\",\"version\":\"0.1.0\",\"build\":1,\"development\":true}")
        .translations_json(include_str!("C:\\dev\\EcclesiaPDF\\app\\.nsh/rust\\translations.json"))
        .capabilities_json("{\"network\":{\"hosts\":[],\"http\":[]},\"process\":{\"execute\":[],\"exit\":false,\"env\":[]},\"fs\":{\"read\":[],\"write\":[]},\"statusBar\":false,\"clipboard\":true,\"picker\":false,\"openUrl\":true,\"windows\":{\"kinds\":[\"normal\"]}}")
}

/// The UI set of the crate ecclesia-app: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "android")]
fn set_app_ecclesia_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_ecclesia_app::*;
    __nsh_native_module() // the crate ecclesia-app (nsh.json "rust")
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
        use app_ecclesia_app::*;
        __nsh_native_components_android(__nsh_components); // the crate ecclesia-app (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "android")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_android;
