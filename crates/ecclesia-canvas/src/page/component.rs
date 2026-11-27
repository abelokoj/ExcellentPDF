//! The `ecclesia.page-canvas` component's props and commands (plan D5, K.8): parsing their JSON and acting on them,
//! the same on every platform. Each platform's component holds a [`PageCanvas`] and forwards to these.

use std::path::Path;
use std::time::Duration;

use ecclesia_core::annot::Markup;
use ecclesia_core::darkmode::{PageColors, Pictures};
use ecclesia_features::annotate::colors::Colors;
use ecclesia_features::annotate::shapes::Kind;
use ecclesia_features::read::autoscroll::Change;
use ecclesia_features::read::focus::Scope;
use ecclesia_features::read::ruler::{Step, Style};
use ecclesia_features::search::find::{Case, Direction, HitId, Marks, Options, Words};
use ecclesia_ink::erase::Eraser;
use ecclesia_ink::pens::{DEFAULT_SIZE, Pen, PenKind};
use ecclesia_mupdf::{PagePoint, ReflowStyle};
use nsh_core::JsonValue;

use super::canvas::{
    DEFAULT_HOVER_DELAY, DEFAULT_HOVER_MAX_WIDTH_PCT, NoteMode, PageCanvas, ZoomChange,
};
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
    /// milliseconds; `false` turns previews off; `null` or anything else: the default. `hoverMaxWidthPct`: maximum
    /// reference preview width as a percentage of the page width (`40..=100`, default `90`). `edits`: a number the app
    /// moves with every edit, undo or redo (A.1), so the pages they changed are drawn again. `annotationColors`: the
    /// tools' colours (A.3), as `ecclesia-features`' `Colors::to_json` writes them; `null`: the defaults. `drawing`: the
    /// drawing tool on (A.8, A.9, A.10), a `shapes::Kind`'s name; anything else: none. `margin`: `true` while the app
    /// shows its margin column (A.11), when the canvas reports where its pages are (`margin` events). `selected`: the
    /// annotations selected (A.13), `{page, items: [{id, bounds, grips, locked}]}`; anything else: none. `pen`: the
    /// pen strokes draw with (H.2), see `pen`. `eraser`: the eraser the pen uses instead (H.5), an `Eraser`'s
    /// name; anything else: the pen draws. Erasing sends `erase` (`{page, eraser, radius, points}`, page points).
    /// `lasso`: `true` while the lasso is on (H.6): a drag on a page draws a loop (`annotLasso`).
    /// `blockSelect`: `true` while rectangular block selection mode is on: plain left-drag selects
    /// rectangular block regions across pages (`Ctrl+left-drag` also triggers block selection anytime).
    /// `fingerDraws`: `true` when a finger that isn't a resting hand draws ink rather than panning (H.8). `snapshot`: `true` while
    /// a drag on a page marks a region to render (O.23), sent as `snapshot` (`{page, rect}`, page points).
    /// `paperTexture`: how strongly the paper's grain shows over the pages (R.23), 0 to 1; anything else: none.
    /// `guide`: the drawing guide on the page (H.11), `ruler`, `protractor` or `compass`; anything else: none. A stroke
    /// along it is drawn true on it, and `guideReading` (`{kind, length, degrees, radius}`) says what it measures.
    /// `zoomPad`: `true` while the zoom writing pad is on (H.12): pen strokes in the bottom third land small in a box
    /// on the page, which steps on by itself.
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
            "reflowStyle" => {
                let style = value.map_or_else(ReflowStyle::default, |val| ReflowStyle {
                    use_document_css: val
                        .get("useDocumentCss")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(true),
                    user_css: val
                        .get("userCss")
                        .and_then(JsonValue::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                });
                self.set_reflow_style(style);
            }
            "edits" => self.edited(),
            "annotationColors" => {
                self.set_annotation_colors(value.map(Colors::from_json).unwrap_or_default())
            }
            "drawing" => self.set_drawing(text.and_then(Kind::parse)),
            "pen" => self.set_pen(value.map_or_else(Pen::default, pen)),
            "eraser" => self.set_eraser(text.and_then(Eraser::from_name)),
            "lasso" => self.set_lasso(value.and_then(JsonValue::as_bool).unwrap_or(false)),
            "blockSelect" => {
                self.set_block_select(value.and_then(JsonValue::as_bool).unwrap_or(false))
            }
            "fingerDraws" => {
                self.set_finger_draws(value.and_then(JsonValue::as_bool).unwrap_or(false))
            }
            "snapshot" => self.set_snapshot(value.and_then(JsonValue::as_bool).unwrap_or(false)),
            "guide" => self.set_guide(text),
            "zoomPad" => self.set_zoom_pad(value.and_then(JsonValue::as_bool).unwrap_or(false)),
            "paperTexture" => {
                self.set_paper_texture(value.and_then(JsonValue::as_f64).map_or(0.0, |s| s as f32))
            }
            "margin" => self.set_margin(value.and_then(JsonValue::as_bool).unwrap_or(false)),
            "externalMarkbar" => {
                self.set_external_markbar(value.and_then(JsonValue::as_bool).unwrap_or(false))
            }
            "selected" => self.set_selected(value),
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
            "hoverMaxWidthPct" => self.set_hover_max_width_pct(
                value
                    .and_then(JsonValue::as_f64)
                    .map_or(DEFAULT_HOVER_MAX_WIDTH_PCT, |pct| pct as f32),
            ),
            "path" => self.open(text.map(Path::new)),
            "background" => match text.and_then(parse_rgb) {
                Some(rgb) => self.set_background(rgb),
                None => log::warn!("page canvas: `background` is not #rrggbb: {value:?}"),
            },
            // Handled by platform hosts that manage a native child window's visibility directly.
            "visible" => {}
            _ => log::debug!("page canvas: unknown prop `{name}`"),
        }
    }

    /// `zoomIn`, `zoomOut`, `actualSize`, `setZoom` (`{"percent": n}`, 10 to 6400), `toggleZoom`,
    /// `zoomToSelection`, `screenDown`, `screenUp`, `stepLeft`, `stepRight` (R.3), `historyBack`, `historyForward`, `firstPage`,
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
            "setZoom" => match args.get("percent").and_then(JsonValue::as_f64) {
                Some(pct) => canvas.set_zoom_percent(pct as f32),
                None => log::warn!("page canvas: setZoom without a numeric percent: {args}"),
            },
            "toggleZoom" => canvas.toggle_zoom(),
            "zoomToSelection" => canvas.zoom_to_selection(),
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
            "blockHighlight" => canvas.mark(Markup::Highlight),
            "underline" => canvas.mark(Markup::Underline),
            "squiggly" => canvas.mark(Markup::Squiggly),
            "strikeOut" => canvas.mark(Markup::StrikeOut),
            "replaceText" => canvas.propose_replace(),
            "insertText" => canvas.propose_insert(),
            "noteSelection" => canvas.note_selection(),
            "markWithColor" => match mark_with_color_args(args) {
                Some((kind, color, mode)) => canvas.mark_with_color(kind, color, mode),
                None => log::warn!("page canvas: markWithColor without a valid kind: {args}"),
            },
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
            "leaveModes" => {
                canvas.leave_block_select_mode();
                canvas.leave_modes();
            }
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
            "reload" => canvas.reload(),
            _ => match ViewCommand::parse(name) {
                Some(command) => canvas.view_command(command),
                None => log::warn!("page canvas: unknown command `{name}`"),
            },
        }
    }
}

