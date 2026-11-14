//! The Rust app's web runtime: its root, which `nsh.js` runs, with the
//! `nsh/base` and the app's components; no QuickJS. `nsh/base` is its
//! core part: the app makes its components by type, which links
//! only those it uses.

#[cfg(target_arch = "wasm32")]
nsh_platform_web::rust_app!(vec![nsh_base::registry_of([nsh_base::parts::core as nsh_base::parts::Part]), set_app_excellent_app()], app_root());

// Off WebAssembly it is empty, as nsh-platform-web is.
#[cfg(not(target_arch = "wasm32"))]
use nsh_core as _;
#[cfg(not(target_arch = "wasm32"))]
use nsh_base as _;

/// The UI set of the crate excellent-app: the entry `#[native_module("…")]` makes in it.
#[cfg(target_arch = "wasm32")]
fn set_app_excellent_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_excellent_app::*;
    __nsh_native_module() // the crate excellent-app (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_arch = "wasm32")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_arch = "wasm32")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_excellent_app::App>()
        .app_id("com.excellentpdf.app")
        .translations_json(include_str!("C:\\dev\\ExcellentPDF\\app\\.nsh/rust\\translations.json"))
}
