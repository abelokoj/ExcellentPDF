//! The `excellent.page-canvas` component's props and commands (plan D5, K.8): parsing their JSON and acting on them,
//! the same on every platform. Each platform's component holds a [`PageCanvas`] and forwards to these.

use std::path::Path;
use std::time::Duration;

use excellent_core::annot::Markup;
use excellent_core::darkmode::{PageColors, Pictures};
use excellent_features::annotate::colors::Colors;
use excellent_features::read::autoscroll::Change;
use excellent_features::read::focus::Scope;
use excellent_features::read::ruler::{Step, Style};
use excellent_features::search::find::{Case, Direction, HitId, Marks, Options, Words};
use excellent_mupdf::PagePoint;
use nsh_core::JsonValue;

use super::canvas::{DEFAULT_HOVER_DELAY, PageCanvas, ZoomChange};
use super::host::Host;
use super::saved::SavedView;
use crate::view::ViewCommand;
use crate::viewport::{Anchor, Side, ZoomMode};

impl<H: Host> PageCanvas<H> {
    /// `path`: the document shown (a string; absent or not a string: none). The
    /// platform layer handles `label`, the accessible name, itself.
    /// `background`: the colour behind the pages, `#rrggbb` (anything else: unchanged). `readingRuler`: the reading
    /// ruler's look, `{height, color, opacity, invert}` (each missing or wrong one: its default). `focusScope`:
    /// `"paragraph"` or `"line"`. `autoScrollSpeed`: pixels a second at 96 dpi. `pageColors`: the colours the pages
    /// are drawn in (R.14), `{ink, paper, link, pictures}` with `#rrggbb` colours (`link` optional) and `pictures`
    /// `"keep"` or `"recolor"`; `null`, or without a valid `ink` and `paper`: the document's own. `reflowFontSize`:
    /// a reflowable book's text size in points (R.28); `null`: the default. `bookmarks`: the bookmarks the mini-map
    /// marks (R.17), `[{page, x, y, color}]` in page points with a `#rrggbb` colour (entries that don't parse are
    /// left out). `hoverDelay`: how long the pointer rests on a link or citation before its preview shows (R.25), in
    /// milliseconds; `false` turns previews off; `null` or anything else: the default. `edits`: a number the app
    /// moves with every edit, undo or redo (A.1), so the pages they changed are drawn again. `annotationColors`: the
    /// tools' colours (A.3), as `excellent-features`' `Colors::to_json` writes them; `null`: the defaults.
    pub fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        let text = value.and_then(JsonValue::as_str);
        match name {
            "readingRuler" => self.set_ruler_style(value.map_or_else(Style::default, ruler_style)),
            "focusScope" => self.set_focus_scope(text.and_then(Scope::parse).unwrap_or_default()),
            "autoScrollSpeed" => {
                if let Some(speed) = value.and_then(JsonValue::as_f64) {
                    self.set_auto_scroll_speed(speed as f32);
                }
            }
            "pageColors" => self.set_page_colors(value.and_then(page_colors)),
            "reflowFontSize" => {
                self.set_reflow_font_size(value.and_then(JsonValue::as_f64).map(|size| size as f32))
            }
            "edits" => self.edited(),
            "annotationColors" => {
                self.set_annotation_colors(value.map(Colors::from_json).unwrap_or_default())
            }
            "bookmarks" => self.set_bookmarks(value.map(bookmarks).unwrap_or_default()),
            "hoverDelay" => self.set_hover_delay(match value {
                Some(JsonValue::Bool(false)) => None,
                other => Some(
                    other
                        .and_then(JsonValue::as_f64)
                        .map_or(DEFAULT_HOVER_DELAY, |ms| {
                            Duration::from_millis(ms.clamp(0.0, 10_000.0) as u64)
                        }),
                ),
            }),
            "path" => self.open(text.map(Path::new)),
            "background" => match text.and_then(parse_rgb) {
                Some(rgb) => self.set_background(rgb),
                None => log::warn!("page canvas: `background` is not #rrggbb: {value:?}"),
            },
            _ => log::debug!("page canvas: unknown prop `{name}`"),
        }
    }

    /// `zoomIn`, `zoomOut`, `actualSize`, `screenDown`, `screenUp`, `stepLeft`, `stepRight` (R.3), `historyBack`, `historyForward`, `firstPage`,
    /// `lastPage`, `goToPage` (`{"index": n}`, from 0), `setView` (a `view` event's data; with only a `page`: that
    /// page), `follow` (V.8: `{page, across, down, fit, zoom}`, `fit` and `zoom` optional; no history), `goToPoint` (`{page, x, y, zoom}`, page points; `zoom` may be `null`), `copy`, `selectAll` (the
    /// reader's page), `readerPlace` (B.1, B.4, B.6: answered by the event of that name), and the view
    /// commands [`ViewCommand::parse`] knows (layouts, rotation, page box, fit modes). The reading aids (R.19-R.21):
    /// `toggleRuler`, `toggleFocus`, `lineDown`, `lineUp`, `toggleAutoScroll`, `autoScrollPause` (without
    /// auto-scroll: a screen down), `autoScrollFaster`, `autoScrollSlower`, `leaveModes`. Find (F.1): `find`
    /// (`{id, text, matchCase, wholeWord, matchMarks}`; blank text clears it), `findNext`, `findPrevious`,
    /// `findShow` (`{page, nth}`), `findClear`.
    ///
    /// Events: `pages` (`{count, labels}`, labels `null` when the pages go by number) when a document opens or
    /// closes, `page` (`{index}`) when the reader's page changes, `copy` (`{text}`) with the selected text,
    /// `autoScrollSpeed` (`{speed}`) when the reader changes it, `edge` (`{at}`) when the mouse reaches or
    /// leaves the canvas's top edge, `press` when the reader presses on the pages, `view` (the place, zoom,
    /// layout and rotation; see the `saved` module) when the view settles somewhere new, and `readerPlace`
    /// (`{page, x, y, zoom, title}`, see [`PageCanvas::reader_place`]) when the app asked for it, and `found`
    /// (`{id, searched, pages, total, current, hits}`) as a search finds hits and moves between them.
    pub fn command(&mut self, name: &str, args: &JsonValue) {
        let canvas = self;
        match name {
            "zoomIn" => canvas.zoom_step(ZoomChange::In),
            "zoomOut" => canvas.zoom_step(ZoomChange::Out),
            "actualSize" => canvas.actual_size(),
            "screenDown" => canvas.scroll_screen(1.0),
            "screenUp" => canvas.scroll_screen(-1.0),
            "stepLeft" => canvas.step_side(Side::Left),
            "stepRight" => canvas.step_side(Side::Right),
            "historyBack" => canvas.history_back(),
            "historyForward" => canvas.history_forward(),
            "firstPage" => canvas.first_page(),
            "lastPage" => canvas.last_page(),
            "copy" => canvas.copy(),
            "highlight" => canvas.mark(Markup::Highlight),
            "underline" => canvas.mark(Markup::Underline),
            "squiggly" => canvas.mark(Markup::Squiggly),
            "strikeOut" => canvas.mark(Markup::StrikeOut),
            "replaceText" => canvas.propose_replace(),
            "insertText" => canvas.propose_insert(),
            "noteSelection" => canvas.note_selection(),
            "selectAll" => canvas.select_page(),
            "readerPlace" => canvas.reader_place(),
            "toggleRuler" => canvas.toggle_ruler(),
            "toggleFocus" => canvas.toggle_focus(),
            "lineDown" => canvas.line_step(Step::Next),
            "lineUp" => canvas.line_step(Step::Previous),
            "toggleAutoScroll" => canvas.toggle_auto_scroll(),
            "autoScrollPause" => canvas.pause_or_screen_down(),
            "autoScrollFaster" => canvas.change_auto_scroll_speed(Change::Faster),
            "autoScrollSlower" => canvas.change_auto_scroll_speed(Change::Slower),
            "leaveModes" => canvas.leave_modes(),
            "find" => {
                let id = args.get("id").and_then(JsonValue::as_u64).unwrap_or(0);
                let text = args.get("text").and_then(JsonValue::as_str).unwrap_or("");
                canvas.find(id, text, find_options(args));
            }
            "findNext" => canvas.find_step(Direction::Next),
            "findPrevious" => canvas.find_step(Direction::Previous),
            "findClear" => canvas.find_clear(),
            "findShow" => match hit_id(args) {
                Some(id) => canvas.find_show(id),
                None => log::warn!("page canvas: findShow without a page and nth: {args}"),
            },
            "goToPage" => match args.get("index").and_then(JsonValue::as_u64) {
                Some(index) => canvas.go_to_page(usize::try_from(index).unwrap_or(usize::MAX)),
                None => log::warn!("page canvas: goToPage without an index: {args}"),
            },
            "goToPoint" => match go_to_point_args(args) {
                Some((page, point, zoom)) => canvas.go_to_point(page, point, zoom),
                None => log::warn!("page canvas: goToPoint without a page and point: {args}"),
            },
            "follow" => match follow_args(args) {
                Some((anchor, zoom)) => canvas.follow_pane(anchor, zoom),
                None => log::warn!("page canvas: follow without a page and place: {args}"),
            },
            "setView" => match SavedView::from_json(args) {
                Some(view) => canvas.set_view(view),
                None => match args.get("page").and_then(JsonValue::as_u64) {
                    Some(page) => canvas.go_to_page(usize::try_from(page).unwrap_or(usize::MAX)),
                    None => log::warn!("page canvas: setView without a view: {args}"),
                },
            },
            _ => match ViewCommand::parse(name) {
                Some(command) => canvas.view_command(command),
                None => log::warn!("page canvas: unknown command `{name}`"),
            },
        }
    }
}

