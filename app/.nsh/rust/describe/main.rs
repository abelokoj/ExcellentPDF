//! Describes the app's UI sets for `nsh`: one JSON document on standard
//! output (nsh-core `ui::description`).

fn main() {
    let sets = vec![set_app_ecclesia_app()];
    println!("{}", nsh_core::element::description::Description::of(&sets).to_json());
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

// The features the app's programs share (`.nsh/rust/hack`).
use workspace_hack as _;
