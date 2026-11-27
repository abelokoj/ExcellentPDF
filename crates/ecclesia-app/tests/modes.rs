//! K.18: the mode bar shows the chosen mode's tools and only those, the keyboard steps through the modes, and the
//! customize dialog opens from the bar and closes with Escape.

use std::time::Duration;

use nsh_core::geometry::{px, size};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

const CUSTOMIZE: &str = "role:dialog[Customize toolbar]";

#[test]
fn each_mode_shows_only_its_tools_and_the_toolbar_can_be_customized() {
    let mut config = EngineConfig::new(ecclesia_app::root());
    config.viewport = size(px(1280.0), px(800.0));
    let mut app = Headless::start(config).expect("the app starts");

    app.wait_for("role:toolbar[Mode and tools]")
        .expect("the mode bar shows");
    app.wait_for("Zoom in").expect("Read mode's tools");
    assert!(
        !app.screen().shows("First page"),
        "Present's tools stay away in Read"
    );

    app.press("label:Present").expect("Present");
    app.wait_for("First page").expect("Present mode's tools");
    app.wait_for_gone("Zoom in", Duration::from_secs(5))
        .expect("Read's tools go");

    // Present is last of the modes offered (Write and Study have no tools yet): the next mode is Read again.
    app.press_key("mod+shift+m").expect("Ctrl+Shift+M");
    app.wait_for("Zoom in").expect("the next mode is Read");

    app.press("label:Customize toolbar…").expect("Customize");
    app.wait_for(CUSTOMIZE).expect("the dialog opens");
    assert!(app.screen().shows("On the Read toolbar"));
    app.press_key("escape").expect("Escape");
    app.wait_for_gone(CUSTOMIZE, Duration::from_secs(5))
        .expect("Escape closes it");
}