/// The page, point and zoom of a `goToPoint` command; `None` without a page or a point.
fn go_to_point_args(args: &JsonValue) -> Option<(usize, PagePoint, Option<f32>)> {
    let number = |key: &str| args.get(key).and_then(JsonValue::as_f64);
    let page = usize::try_from(args.get("page")?.as_u64()?).ok()?;
    let point = PagePoint {
        x: number("x")? as f32,
        y: number("y")? as f32,
    };
    let zoom = number("zoom").map(|zoom| zoom as f32);
    Some((page, point, zoom))
}

/// The place of a `follow` command (V.8), `{page, across, down}`, and its zoom when it has `fit` (`"custom"`,
/// `"width"`, `"page"` or `"height"`) and `zoom`.
fn follow_args(args: &JsonValue) -> Option<(Anchor, Option<(ZoomMode, f32)>)> {
    let number = |key: &str| args.get(key)?.as_f64().filter(|n| n.is_finite());
    let anchor = Anchor {
        page: usize::try_from(args.get("page")?.as_u64()?).ok()?,
        across: number("across")? as f32,
        down: number("down")? as f32,
    };
    let mode = match args.get("fit").and_then(JsonValue::as_str) {
        Some("custom") => Some(ZoomMode::Custom),
        Some("width") => Some(ZoomMode::FitWidth),
        Some("page") => Some(ZoomMode::FitPage),
        Some("height") => Some(ZoomMode::FitHeight),
        _ => None,
    };
    let zoom = number("zoom")
        .filter(|zoom| *zoom > 0.0)
        .map(|zoom| zoom as f32);
    Some((anchor, mode.zip(zoom)))
}

