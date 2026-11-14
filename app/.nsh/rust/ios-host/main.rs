//! The iOS development host with `nsh/base` and the app's Rust components.

fn main() {
    #[cfg(target_os = "ios")]
    nsh_platform_ios::set_ui_sets(vec![nsh_base::registry(), set_app_excellent_app()]);
    #[cfg(target_os = "ios")]
    nsh_platform_ios::set_components(app_components);
    #[cfg(target_os = "ios")]
    nsh_platform_ios::set_root(app_root);
    nsh_ios_host::main()
}

/// The UI set of the crate excellent-app: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "ios")]
fn set_app_excellent_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_excellent_app::*;
    __nsh_native_module() // the crate excellent-app (nsh.json "rust")
}

// Without the entry: nsh's stand-in, which fails the build saying what to add.
#[cfg(target_os = "ios")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_module as __nsh_native_module;

/// The app's native components on this platform: what `#[native_components(ios)]` registers in its crates.
#[cfg(target_os = "ios")]
fn app_components(__nsh_components: &nsh_platform_ios::Components) {
    {
        #[allow(unused_imports)]
        use app_excellent_app::*;
        __nsh_native_components_ios(__nsh_components); // the crate excellent-app (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "ios")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_ios;

/// The app's root (`nsh.json` `"root"`), with its theme, appearance and
/// status bar.
#[cfg(target_os = "ios")]
fn app_root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<app_excellent_app::App>()
        .app_id("com.excellentpdf.app")
        .translations_json(include_str!("C:\\dev\\ExcellentPDF\\app\\.nsh/rust\\translations.json"))
}
