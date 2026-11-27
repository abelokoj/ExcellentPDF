//! T6: the shell starts with its three panels, and every command runs from its shortcut and from the palette.

use nsh_core::geometry::{point, px, size};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

fn start() -> Headless {
    let mut config = EngineConfig::new(ecclesia_app::root());
    // A desktop window: below 600 points the dock area turns its docks into drawers.
    config.viewport = size(px(1280.0), px(800.0));
    let mut app = Headless::start(config).expect("the app starts");
    app.wait_for("Home").expect("the home page shows");
    app
}

#[test]
fn starts_with_the_document_between_pages_and_inspector() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();
    let screen = app.screen();
    assert!(
        screen.matches("label:Home").expect("valid home selector"),
        "the compact home icon button shows"
    );
    assert!(screen.shows("Ecclesia PDF"), "the home page shows");
    for panel in ["Bookmarks", "Pages", "Outline", "Annotations", "Results"] {
        assert!(
            screen
                .matches(&format!("label:{panel}"))
                .expect("valid rail selector"),
            "{panel} icon on left rail"
        );
    }
    assert!(
        !screen.shows("Open a document to see its pages here"),
        "wide left drawer starts collapsed"
    );
    assert!(!screen.shows("Inspector"), "right sidebar starts closed");

    // Clicking a rail icon expands the wide left drawer; pressing Close panel collapses it back to the thin rail.
    app.press("label:Pages").expect("press Pages rail icon");
    app.wait_for("Open a document to see its pages here")
        .expect("Pages drawer expands");
    app.press("label:Close panel")
        .expect("press drawer close button");
    app.wait_for_gone("Open a document to see its pages here", wait)
        .expect("wide left drawer collapses");
    assert!(
        app.screen()
            .matches("label:Pages")
            .expect("valid rail selector"),
        "thin left icon rail stays visible"
    );
}

#[test]
fn shortcuts_toggle_the_docks() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();
    app.press_key("f12").expect("F12");
    app.wait_for_gone("label:Pages", wait)
        .expect("the left icon rail hides");
    app.press_key("f12").expect("F12");
    app.wait_for("label:Pages")
        .expect("the left icon rail opens again");

    app.press_key("mod+alt+b")
        .expect("the inspector's shortcut");
    app.wait_for("Inspector").expect("the right sidebar opens");
    app.press_key("mod+alt+b")
        .expect("the inspector's shortcut again");
    app.wait_for_gone("Inspector", wait)
        .expect("the right sidebar closes");
}

#[test]
fn the_palette_lists_every_command_and_runs_one() {
    let wait = std::time::Duration::from_secs(5);
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
    app.wait_for_gone("role:dialog", wait)
        .expect("the palette closes");
    app.wait_for("Inspector")
        .expect("the chosen command opened the right sidebar");
}

