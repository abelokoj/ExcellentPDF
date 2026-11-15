//! T6: the shell starts with its three panels, and every command runs from its shortcut and from the palette.

use nsh_core::geometry::{px, size};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

fn start() -> Headless {
    let mut config = EngineConfig::new(excellent_app::root());
    // A desktop window: below 600 points the dock area turns its docks into drawers.
    config.viewport = size(px(1280.0), px(800.0));
    let mut app = Headless::start(config).expect("the app starts");
    app.wait_for("Home").expect("the home page shows");
    app
}

#[test]
fn starts_with_the_document_between_pages_and_inspector() {
    let app = start();
    let screen = app.screen();
    for panel in ["Document", "Pages", "Outline", "Inspector", "Results"] {
        assert!(screen.shows(panel), "{panel} tab");
    }
}

#[test]
fn shortcuts_toggle_the_docks() {
    let mut app = start();
    app.press_key("f12").expect("F12");
    app.wait_for_gone("Pages", std::time::Duration::from_secs(5))
        .expect("the sidebar closes");
    app.press_key("f12").expect("F12");
    app.wait_for("Pages").expect("the sidebar opens again");

    app.press_key("mod+alt+b")
        .expect("the inspector's shortcut");
    app.wait_for_gone("Inspector", std::time::Duration::from_secs(5))
        .expect("the inspector closes");
}

#[test]
fn the_palette_lists_every_command_and_runs_one() {
    let mut app = start();
    app.press_key("mod+k").expect("Ctrl+K");
    app.wait_for("role:dialog[Command palette]")
        .expect("the palette opens");
    for label in ["Command palette", "Toggle sidebar", "Toggle inspector"] {
        assert!(app.screen().shows(label), "{label} is in the palette");
    }
    // The title bar shows the same placeholder, so look inside the dialog.
    app.type_text("role:dialog >> text:Type a command", "inspector")
        .expect("typing filters");
    app.press_key("enter").expect("Return chooses");
    app.wait_for_gone("role:dialog", std::time::Duration::from_secs(5))
        .expect("the palette closes");
    app.wait_for_gone("Inspector", std::time::Duration::from_secs(5))
        .expect("the chosen command ran");
}

/// The title's colour: the theme's `sidebar-foreground`, which every family tints its own way.
fn title_colour(screen: &nsh_testing::Screen) -> Option<nsh_core::Color> {
    screen
        .span_style("text:ExcellentPDF")
        .ok()
        .map(|style| style.color)
}

/// Opens the theme picker from the palette, as a user without a shortcut does.
fn open_theme_picker(app: &mut Headless) {
    app.press_key("mod+k").expect("Ctrl+K");
    app.wait_for("role:dialog[Command palette]")
        .expect("the palette opens");
    app.type_text("role:dialog >> text:Type a command", "choose theme")
        .expect("typing filters");
    app.press_key("enter").expect("Return chooses");
    app.wait_for("role:dialog[Theme]")
        .expect("the theme picker opens");
}

#[test]
fn the_theme_picker_previews_live_escape_goes_back_and_enter_keeps() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();
    let neutral = title_colour(app.screen()).expect("the title shows");

    // K.17: moving the highlight applies the theme at once.
    open_theme_picker(&mut app);
    app.press_key("down").expect("Down");
    app.wait_until("Aurora previews", wait, |s| {
        title_colour(s).is_some_and(|c| c != neutral)
    })
    .expect("the highlighted theme is applied while the picker is open");
    let aurora = title_colour(app.screen()).expect("the title shows");

    // Escape puts back the theme in effect.
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("role:dialog", wait)
        .expect("the picker closes");
    app.wait_until("Neutral is back", wait, |s| {
        title_colour(s) == Some(neutral)
    })
    .expect("Escape goes back to the theme in effect");

    // Enter keeps the highlighted one.
    open_theme_picker(&mut app);
    app.press_key("down").expect("Down");
    app.wait_until("Aurora previews", wait, |s| title_colour(s) == Some(aurora))
        .expect("the preview starts from the theme in effect");
    app.press_key("enter").expect("Return keeps it");
    app.wait_for_gone("role:dialog", wait)
        .expect("the picker closes");
    assert_eq!(title_colour(app.screen()), Some(aurora), "Aurora is kept");
}

#[test]
fn the_settings_window_opens_from_its_key_and_hands_over_to_the_theme_picker() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();
    app.press_key("mod+,").expect("Ctrl+,");
    app.wait_for("role:dialog[Settings]")
        .expect("the settings window opens");
    for label in [
        "Appearance",
        "Theme",
        "Light or dark",
        "Reading ruler colour",
    ] {
        assert!(
            app.screen().shows(label),
            "{label} is in the settings window"
        );
    }
    app.press("role:dialog >> text:Neutral")
        .expect("the theme row's button");
    app.wait_for("role:dialog[Theme]")
        .expect("the theme picker opens in its place");
    assert!(
        !app.screen().shows("Light or dark"),
        "the settings window closed"
    );
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("role:dialog", wait)
        .expect("the picker closes");
}
