//! The reading aids on the canvas: the reading ruler (plan R.20), focus mode (R.19), auto-scroll and middle-click
//! scrolling (R.21). Their models are `excellent-features`' `read::{ruler, focus, autoscroll}`; this holds their state
//! on the canvas, moves them with the keys, the mouse and the frame clock, and draws them.
//!
//! The ruler and focus mode share one reading line: a line of a page's text (K.13), or a free height where there is
//! no text. The ruler's band is drawn darkest-wins under the ink, so text under it stays black; focus mode and the
//! inverted ruler lay the page area's colour over everything outside what they keep lit.

use std::time::Instant;

use excellent_canvas::Rect;
use excellent_features::read::autoscroll::{self, AutoScroll, Change, Pan};
use excellent_features::read::focus::{self, Scope};
use excellent_features::read::ruler::{self, Step, Style};
use excellent_mupdf::{PagePoint, PageRect};

use super::Canvas;
use crate::gfx::{self, Gfx, Wash};
use crate::placement::Placement;

/// Where the reading line starts, as a share of the window's height from the top (Enhanced's `yFrac`).
const START_AT: f32 = 0.40;
/// What the arrow keys scroll with neither the ruler nor focus mode on, in pixels at 96 dpi.
const LINE_SCROLL: f32 = 40.0;
/// Room above and below the line in the band, and around focus mode's paragraph, in pixels at 96 dpi.
const PAD: f32 = 4.0;
/// Room beside the text in the band, in pixels at 96 dpi.
const SIDE: f32 = 8.0;
/// The band's and the lit area's corner radius, in pixels at 96 dpi.
const RADIUS: f32 = 6.0;
/// How strongly focus mode and the inverted ruler cover the rest of the page (Enhanced's invert mask, `0x99`).
// Enhanced dims with black; this dims with the page area's colour, so the dimmed page matches the app's look.
const DIM: f32 = 0x99 as f32 / 255.0;

/// The reading line.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Spot {
    /// Line `line` (an index into the page's [`excellent_mupdf::TextLayout::lines`]) of page `page`.
    Line { page: usize, line: usize },
    /// Client pixel height `y`, where no line is: off the pages, on a page without text, or text not read yet.
    Free { y: f32 },
}

/// The reading aids' state on one canvas.
#[derive(Debug)]
pub(super) struct Reading {
    /// The ruler is on.
    ruler: bool,
    style: Style,
    /// Focus mode is on.
    focus: bool,
    scope: Scope,
    /// The reading line, while the ruler or focus mode is on.
    spot: Option<Spot>,
    /// Auto-scroll's speed, in pixels a second at 96 dpi: the setting, and the last one chosen.
    speed: f32,
    auto: Option<AutoScroll>,
    pan: Option<Pan>,
    /// When the last frame that moved the pages was drawn; `None` when nothing moves.
    last_frame: Option<Instant>,
    /// Where the mouse is, in client pixels; `None` outside the canvas.
    mouse: Option<(f32, f32)>,
    /// The mouse is at the window's top edge, as last reported.
    at_edge: bool,
}

impl Default for Reading {
    fn default() -> Self {
        Self {
            ruler: false,
            style: Style::default(),
            focus: false,
            scope: Scope::default(),
            spot: None,
            speed: autoscroll::DEFAULT_SPEED,
            auto: None,
            pan: None,
            last_frame: None,
            mouse: None,
            at_edge: false,
        }
    }
}

impl Reading {
    /// Whether anything shows the reading line.
    fn wants_line(&self) -> bool {
        self.ruler || self.focus
    }

    /// Whether the pages are moving on their own.
    fn running(&self) -> bool {
        self.auto.as_ref().is_some_and(|auto| !auto.is_paused()) || self.pan.is_some()
    }

    /// Whether middle-click scrolling is on.
    pub(super) fn is_panning(&self) -> bool {
        self.pan.is_some()
    }

    /// Whether the pages around the screen's need their text read: the reading line steps onto them.
    pub(super) fn reads_ahead(&self) -> bool {
        self.wants_line()
    }

    /// A document closed: the reading line goes and the pages stop moving. The modes stay on.
    pub(super) fn reset(&mut self) {
        self.spot = None;
        self.auto = None;
        self.pan = None;
        self.last_frame = None;
    }
}

