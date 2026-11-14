//! The Rust app's `libnsh.so`: its root, which `AppActivity` runs, with
//! `nsh/base` and the app's components; no JS runtime.

#[cfg(target_os = "android")]
nsh::ui_sets!(vec![nsh_base::registry(), set_app_excellent_app()]);
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

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "android")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_excellent_app::App>()
        .app_id("com.excellentpdf.app")
        .translations_json(include_str!("C:\\dev\\ExcellentPDF\\app\\.nsh/rust\\translations.json"))
}