/// How a `find` command matches: each flag missing or not `true` is off.
fn find_options(args: &JsonValue) -> Options {
    let on = |key: &str| args.get(key).and_then(JsonValue::as_bool) == Some(true);
    Options {
        case: if on("matchCase") {
            Case::Match
        } else {
            Case::Ignore
        },
        words: if on("wholeWord") {
            Words::Whole
        } else {
            Words::Anywhere
        },
        marks: if on("matchMarks") {
            Marks::Match
        } else {
            Marks::Ignore
        },
    }
}

/// The hit of a `findShow` command.
fn hit_id(args: &JsonValue) -> Option<HitId> {
    let index = |key: &str| usize::try_from(args.get(key)?.as_u64()?).ok();
    Some(HitId {
        page: index("page")?,
        nth: index("nth")?,
    })
}

/// The ruler's look from the `readingRuler` prop: each value missing or of the wrong kind is its default.
fn ruler_style(value: &JsonValue) -> Style {
    let default = Style::default();
    let number = |key: &str, fallback: f32| {
        value
            .get(key)
            .and_then(JsonValue::as_f64)
            .map_or(fallback, |n| n as f32)
    };
    Style {
        height: number("height", default.height),
        color: value
            .get("color")
            .and_then(JsonValue::as_str)
            .and_then(parse_rgb)
            .unwrap_or(default.color),
        opacity: number("opacity", default.opacity),
        invert: value
            .get("invert")
            .and_then(JsonValue::as_bool)
            .unwrap_or(default.invert),
    }
}

/// The colours of the `pageColors` prop; `None` for `null`, and (logged) for colours that don't parse.
fn page_colors(value: &JsonValue) -> Option<PageColors> {
    if value.is_null() {
        return None;
    }
    let color = |key: &str| {
        let rgb = value
            .get(key)
            .and_then(JsonValue::as_str)
            .and_then(parse_rgb)?;
        Some([16, 8, 0].map(|shift| u8::try_from(rgb >> shift & 0xff).expect("a byte")))
    };
    let (Some(ink), Some(paper)) = (color("ink"), color("paper")) else {
        log::warn!("page canvas: `pageColors` needs #rrggbb `ink` and `paper`: {value}");
        return None;
    };
    Some(PageColors {
        ink,
        paper,
        link: color("link"),
        pictures: match value.get("pictures").and_then(JsonValue::as_str) {
            Some("recolor") => Pictures::Recolor,
            _ => Pictures::Keep,
        },
    })
}

