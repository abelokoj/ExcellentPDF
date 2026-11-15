//! The `excellent.page-canvas` NSH component (plan D5, K.8): the canvas window, its props and its commands.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use nsh_core::JsonValue;
use nsh_platform_windows::{ComponentContext, NativeComponent};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, SetWindowTextW};
use windows::core::HSTRING;

use crate::canvas::{Canvas, ZoomChange};
use crate::saved::SavedView;
use crate::window;
use excellent_canvas::viewport::Side;
use excellent_core::darkmode::{PageColors, Pictures};
use excellent_core::view::ViewCommand;
use excellent_features::read::autoscroll::Change;
use excellent_features::read::focus::Scope;
use excellent_features::read::ruler::{Step, Style};
use excellent_mupdf::PagePoint;

/// The component's name, as the app's `native(...)` element uses it.
pub const NAME: &str = "excellent.page-canvas";

/// A page canvas in an NSH view.
pub(crate) struct PageCanvas {
    hwnd: HWND,
    canvas: Rc<RefCell<Canvas>>,
}

impl PageCanvas {
    /// A new canvas, or `None` (logged) when Windows can't make its window.
    pub(crate) fn new(context: ComponentContext) -> Option<Self> {
        match window::create() {
            Ok((hwnd, canvas)) => {
                canvas.borrow_mut().set_events(context);
                Some(Self { hwnd, canvas })
            }
            Err(err) => {
                log::error!("cannot make the page canvas window: {err}");
                None
            }
        }
    }
}

impl NativeComponent for PageCanvas {
    fn window(&self) -> Option<HWND> {
        Some(self.hwnd)
    }

    /// `path`: the document shown (a string; absent or not a string: none). `label`: the accessible name.
    /// `background`: the colour behind the pages, `#rrggbb` (anything else: unchanged). `readingRuler`: the reading
    /// ruler's look, `{height, color, opacity, invert}` (each missing or wrong one: its default). `focusScope`:
    /// `"paragraph"` or `"line"`. `autoScrollSpeed`: pixels a second at 96 dpi. `pageColors`: the colours the pages
    /// are drawn in (R.14), `{ink, paper, link, pictures}` with `#rrggbb` colours (`link` optional) and `pictures`
    /// `"keep"` or `"recolor"`; `null`, or without a valid `ink` and `paper`: the document's own. `reflowFontSize`:
    /// a reflowable book's text size in points (R.28); `null`: the default.
    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        let text = value.and_then(JsonValue::as_str);
        match name {
            "readingRuler" => self
                .canvas
                .borrow_mut()
                .set_ruler_style(value.map_or_else(Style::default, ruler_style)),
            "focusScope" => self
                .canvas
                .borrow_mut()
                .set_focus_scope(text.and_then(Scope::parse).unwrap_or_default()),
            "autoScrollSpeed" => {
                if let Some(speed) = value.and_then(JsonValue::as_f64) {
                    self.canvas.borrow_mut().set_auto_scroll_speed(speed as f32);
                }
            }
            "pageColors" => self
                .canvas
                .borrow_mut()
                .set_page_colors(value.and_then(page_colors)),
            "reflowFontSize" => self
                .canvas
                .borrow_mut()
                .set_reflow_font_size(value.and_then(JsonValue::as_f64).map(|size| size as f32)),
            "path" => self.canvas.borrow_mut().open(text.map(Path::new)),
            "background" => match text.and_then(parse_rgb) {
                Some(rgb) => self.canvas.borrow_mut().set_background(rgb),
                None => log::warn!("page canvas: `background` is not #rrggbb: {value:?}"),
            },
            "label" => {
                // SAFETY: a plain call on our live window; the string lives for it.
                if let Err(err) =
                    unsafe { SetWindowTextW(self.hwnd, &HSTRING::from(text.unwrap_or(""))) }
                {
                    log::warn!("cannot name the page canvas window: {err}");
                }
            }
            _ => log::debug!("page canvas: unknown prop `{name}`"),
        }
    }

    /// `zoomIn`, `zoomOut`, `actualSize`, `screenDown`, `screenUp`, `stepLeft`, `stepRight` (R.3), `historyBack`, `historyForward`, `firstPage`,
    /// `lastPage`, `goToPage` (`{"index": n}`, from 0), `setView` (a `view` event's data; with only a `page`: that
    /// page), `goToPoint` (`{page, x, y, zoom}`, page points; `zoom` may be `null`), `copy`, `selectAll` (the
    /// reader's page), `readerPlace` (B.1, B.4, B.6: answered by the event of that name), and the view
    /// commands [`ViewCommand::parse`] knows (layouts, rotation, page box, fit modes). The reading aids (R.19-R.21):
    /// `toggleRuler`, `toggleFocus`, `lineDown`, `lineUp`, `toggleAutoScroll`, `autoScrollPause` (without
    /// auto-scroll: a screen down), `autoScrollFaster`, `autoScrollSlower`, `leaveModes`.
    ///
    /// Events: `pages` (`{count, labels}`, labels `null` when the pages go by number) when a document opens or
    /// closes, `page` (`{index}`) when the reader's page changes, `copy` (`{text}`) with the selected text,
    /// `autoScrollSpeed` (`{speed}`) when the reader changes it, `edge` (`{at}`) when the mouse reaches or
    /// leaves the canvas's top edge, `press` when the reader presses on the pages, `view` (the place, zoom,
    /// layout and rotation; see [`crate::saved`]) when the view settles somewhere new, and `readerPlace`
    /// (`{page, x, y, zoom, title}`, see [`Canvas::reader_place`]) when the app asked for it.
    fn command(&mut self, name: &str, args: &JsonValue) {
        let mut canvas = self.canvas.borrow_mut();
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
            "goToPage" => match args.get("index").and_then(JsonValue::as_u64) {
                Some(index) => canvas.go_to_page(usize::try_from(index).unwrap_or(usize::MAX)),
                None => log::warn!("page canvas: goToPage without an index: {args}"),
            },
            "goToPoint" => match go_to_point_args(args) {
                Some((page, point, zoom)) => canvas.go_to_point(page, point, zoom),
                None => log::warn!("page canvas: goToPoint without a page and point: {args}"),
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

impl Drop for PageCanvas {
    fn drop(&mut self) {
        // SAFETY: our window, destroyed once; WM_NCDESTROY releases the canvas reference it holds.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
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

/// `#rrggbb` as `0xRRGGBB`.
fn parse_rgb(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::{PageColors, Pictures, Style, page_colors, parse_rgb, ruler_style};

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
