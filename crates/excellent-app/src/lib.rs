//! The NSH app (plan §5.4, §10): the root component, the window's shell and the wiring between them.
//!
//! `app/nsh.json` names [`App`] as the root; the desktop binary (`src/main.rs`) and the tests start it through
//! [`root`], which adds the app's translations.

mod annotate;
mod bookmarks;
mod commands;
mod editing;
mod find;
mod full_screen;
mod goto;
mod home;
mod keymap;
mod l10n;
mod library;
mod links;
mod list_dialog;
mod modes;
mod onboarding;
mod open;
mod outline;
mod panes;
mod print;
mod reading;
mod reading_theme_bar;
mod recent;
mod recovery_bar;
mod session;
mod settings;
mod settings_window;
mod shell;
mod tabs;
mod theme;
mod thumbnails;
mod windows;
mod workspaces;

pub use open::{Started, start};

use nsh_base::theme as current_theme;
use nsh_core::element::Root;
use nsh_core::prelude::*;

/// The app's UI set (`app/nsh.json` `"rust"`).
#[native_module("excellent/app")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// The app's root: the whole window, on the theme's background.
#[component]
pub fn App() -> impl IntoElement {
    // With the line `main` logs, this splits cold start between NSH's setup and our first build (T10).
    log::debug!("app: building the root view");
    view()
        .flex_1()
        .bg(move || current_theme().background)
        .child(shell::shell())
}

/// The app's id (`app/nsh.json` `"id"`; a placeholder until the maintainer picks one): NSH keeps the app's files, and
/// ours, under it in the system's data folder.
pub const APP_ID: &str = "com.excellentpdf.app";

/// Reads the user's settings (K.16). The desktop binary calls it before [`root`]; tests don't, so they never read or
/// write the user's file.
pub fn load_settings() {
    settings::load_shared();
}

/// Opens the library database (K.20), for the session (R.9). The desktop binary calls it before [`root`]; tests
/// don't, so they neither restore nor save a session.
pub fn open_library() {
    library::open();
}

/// Lets the first-run guide and the tips show in this run (U.9). The desktop binary calls it; tests don't, so no
/// guide lies over what they drive (`tests/onboarding` turns it on for its own).
pub fn offer_first_run_guide() {
    onboarding::offer();
}

/// Keeps the operation logs of the documents edited (K.14) in the app's folder, so a crash's edits are offered back
/// at the next start. The desktop binary calls it before [`root`]; tests don't, so their logs stay in the temporary
/// folder.
pub fn keep_edit_logs() {
    match settings::folder() {
        Some(folder) => {
            if !excellent_core::shared::keep_logs_in(folder.join("oplog")) {
                log::warn!("edit logs: a document opened first; they stay in the temporary folder");
            }
        }
        None => log::warn!("edit logs: no app folder; they stay in the temporary folder"),
    }
}

/// The page canvas's component name (plan D5); every platform registers its own under it.
pub(crate) const PAGE_CANVAS: &str = "excellent.page-canvas";

/// The app's native components on Windows: the page canvas. NSH's own hosts (`nsh run`) call this through the
/// attribute; the desktop binary calls it itself.
#[native_components(windows)]
pub fn components(components: &nsh_platform_windows::Components) {
    excellent_canvas_win::register(components);
}

/// The app's native components on Linux: the page canvas (K.10).
#[native_components(linux)]
pub fn components(components: &nsh_platform_linux::Components) {
    excellent_canvas_gtk::register(components);
}

/// The app's native components on macOS: the page canvas (K.9).
#[native_components(macos)]
pub fn components(components: &nsh_platform_macos::Components) {
    excellent_canvas_mac::register(components);
}

/// [`App`] as the runners and tests start it, with the app's translations (U.7) and the user's theme in place before
/// the window shows (K.17).
pub fn root() -> Root {
    let choice = theme::saved();
    theme::follow_system(&settings::shared());
    Root::of::<App>()
        .translations(l10n::translations())
        .tokens(theme::tokens(choice))
        .appearance(choice.appearance)
}