/// The title's colour: the theme's `foreground`, which every family tints its own way.
fn title_colour(screen: &nsh_testing::Screen) -> Option<nsh_core::Color> {
    screen
        .span_style("text:Ecclesia PDF")
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
    // The picker places its highlight and takes the keyboard a frame after it shows: keys before that race it (slow
    // CI runners lose).
    app.settle();
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
    for label in ["Appearance", "Theme", "Light or dark"] {
        assert!(
            app.screen().shows(label),
            "{label} is in the default Appearance section"
        );
    }

    // Sidebar section navigation switches the right pane to Reading.
    app.press("role:dialog >> label:Reading")
        .expect("click Reading section in sidebar");
    app.wait_for("Reading ruler colour")
        .expect("Reading section content shows");

    // Live search filters across regular and advanced sections (e.g. Security & sandbox).
    app.type_text("role:dialog >> text:Search settings", "sandbox")
        .expect("type into settings search");
    app.wait_for("Active OS containment")
        .expect("live search finds sandbox setting across sections");

    // Clicking Advanced in the sidebar clears search and opens the Advanced Settings area.
    app.press("role:dialog >> label:Advanced")
        .expect("click Advanced section in sidebar");
    app.wait_for("Configuration overrides")
        .expect("Advanced Settings settings.toml inspector shows");
    assert!(
        app.screen().shows("Active OS containment"),
        "Security & sandbox card shows inside Advanced Settings"
    );

    // Clicking Appearance returns to the Appearance section.
    app.press("role:dialog >> label:Appearance")
        .expect("click Appearance section in sidebar");
    app.wait_for("Light or dark")
        .expect("Appearance section shows again");

    app.press("role:dialog >> text:Neutral")
        .expect("the theme row's button");
    app.wait_for("role:dialog[Theme]")
        .expect("the theme picker opens in its place");
    app.settle();
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
    let mut config = EngineConfig::new(ecclesia_app::root());
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
    let mut config = EngineConfig::new(ecclesia_app::root());
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

#[test]
fn clicking_outside_a_dialog_or_dropdown_closes_it() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();

    // 1. Settings window (centered `list_dialog`): non-interactive text inside the card does not
    //    target the dismiss backdrop; clicking outside on the backdrop closes the dialog.
    app.press_key("mod+,").expect("Ctrl+,");
    app.wait_for("role:dialog[Settings]")
        .expect("the settings window opens");
    assert!(
        app.screen()
            .press_target("role:dialog >> text:Changes are saved as you make them")
            .expect("valid settings text selector")
            .is_none(),
        "non-interactive card content must not route presses to the dismiss backdrop"
    );
    let backdrop = app
        .screen()
        .layout("id:dialog-backdrop")
        .expect("valid backdrop selector")
        .expect("the settings dialog renders a dismiss backdrop");
    assert!(
        backdrop.bounds.size.width >= px(1200.0) && backdrop.bounds.size.height >= px(700.0),
        "dismiss backdrop covers the window: {:?}",
        backdrop.bounds
    );
    app.press_at("id:dialog-backdrop", point(px(16.0), px(16.0)))
        .expect("click outside settings card");
    app.wait_for_gone("role:dialog", wait)
        .expect("clicking outside closes the settings window");

    // 2. Tab list dropdown (top-right): clicking a control inside keeps it open; clicking outside closes it.
    app.press_key("mod+shift+a").expect("Ctrl+Shift+A");
    app.wait_for("role:dialog[Tabs]")
        .expect("the tab list dropdown opens");
    app.press("label:Wider tab list")
        .expect("click widen control inside tab list");
    assert!(
        app.screen()
            .matches("role:dialog[Tabs]")
            .expect("valid tab list dialog selector"),
        "clicking inside the tab list card keeps it open"
    );
    app.press_at("id:dialog-backdrop", point(px(16.0), px(700.0)))
        .expect("click outside tab list card");
    app.wait_for_gone("role:dialog", wait)
        .expect("clicking outside closes the tab list dropdown");

    // 3. Workspace dropdown (top-left): clicking outside closes it.
    app.press_key("mod+k").expect("Ctrl+K");
    app.wait_for("role:dialog[Command palette]")
        .expect("the palette opens");
    app.type_text("role:dialog >> text:Type a command", "switch workspace")
        .expect("typing filters");
    app.press_key("enter")
        .expect("Return opens workspace dropdown");
    app.wait_for("role:dialog[Workspaces]")
        .expect("the workspace dropdown opens");
    app.press_at("id:dialog-backdrop", point(px(1200.0), px(700.0)))
        .expect("click outside workspace dropdown");
    app.wait_for_gone("role:dialog", wait)
        .expect("clicking outside closes the workspace dropdown");
}

#[test]
fn narrow_unmaximized_window_keeps_chrome_controls_within_viewport() {
    let wait = std::time::Duration::from_secs(5);
    let mut config = EngineConfig::new(ecclesia_app::root());
    config.viewport = size(px(800.0), px(600.0));
    let mut app = Headless::start(config).expect("the app starts at 800x600");
    app.wait_for("Home").expect("the home page shows");

    let screen = app.screen();
    for selector in [
        "label:Home",
        "text:Customize",
        "label:Toggle sidebar",
        "label:Toggle inspector",
    ] {
        let info = screen
            .layout(selector)
            .expect("valid selector")
            .unwrap_or_else(|| panic!("{selector} must be laid out at 800x600"));
        let right = info.bounds.origin.x + info.bounds.size.width;
        assert!(
            info.bounds.origin.x >= px(0.0) && right <= px(800.5),
            "{selector} overflowed the 800px viewport: {:?}",
            info.bounds
        );
    }

    // Right sidebar toggle and Customize button remain interactive at narrow unmaximized widths.
    app.press("label:Toggle inspector")
        .expect("press right sidebar toggle");
    app.wait_for("Inspector")
        .expect("right sidebar opens at 800x600");
    app.press("label:Toggle inspector")
        .expect("press right sidebar toggle again");
    app.wait_for_gone("Inspector", wait)
        .expect("right sidebar closes at 800x600");

    app.press("text:Customize").expect("press Customize button");
    app.wait_for("role:dialog[Customize toolbar]")
        .expect("customize toolbar dialog opens at 800x600");
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("role:dialog", wait)
        .expect("customize toolbar dialog closes");
}