/// The `bookmarks` prop's entries: `(page, point, colour)`, each that doesn't parse left out.
fn bookmarks(value: &JsonValue) -> Vec<(usize, PagePoint, u32)> {
    let entry = |entry: &JsonValue| {
        let page = usize::try_from(entry.get("page")?.as_u64()?).ok()?;
        let x = entry.get("x")?.as_f64()? as f32;
        let y = entry.get("y")?.as_f64()? as f32;
        let color = parse_rgb(entry.get("color")?.as_str()?)?;
        Some((page, PagePoint { x, y }, color))
    };
    value
        .as_array()
        .map(|entries| entries.iter().filter_map(entry).collect())
        .unwrap_or_default()
}

/// `#rrggbb` as `0xRRGGBB`.
fn parse_rgb(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        Case, HitId, Marks, Options, PageColors, PagePoint, Pictures, Style, Words, bookmarks,
        find_options, hit_id, page_colors, parse_rgb, ruler_style,
    };

    #[test]
    fn reads_bookmarks_and_leaves_out_the_broken_ones() {
        let read = bookmarks(&serde_json::json!([
            { "page": 2, "x": 10.0, "y": 20.5, "color": "#ff8000" },
            { "page": -1, "x": 0, "y": 0, "color": "#000000" },
            { "page": 3, "x": 0, "y": 0, "color": "red" },
            { "page": 4, "x": 1, "y": 2, "color": "#00ff00" },
        ]));
        assert_eq!(
            read,
            [
                (2, PagePoint { x: 10.0, y: 20.5 }, 0xff8000),
                (4, PagePoint { x: 1.0, y: 2.0 }, 0x00ff00),
            ]
        );
        assert!(bookmarks(&serde_json::json!(null)).is_empty());
    }

    #[test]
    fn reads_find_options_and_hits() {
        let options = find_options(&serde_json::json!({ "matchCase": true, "wholeWord": "yes" }));
        assert_eq!(
            options,
            Options {
                case: Case::Match,
                words: Words::Anywhere,
                marks: Marks::Ignore
            }
        );
        assert_eq!(
            hit_id(&serde_json::json!({ "page": 3, "nth": 1 })),
            Some(HitId { page: 3, nth: 1 })
        );
        assert_eq!(hit_id(&serde_json::json!({ "page": 3 })), None);
    }

    #[test]
    fn reads_page_colours_and_falls_back_to_the_document_own() {
        let colors = page_colors(&serde_json::json!({
            "ink": "#e6e6e1", "paper": "#121724", "link": "#78aaff", "pictures": "recolor"
        }));
        assert_eq!(
            colors,
            Some(PageColors {
                ink: [0xe6, 0xe6, 0xe1],
                paper: [0x12, 0x17, 0x24],
                link: Some([0x78, 0xaa, 0xff]),
                pictures: Pictures::Recolor,
            })
        );
        let kept = page_colors(&serde_json::json!({ "ink": "#ffffff", "paper": "#000000" }));
        assert_eq!(
            kept.map(|c| (c.link, c.pictures)),
            Some((None, Pictures::Keep))
        );
        assert_eq!(page_colors(&serde_json::Value::Null), None);
        assert_eq!(
            page_colors(&serde_json::json!({ "ink": "white", "paper": "#000000" })),
            None
        );
    }

    #[test]
    fn reads_hex_colours() {
        assert_eq!(parse_rgb("#1a2B3c"), Some(0x1a2b3c));
        assert_eq!(parse_rgb("1a2b3c"), None);
        assert_eq!(parse_rgb("#fff"), None);
        assert_eq!(parse_rgb("#gggggg"), None);
    }

    #[test]
    fn the_ruler_prop_falls_back_to_defaults_value_by_value() {
        let style =
            ruler_style(&serde_json::json!({ "height": 30, "color": "#ff0000", "invert": "yes" }));
        assert_eq!(style.height, 30.0);
        assert_eq!(style.color, 0xff0000);
        assert_eq!(style.opacity, Style::default().opacity);
        assert!(!style.invert);
    }
}
