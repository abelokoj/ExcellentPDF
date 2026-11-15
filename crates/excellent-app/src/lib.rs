//! The NSH app (plan §5.4, §10): the root component, the window's shell and the wiring between them.
//!
//! `app/nsh.json` names [`App`] as the root; the desktop binary (`src/main.rs`) and the tests start it through
//! [`root`], which adds the app's translations.

mod bookmarks;
mod commands;
mod editing;
mod full_screen;
mod goto;
mod keymap;
mod l10n;
mod library;
mod links;
mod list_dialog;
mod open;
mod outline;
mod reading;
mod reading_theme_bar;
mod recent;
mod session;
mod settings;
mod settings_window;
mod shell;
mod tabs;
mod theme;
mod thumbnails;

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

/// [`App`] as the runners and tests start it, with the app's translations (U.7) and the user's theme in place before
/// the window shows (K.17).
pub fn root() -> Root {
    let choice = theme::saved();
    Root::of::<App>()
        .translations(l10n::translations())
        .tokens(theme::tokens(choice))
        .appearance(choice.appearance)
}