#[test]
fn page_shortcuts_work_immediately_after_clicking_toolbar_and_chrome_buttons() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
    app.on_pick_paths(vec![file])
        .expect("the open dialog answers");
    app.press_key("mod+o").expect("Ctrl+O");
    app.wait_for("zlib-manual.pdf")
        .expect("the document opens in a tab");

    // Clicking toolbar / mode / rail buttons must never trap Space or block page shortcuts.
    app.press("text:Annotate")
        .expect("switch to Annotate mode via mode pill");
    app.settle();
    app.press("label:Toggle inspector")
        .expect("open right sidebar via toolbar button");
    app.wait_for("Inspector").expect("right sidebar opens");

    // Pressing Space while reading must be consumed by the PDF rather than re-pressing the
    // previously clicked Toggle inspector button.
    app.press_key("space").expect("Space on pages");
    app.settle();
    assert!(
        app.screen().shows("Inspector"),
        "Space must not re-activate the previously clicked toolbar button"
    );

    // Single-letter page shortcuts (`g`, `/`) work right after clicking chrome controls.
    app.press_key("g").expect("g opens Go to page");
    app.wait_for("role:dialog[Go to page]")
        .expect("Go to page opens via single-letter shortcut");
    app.press_key("escape").expect("Escape closes Go to page");
    app.wait_for_gone("role:dialog", wait)
        .expect("Go to page closes");

    // Opening and closing the left drawer clears sidebar focus and keeps page shortcuts working.
    app.press("label:Pages").expect("open Pages drawer");
    app.wait_for("label:Close panel >> nth=0")
        .expect("Pages drawer opens");
    app.press("label:Close panel >> nth=0")
        .expect("close Pages drawer");
    app.settle();

    app.press_key("/").expect("/ opens Find bar");
    app.wait_for("text:Find in document")
        .expect("Find bar opens via / page shortcut");
    app.press_key("escape").expect("Escape closes Find bar");
    app.wait_for_gone("text:Find in document", wait)
        .expect("Find bar closes on Escape");
}

#[test]
fn find_bar_text_input_keeps_plain_keys_and_closes_on_escape() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/zlib-manual.pdf");
    app.on_pick_paths(vec![file])
        .expect("the open dialog answers");
    app.press_key("mod+o").expect("Ctrl+O");
    app.wait_for("zlib-manual.pdf")
        .expect("the document opens in a tab");

    app.press_key("mod+f").expect("Ctrl+F");
    app.wait_for("text:Find in document")
        .expect("Find bar opens");
    app.type_text("text:Find in document", "g/f")
        .expect("typing page-shortcut letters into Find bar");
    app.settle();
    assert!(
        !app.screen()
            .matches("role:dialog[Go to page]")
            .expect("valid Go to page selector"),
        "typing 'g' inside the Find bar must not trigger Go to page"
    );
    app.press_key("escape")
        .expect("Escape closes Find bar even while input is focused");
    app.wait_for_gone("text:Find in document", wait)
        .expect("Find bar closes cleanly on Escape");
}

#[test]
fn palette_alias_and_tab_switching_shortcuts_work() {
    let wait = std::time::Duration::from_secs(5);
    let mut app = start();
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    app.on_pick_paths(vec![
        corpus.join("zlib-manual.pdf"),
        corpus.join("ten-page-paper.pdf"),
    ])
    .expect("the open dialog answers with two files");
    app.press_key("mod+o").expect("Ctrl+O");
    app.wait_for("zlib-manual.pdf").expect("first tab opens");
    app.wait_for("ten-page-paper.pdf")
        .expect("second tab opens");

    app.press_key("mod+shift+p").expect("Ctrl+Shift+P");
    app.wait_for("role:dialog[Command palette]")
        .expect("Ctrl+Shift+P opens the command palette");
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("role:dialog", wait)
        .expect("the palette closes");

    // Cycle across both tabs with Ctrl+Tab, Ctrl+PageDown, and Ctrl+PageUp, then close both.
    app.press_key("ctrl+tab").expect("Ctrl+Tab");
    app.press_key("ctrl+pagedown").expect("Ctrl+PageDown");
    app.press_key("ctrl+pageup").expect("Ctrl+PageUp");
    app.press_key("mod+w").expect("close active tab");
    app.press_key("mod+w").expect("close remaining tab");
    app.wait_for("Home")
        .expect("closing both tabs returns to Home");
}
