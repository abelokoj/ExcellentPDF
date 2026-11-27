//! U.9: the first-run guide opens at a start without files, steps forward and back, closes on Skip, and comes back
//! from the palette. A test binary of its own: offering the guide is for the whole process, and the shell's tests
//! drive the window without it.

use std::time::Duration;

use nsh_core::geometry::{px, size};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

const GUIDE: &str = "role:dialog[Getting started]";

#[test]
fn the_guide_opens_at_first_start_steps_and_comes_back_from_the_palette() {
    ecclesia_app::offer_first_run_guide();
    let mut config = EngineConfig::new(ecclesia_app::root());
    config.viewport = size(px(1280.0), px(800.0));
    let mut app = Headless::start(config).expect("the app starts");

    app.wait_for(GUIDE)
        .expect("the guide opens with the window");
    assert!(app.screen().shows("Step 1 of 3"));
    app.press("label:Next").expect("Next");
    app.wait_for("Pick a mode, make reading yours")
        .expect("the second step");
    app.press("label:Back").expect("Back");
    app.wait_for("Step 1 of 3").expect("back at the first step");
    app.press("label:Skip").expect("Skip");
    app.wait_for_gone(GUIDE, Duration::from_secs(5))
        .expect("Skip closes the guide");

    app.press_key("mod+k").expect("Ctrl+K");
    app.wait_for("role:dialog[Command palette]")
        .expect("the palette opens");
    app.type_text("role:dialog >> text:Type a command", "getting-started")
        .expect("typing filters");
    app.press_key("enter").expect("Return chooses");
    app.wait_for(GUIDE)
        .expect("the command shows the guide again");
    app.press_key("escape").expect("Escape");
    app.wait_for_gone(GUIDE, Duration::from_secs(5))
        .expect("Escape skips it");
}