/// Parses `markWithColor` command arguments: `{kind, color?, note?}`.
fn mark_with_color_args(args: &JsonValue) -> Option<(Markup, Option<u32>, NoteMode)> {
    let (kind, mut mode) = match args.get("kind").and_then(JsonValue::as_str)? {
        "highlight" => (Markup::Highlight, NoteMode::Plain),
        "underline" => (Markup::Underline, NoteMode::Plain),
        "squiggly" => (Markup::Squiggly, NoteMode::Plain),
        "strikeOut" => (Markup::StrikeOut, NoteMode::Plain),
        "note" => (Markup::Highlight, NoteMode::OpenNote),
        _ => return None,
    };
    if args.get("note").and_then(JsonValue::as_bool) == Some(true) {
        mode = NoteMode::OpenNote;
    }
    let color = args
        .get("color")
        .and_then(JsonValue::as_str)
        .and_then(parse_rgb);
    Some((kind, color, mode))
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

/// The `pen` prop, `{kind, size, color, pressure}` (H.2, H.7): the kind by name, the size in points, the colour as
/// `#rrggbb` and what the width curve's exponent is multiplied by; what is missing or unknown comes as the kind has it.
fn pen(value: &JsonValue) -> Pen {
    let kind = value
        .get("kind")
        .and_then(JsonValue::as_str)
        .and_then(PenKind::parse)
        .unwrap_or_default();
    let size = value.get("size").and_then(JsonValue::as_f64);
    let mut pen = kind.pen(size.map_or(DEFAULT_SIZE, |size| size as f32));
    if let Some(color) = value
        .get("color")
        .and_then(JsonValue::as_str)
        .and_then(parse_rgb)
    {
        pen.color = color;
    }
    if let Some(pressure) = value.get("pressure").and_then(JsonValue::as_f64) {
        pen.curve.exponent *= (pressure as f32).clamp(0.25, 4.0);
    }
    pen
}

#[cfg(test)]
mod tests {
    use super::{
        Case, HitId, Marks, Markup, NoteMode, Options, PageColors, PagePoint, Pictures, Style,
        Words, bookmarks, find_options, hit_id, mark_with_color_args, page_colors, parse_rgb, pen,
        ruler_style,
    };

    #[test]
    fn parses_mark_with_color_args() {
        assert_eq!(
            mark_with_color_args(&serde_json::json!({
                "kind": "highlight",
                "color": "#ffca28",
                "note": false
            })),
            Some((Markup::Highlight, Some(0xffca28), NoteMode::Plain))
        );
        assert_eq!(
            mark_with_color_args(&serde_json::json!({ "kind": "note" })),
            Some((Markup::Highlight, None, NoteMode::OpenNote))
        );
        assert_eq!(
            mark_with_color_args(&serde_json::json!({ "kind": "underline", "color": "#ef4444" })),
            Some((Markup::Underline, Some(0xef4444), NoteMode::Plain))
        );
        assert_eq!(
            mark_with_color_args(&serde_json::json!({ "kind": "unknown" })),
            None
        );
    }

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

    #[test]
    fn the_pen_prop_takes_a_favourite_s_colour_and_pressure() {
        use ecclesia_ink::pens::PenKind;
        let plain = PenKind::Pencil.pen(3.0);
        let read = pen(&serde_json::json!({
            "kind": "pencil", "size": 3.0, "color": "#aa3300", "pressure": 1.6,
        }));
        assert_eq!(read.kind, PenKind::Pencil);
        assert_eq!(read.color, 0xaa3300);
        assert!((read.curve.exponent - plain.curve.exponent * 1.6).abs() < 1e-5);
        let bare = pen(&serde_json::json!({"kind": "pencil", "size": 3.0}));
        assert_eq!(bare.color, plain.color, "the kind's own colour");
    }
}
