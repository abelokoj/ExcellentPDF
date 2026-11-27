//! The Windows release runner with `nsh/base` and the app's Rust components.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "windows")]
    nsh_platform_windows::set_ui_sets(vec![nsh_base::registry(), set_app_ecclesia_app()]);
    #[cfg(target_os = "windows")]
    nsh_platform_windows::set_components(app_components);
    nsh_windows_app::main()
}

/// The UI set of the crate ecclesia-app: the entry `#[native_module("…")]` makes in it.
#[cfg(target_os = "windows")]
fn set_app_ecclesia_app() -> nsh_core::element::Registry {
    #[allow(unused_imports)]
    use app_ecclesia_app::*;
    __nsh_native_module() // the crate ecclesia-app (nsh.json "rust")
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
        use app_ecclesia_app::*;
        __nsh_native_components_windows(__nsh_components); // the crate ecclesia-app (nsh.json "rust")
    }
}

// Without the entry: nsh's stand-in, which registers nothing.
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use nsh_core::element::__private::no_native_components as __nsh_native_components_windows;