/// What the reading aids draw in a frame, worked out before drawing starts.
pub(super) struct Aids {
    /// The ruler's band and its colour, washed over the page under the ink.
    band: Option<(Rect, u32)>,
    /// How the band lies over the page.
    wash: Wash,
    /// The areas kept lit, with the page area's colour over the rest of the screen, one layer each.
    lit: Vec<Rect>,
    screen: Rect,
    radius: f32,
    shade: u32,
}

impl Aids {
    /// Draws what goes under the ink: the band.
    pub(super) fn draw_under(&self, gfx: &Gfx) -> windows::core::Result<()> {
        match &self.band {
            Some((band, rgb)) => gfx.band(&gfx::rect(band), self.radius, *rgb, self.wash),
            None => Ok(()),
        }
    }

    /// Draws what goes over everything: the dimmed page around what is lit.
    pub(super) fn draw_over(&self, gfx: &Gfx) -> windows::core::Result<()> {
        for lit in &self.lit {
            gfx.dim_outside(
                &gfx::rect(&self.screen),
                &gfx::rect(lit),
                self.radius,
                self.shade,
                DIM,
            )?;
        }
        Ok(())
    }
}

/// `rect` grown by `x` on each side and `y` above and below.
fn grown(rect: Rect, x: f32, y: f32) -> Rect {
    Rect {
        x0: rect.x0 - x,
        y0: rect.y0 - y,
        x1: rect.x1 + x,
        y1: rect.y1 + y,
    }
}

impl Canvas {
    /// Device pixels to a pixel at 96 dpi.
    fn dpi_scale(&self) -> f32 {
        self.viewport.pixels_per_point() * 72.0 / 96.0
    }

    /// The screen.
    pub(super) fn screen(&self) -> Rect {
        Rect {
            x0: 0.0,
            y0: 0.0,
            x1: self.viewport.width(),
            y1: self.viewport.height(),
        }
    }

