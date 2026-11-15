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

/// U.4: at 200 % scaling a 1280 × 800 screen is 640 × 400 points; the home page and the palette still fit and work.
#[test]
fn the_window_works_at_double_scaling() {
    let mut config = EngineConfig::new(excellent_app::root());
    config.viewport = size(px(640.0), px(400.0));
    let mut app = Headless::start(config).expect("the app starts");
    app.wait_for("Home").expect("the home page shows");
    app.press_key("mod+k").expect("Ctrl+K");
    app.wait_for("role:dialog[Command palette]")
        .expect("the palette opens");
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("role:dialog", std::time::Duration::from_secs(5))
        .expect("the palette closes");
}

/// Starts the app with the system's language `tag`, written right to left or not.
fn start_in(tag: &str, rtl: bool, first_text: &str) -> Headless {
    use nsh_core::platform::{Languages, Locale};
    let mut config = EngineConfig::new(excellent_app::root());
    config.viewport = size(px(1280.0), px(800.0));
    let locale = Locale::parse(tag).expect("a language tag");
    config.environment.locale = locale;
    config.environment.locale_settings.languages = Languages::one(locale);
    config.environment.rtl = rtl;
    let mut app = Headless::start(config).expect("the app starts");
    app.wait_for(first_text).expect("the home page shows");
    app
}

/// A shortcut's key cap ("⌘K", "Ctrl+K") names keys, which the platform spells, not `t!`.
fn is_key_cap(text: &str) -> bool {
    text.contains(['⌘', '⌥', '⇧', '⌃']) || text.starts_with("Ctrl+") || text.starts_with("Alt+")
}

/// U.7: in the pseudo-locale every text the window shows is bracketed, so none skips `t!`. File and workspace names
/// are the reader's own words, and texts with no letters (key caps, counts) have nothing to translate.
#[test]
fn every_text_goes_through_the_localization_layer() {
    let app = start_in("en-XA", false, "⟦Ĥöɱé⟧");
    let screen = app.screen();
    let untranslated: Vec<&str> = screen
        .texts()
        .into_iter()
        .filter(|text| {
            !text.contains('⟦')
                && !is_key_cap(text)
                && text.chars().any(|c| c.is_ascii_alphabetic())
        })
        .collect();
    assert!(untranslated.is_empty(), "text outside t!: {untranslated:?}");
}

/// U.7: the window works in Arabic, mirrored, and its commands run.
#[test]
fn the_window_works_in_arabic() {
    let mut app = start_in("ar", true, "الرئيسية");
    app.press_key("mod+k").expect("Ctrl+K");
    app.wait_for("role:dialog[لوحة الأوامر]")
        .expect("the palette opens, in Arabic");
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("role:dialog", std::time::Duration::from_secs(5))
        .expect("the palette closes");
}

/// U.1: a reader with no mouse opens a document, goes to a page, finds words and closes it again, from keys alone.
#[test]
fn a_document_is_read_from_the_keyboard_alone() {
    let mut app = start();
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
    app.on_pick_paths(vec![file])
        .expect("the open dialog answers");
    app.press_key("mod+o").expect("Ctrl+O");
    app.wait_for("zlib-manual.pdf")
        .expect("the document opens in a tab");

    app.press_key("mod+g").expect("Ctrl+G");
    app.wait_for("role:dialog[Go to page]")
        .expect("the page dialog opens");
    // Headless lays out no page canvas, so the document counts no pages and any page typed is refused; going to a
    // page is covered by `read::goto`'s tests. Here the dialog only has to open and close from the keyboard.
    app.press_key("escape").expect("Escape closes it");
    app.wait_for_gone("role:dialog", std::time::Duration::from_secs(5))
        .expect("the dialog closes");

    app.press_key("mod+f").expect("Ctrl+F");
    app.wait_for("text:Find in document")
        .expect("the find bar opens");
    app.press_key("escape").expect("Escape");

    app.press_key("mod+w").expect("Ctrl+W");
    app.wait_for("Home")
        .expect("closing the last tab shows home");
}
