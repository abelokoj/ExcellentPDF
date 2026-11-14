//! The Rust app's UI sets and root, on no platform: what the runner of a
//! platform implemented outside NSH starts (`docs/dev/platforms.md`).

/// The UI sets the app serves: `nsh/base`, then the app's crates'.
pub fn ui_sets() -> Vec<nsh_core::element::Registry> {
    vec![nsh_base::registry(), set_app_excellent_app()]
}

/// The app's crates' UI sets without `nsh/base`'s: what a platform that runs
/// no TSX serves (their files, such as fonts), so that its module links only
/// the base components the app's code uses.
pub fn app_ui_sets() -> Vec<nsh_core::element::Registry> {
    vec![set_app_excellent_app()]
}

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and status bar.
pub fn root() -> nsh_core::element::Root {
    app_root()
}

/// The UI set of the crate excellent-app: the entry `#[native_module("…")]` makes in it.
fn set_app_excellent_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_excellent_app::*;
    __nsh_native_module() // the crate excellent-app (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_excellent_app::App>()
        .app_id("com.excellentpdf.app")
        .translations_json(include_str!("C:\\dev\\ExcellentPDF\\app\\.nsh/rust\\translations.json"))
}