    /// The box around `rect` of page `page` on screen, in client pixels.
    pub(super) fn screen_box(&self, page: usize, rect: &PageRect) -> Option<Rect> {
        let placement = Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)?;
        let corners = [
            (rect.x0, rect.y0),
            (rect.x1, rect.y0),
            (rect.x1, rect.y1),
            (rect.x0, rect.y1),
        ]
        .map(|(x, y)| {
            let (x, y) = placement.to_layout(PagePoint { x, y });
            self.viewport.to_screen_point(x, y)
        });
        let mut screen = Rect {
            x0: f32::INFINITY,
            y0: f32::INFINITY,
            x1: f32::NEG_INFINITY,
            y1: f32::NEG_INFINITY,
        };
        for (x, y) in corners {
            screen.x0 = screen.x0.min(x);
            screen.y0 = screen.y0.min(y);
            screen.x1 = screen.x1.max(x);
            screen.y1 = screen.y1.max(y);
        }
        Some(screen)
    }

    /// The reading line under client pixel (`x`, `y`): the nearest line of the page there, or that height.
    fn spot_at(&self, x: f32, y: f32) -> Spot {
        self.page_point(x as i32, y as i32)
            .and_then(|(page, point)| {
                let line = self.texts.get(page)?.line_near(point)?;
                Some(Spot::Line { page, line })
            })
            .unwrap_or(Spot::Free { y })
    }

    /// The box of line `line` of page `page` on screen.
    fn line_box(&self, page: usize, line: usize) -> Option<Rect> {
        let bbox = self.texts.get(page)?.layout().lines.get(line)?.bbox;
        self.screen_box(page, &bbox)
    }

    /// The band's box on screen around the reading line: as wide as the line's paragraph, and at least as tall as
    /// the style asks.
    fn band_box(&self) -> Option<Rect> {
        let scale = self.dpi_scale();
        let height = self.reading.style.height * scale;
        let pad = PAD * scale;
        match self.reading.spot? {
            Spot::Line { page, line } => {
                let layout = self.texts.get(page)?.layout();
                let text_line = layout.lines.get(line)?;
                let line_box = self.screen_box(page, &text_line.bbox)?;
                let block = layout
                    .blocks
                    .get(text_line.block)
                    .and_then(|block| self.screen_box(page, &block.bbox))
                    .unwrap_or(line_box);
                let (top, bottom) = ruler::band(line_box.y0, line_box.y1, height, pad);
                let side = SIDE * scale;
                Some(Rect {
                    x0: block.x0.min(line_box.x0) - side,
                    y0: top,
                    x1: block.x1.max(line_box.x1) + side,
                    y1: bottom,
                })
            }
            Spot::Free { y } => {
                let (top, bottom) = ruler::band(y, y, height, pad);
                Some(Rect {
                    y0: top,
                    y1: bottom,
                    ..self.screen()
                })
            }
        }
    }

    /// The box focus mode keeps lit on screen: the reading line's paragraph or the line itself.
    fn lit_box(&self) -> Option<Rect> {
        let Some(Spot::Line { page, line }) = self.reading.spot else {
            return self.band_box();
        };
        let rect = focus::lit(self.texts.get(page)?.layout(), line, self.reading.scope)?;
        let scale = self.dpi_scale();
        Some(grown(
            self.screen_box(page, &rect)?,
            SIDE * scale,
            PAD * scale,
        ))
    }

    /// What the reading aids draw in this frame.
    pub(super) fn aids(&self) -> Aids {
        let reading = &self.reading;
        let band = reading.ruler.then(|| self.band_box()).flatten();
        let mut lit = Vec::new();
        if reading.focus
            && let Some(rect) = self.lit_box()
        {
            lit.push(rect);
        }
        if reading.style.invert {
            lit.extend(band);
        }
        Aids {
            band: band
                .filter(|_| !reading.style.invert)
                .map(|band| (band, reading.style.tint())),
            wash: self.wash(),
            lit,
            screen: self.screen(),
            radius: RADIUS * self.dpi_scale(),
            shade: self.background,
        }
    }

    /// Scrolls so the band is in view with its own height of room above and below it.
    fn keep_line_in_view(&mut self) {
        let Some(band) = self.band_box() else {
            return;
        };
        let height = self.viewport.height();
        let margin = if band.height() * 3.0 < height {
            band.height()
        } else {
            0.0
        };
        if band.y0 < margin {
            self.viewport.scroll_by(0.0, band.y0 - margin, &self.layout);
        } else if band.y1 > height - margin {
            self.viewport
                .scroll_by(0.0, band.y1 - (height - margin), &self.layout);
        }
    }

    /// The line a step goes to from line `line` of page `page`: on the page, or onto the next or previous page's
    /// first or last line. `None` when that page's text isn't read yet or it has none.
    fn line_after(&self, page: usize, line: usize, step: Step) -> Option<(usize, usize)> {
        let text = self.texts.get(page)?;
        if let Some(next) = ruler::step(text.layout(), line, step) {
            return Some((page, next));
        }
        let next_page = match step {
            Step::Next => page.checked_add(1).filter(|&p| p < self.layout.len()),
            Step::Previous => page.checked_sub(1),
        }?;
        let line = ruler::entry(self.texts.get(next_page)?.layout(), step)?;
        Some((next_page, line))
    }

    /// Picks the reading line where it starts: under the mouse if the ruler follows it there, else at
    /// [`START_AT`] of the screen's height. With nothing showing it, there is none.
    fn start_line(&mut self) {
        if !self.reading.wants_line() {
            self.reading.spot = None;
            return;
        }
        if self.reading.spot.is_some() {
            return;
        }
        let (x, y) = self
            .reading
            .mouse
            .filter(|_| self.reading.ruler)
            .unwrap_or((
                self.viewport.width() / 2.0,
                self.viewport.height() * START_AT,
            ));
        self.reading.spot = Some(self.spot_at(x, y));
    }

    /// Turns the reading ruler on or off (R.20).
    pub(crate) fn toggle_ruler(&mut self) {
        self.reading.ruler = !self.reading.ruler;
        self.start_line();
        self.invalidate();
    }

    /// Turns focus mode on or off (R.19).
    pub(crate) fn toggle_focus(&mut self) {
        self.reading.focus = !self.reading.focus;
        self.start_line();
        self.invalidate();
    }

    /// The ruler's look: the settings `reading_ruler_*`.
    pub(crate) fn set_ruler_style(&mut self, style: Style) {
        let style = style.clamped();
        if style != self.reading.style {
            self.reading.style = style;
            self.invalidate();
        }
    }

    /// What focus mode keeps lit: the setting `focus_scope`.
    pub(crate) fn set_focus_scope(&mut self, scope: Scope) {
        if scope != self.reading.scope {
            self.reading.scope = scope;
            self.invalidate();
        }
    }

    /// Auto-scroll's speed the next time it starts, in pixels a second at 96 dpi: the setting
    /// `auto_scroll_speed`.
    pub(crate) fn set_auto_scroll_speed(&mut self, speed: f32) {
        self.reading.speed = autoscroll::clamp_speed(speed);
    }

    /// The Down or Up arrow: moves the reading line one line in reading order, scrolling to keep it in view. With
    /// neither the ruler nor focus mode on, scrolls a line's worth.
    pub(crate) fn line_step(&mut self, step: Step) {
        let sign = match step {
            Step::Next => 1.0,
            Step::Previous => -1.0,
        };
        let scale = self.dpi_scale();
        if !self.reading.wants_line() {
            self.scroll_by(0.0, sign * LINE_SCROLL * scale);
            return;
        }
        let band = self.reading.style.height * scale;
        let screen = self.screen();
        let on_screen = match self.reading.spot {
            Some(Spot::Line { page, line }) => self
                .line_box(page, line)
                .filter(|b| b.intersect(&screen).is_some())
                .map(|_| (page, line)),
            _ => None,
        };
        match on_screen {
            Some((page, line)) => match self.line_after(page, line, step) {
                Some((page, line)) => self.reading.spot = Some(Spot::Line { page, line }),
                // The next page's text is on its way, or it has none: move on by a band, the line with the page.
                None => self.viewport.scroll_by(0.0, sign * band, &self.layout),
            },
            // The reader scrolled the line away, or there was none: pick one where the line was, or starts.
            None => {
                let y = match self.reading.spot {
                    Some(Spot::Free { y }) => y,
                    _ => screen.height() * START_AT,
                };
                let spot = self.spot_at(screen.width() / 2.0, y);
                if matches!(spot, Spot::Free { .. }) {
                    self.viewport.scroll_by(0.0, sign * band, &self.layout);
                }
                self.reading.spot = Some(spot);
            }
        }
        self.keep_line_in_view();
        self.invalidate();
    }

    /// Text came for pages near the screen: a free reading line finds its line.
    pub(super) fn text_arrived(&mut self) -> bool {
        let Some(Spot::Free { y }) = self.reading.spot else {
            return false;
        };
        let x = self
            .reading
            .mouse
            .filter(|_| self.reading.ruler)
            .map_or(self.viewport.width() / 2.0, |(x, _)| x);
        let spot = self.spot_at(x, y);
        self.reading.spot = Some(spot);
        spot != Spot::Free { y }
    }

    /// Starts auto-scroll at the last speed chosen, or stops it (R.21).
    pub(crate) fn toggle_auto_scroll(&mut self) {
        self.reading.auto = match self.reading.auto {
            Some(_) => None,
            None => Some(AutoScroll::new(self.reading.speed)),
        };
        self.reading.last_frame = None;
        self.invalidate();
    }

    /// Space: pauses or resumes auto-scroll; without it, scrolls a screen down.
    pub(crate) fn pause_or_screen_down(&mut self) {
        match &mut self.reading.auto {
            Some(auto) => {
                auto.toggle_pause();
                self.reading.last_frame = None;
                self.invalidate();
            }
            None => self.scroll_screen(1.0),
        }
    }

    /// Auto-scroll goes one speed step faster or slower, and tells the app the new speed to keep
    /// (`autoScrollSpeed`: `{speed}`). Nothing while it is off.
    pub(crate) fn change_auto_scroll_speed(&mut self, change: Change) {
        let Some(auto) = &mut self.reading.auto else {
            return;
        };
        let speed = auto.change_speed(change);
        self.reading.speed = speed;
        self.emit("autoScrollSpeed", serde_json::json!({ "speed": speed }));
    }

    /// Escape: the ruler, focus mode, auto-scroll and middle-click scrolling all stop.
    pub(crate) fn leave_modes(&mut self) {
        let reading = &mut self.reading;
        let was = reading.wants_line() || reading.auto.is_some() || reading.pan.is_some();
        reading.ruler = false;
        reading.focus = false;
        reading.reset();
        if was {
            self.invalidate();
        }
    }

    /// The middle button went down at client pixel (`x`, `y`): middle-click scrolling starts from there, or stops
    /// if it was on. Returns whether it is on, when the window holds the mouse so the pointer can leave it.
    pub(crate) fn middle_down(&mut self, x: i32, y: i32) -> bool {
        let at = (x as f32, y as f32);
        if self.reading.pan.take().is_none() {
            self.reading.pan = Some(Pan::new(at.0, at.1));
            self.reading.mouse = Some(at);
        }
        self.reading.last_frame = None;
        self.invalidate();
        self.reading.pan.is_some()
    }

    /// Stops middle-click scrolling. Returns whether it was on: then the press that stopped it does nothing else.
    pub(crate) fn stop_pan(&mut self) -> bool {
        let was = self.reading.pan.take().is_some();
        if was {
            self.invalidate();
        }
        was
    }

    /// Follows the mouse (`None`: it left the canvas): the ruler goes to the line under it, middle-click scrolling
    /// aims at it, and the app hears when it reaches or leaves the window's top edge (`edge`: `{at}`), where full
    /// screen brings back the title bar.
    pub(super) fn reading_mouse(&mut self, at: Option<(i32, i32)>) {
        let at = at.map(|(x, y)| (x as f32, y as f32));
        let moved = at != self.reading.mouse;
        // Middle-click scrolling holds the mouse: it reports positions past the window, and keeps the last one.
        if at.is_some() || self.reading.pan.is_none() {
            self.reading.mouse = at;
        }
        let edge = at.is_some_and(|(_, y)| focus::reveals(y, self.dpi_scale()));
        if edge != self.reading.at_edge {
            self.reading.at_edge = edge;
            self.emit("edge", serde_json::json!({ "at": edge }));
        }
        if moved
            && self.reading.ruler
            && !self.selecting
            && self.reading.pan.is_none()
            && let Some((x, y)) = at
        {
            let spot = self.spot_at(x, y);
            if self.reading.spot != Some(spot) {
                self.reading.spot = Some(spot);
                self.invalidate();
            }
        }
    }

    /// Moves auto-scroll and middle-click scrolling on by the time since the last frame. While auto-scroll runs,
    /// the band holds its height and the lines pass through it. Returns whether the pages go on moving, when the
    /// frame asks for the next one: frames come at the display's refresh rate then, and none while nothing moves.
    pub(super) fn advance_motion(&mut self) -> bool {
        if !self.reading.running() {
            self.reading.last_frame = None;
            return false;
        }
        let now = Instant::now();
        let seconds = self
            .reading
            .last_frame
            .replace(now)
            .map_or(0.0, |then| (now - then).as_secs_f32());
        let scale = self.dpi_scale();
        let held = self
            .reading
            .auto
            .as_ref()
            .filter(|_| self.reading.wants_line())
            .and_then(|_| self.band_box())
            .map(|band| ((band.x0 + band.x1) / 2.0, (band.y0 + band.y1) / 2.0));
        if let Some(auto) = &mut self.reading.auto {
            let pixels = auto.advance(seconds, scale);
            if pixels != 0.0 {
                let before = (self.viewport.visible(), self.viewport.band());
                self.viewport.scroll_forward(pixels, &self.layout);
                if (self.viewport.visible(), self.viewport.band()) == before {
                    // The last page's end: it stops there, paused, as Enhanced's does.
                    auto.pause();
                }
            }
        }
        if let (Some(pan), Some((x, y))) = (&mut self.reading.pan, self.reading.mouse) {
            let (dx, dy) = pan.advance(x, y, seconds);
            if dx != 0.0 || dy != 0.0 {
                self.viewport.scroll_by(dx, dy, &self.layout);
            }
        }
        if let Some((x, y)) = held {
            self.reading.spot = Some(self.spot_at(x, y));
        }
        self.reading.running()
    }
}
