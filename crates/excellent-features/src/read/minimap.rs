//! The mini-map (plan R.17), the model: a narrow strip down the pages' right edge that stands for the whole document,
//! with a tick where each row of pages starts, a thumb over the part on screen, and coloured marks for highlights,
//! notes, bookmarks and search hits. Pressing or dragging on it scrolls there.
//!
//! The strip runs top to bottom in reading order whatever the layout: a horizontal layout's pages go down it too,
//! and a right-to-left one is turned round. Positions "along" are layout coordinates on the layout's axis.
//!
//! Each kind of mark has its own lane, so they are told apart by place as well as by colour: highlights on the left,
//! notes in the middle, search hits on the right, bookmarks across the whole strip.

/// The strip's width, in pixels at 96 dpi.
pub const WIDTH: f32 = 12.0;
/// The thumb's least height, in pixels at 96 dpi: it stays easy to see and to grab in a long document.
pub const MIN_THUMB: f32 = 16.0;
/// A mark's least height, in pixels at 96 dpi.
pub const MIN_MARK: f32 = 2.0;
/// The current search hit's least height, in pixels at 96 dpi: taller than the other hits.
pub const MIN_CURRENT: f32 = 4.0;
/// Ticks closer than this, in pixels at 96 dpi, are left out: in a long document they would be a grey smear.
pub const MIN_TICK_GAP: f32 = 3.0;

/// What a mark stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkKind {
    /// A highlight, underline, squiggly underline or strikeout.
    Highlight,
    /// Any other annotation: a note, a text box, a shape, ink.
    Note,
    /// A bookmark.
    Bookmark,
    /// A search hit.
    Hit,
    /// The search hit the reader is on.
    CurrentHit,
}

impl MarkKind {
    /// The lane it is drawn in.
    pub fn lane(self) -> Lane {
        match self {
            Self::Highlight => Lane::Left,
            Self::Note => Lane::Middle,
            Self::Bookmark => Lane::Across,
            Self::Hit | Self::CurrentHit => Lane::Right,
        }
    }

    /// Its least height, in points.
    fn min_height(self) -> f32 {
        match self {
            Self::CurrentHit => MIN_CURRENT,
            _ => MIN_MARK,
        }
    }
}

/// Where across the strip a mark is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    /// The left third.
    Left,
    /// The middle third.
    Middle,
    /// The right third.
    Right,
    /// The whole width.
    Across,
}

impl Lane {
    /// Its left and right edges in a strip from `x0` that is `width` wide.
    pub fn span(self, x0: f32, width: f32) -> (f32, f32) {
        let third = width / 3.0;
        match self {
            Self::Left => (x0, x0 + third),
            Self::Middle => (x0 + third, x0 + 2.0 * third),
            Self::Right => (x0 + 2.0 * third, x0 + width),
            Self::Across => (x0, x0 + width),
        }
    }
}

/// Something to mark: where it starts and ends along the layout, what it is, and its colour (`0xRRGGBB`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    /// Where it starts along the layout's axis.
    pub from: f32,
    /// Where it ends along the layout's axis.
    pub to: f32,
    /// What it stands for.
    pub kind: MarkKind,
    /// Its colour.
    pub color: u32,
}

/// A mark placed on the strip: its top and bottom in pixels, its lane and its colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Its top.
    pub y0: f32,
    /// Its bottom.
    pub y1: f32,
    /// Its lane.
    pub lane: Lane,
    /// Its colour.
    pub color: u32,
}

/// Which way reading goes along the layout's axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// The way the axis goes: down, or left to right.
    #[default]
    Forward,
    /// Against it: a right-to-left layout.
    Reversed,
}

/// The strip's scale: the document's extent along the layout, mapped onto the strip's height in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    start: f32,
    end: f32,
    order: Order,
    top: f32,
    length: f32,
    /// Device pixels per pixel at 96 dpi.
    ppp: f32,
}

impl Scale {
    /// The extent `start` to `end` along the layout, read in `order`, on a strip from `top` that is `length` pixels
    /// long, on a screen with `ppp` device pixels per pixel at 96 dpi. `None` for an empty extent or strip.
    pub fn new(
        start: f32,
        end: f32,
        order: Order,
        top: f32,
        length: f32,
        ppp: f32,
    ) -> Option<Self> {
        let usable = end > start
            && length > 0.0
            && ppp > 0.0
            && [start, end, top, length, ppp].iter().all(|v| v.is_finite());
        usable.then_some(Self {
            start,
            end,
            order,
            top,
            length,
            ppp,
        })
    }

    /// The strip's top and bottom, in pixels.
    pub fn strip(&self) -> (f32, f32) {
        (self.top, self.top + self.length)
    }

    /// Where `along` lands on the strip, in pixels.
    pub fn to_strip(&self, along: f32) -> f32 {
        let share = ((along - self.start) / (self.end - self.start)).clamp(0.0, 1.0);
        let share = match self.order {
            Order::Forward => share,
            Order::Reversed => 1.0 - share,
        };
        self.top + share * self.length
    }

    /// The place along the layout that strip pixel `y` stands for.
    pub fn to_layout(&self, y: f32) -> f32 {
        let share = ((y - self.top) / self.length).clamp(0.0, 1.0);
        let share = match self.order {
            Order::Forward => share,
            Order::Reversed => 1.0 - share,
        };
        self.start + share * (self.end - self.start)
    }

    /// The top and bottom of the strip between `from` and `to` along the layout, at least `min` (at 96 dpi) tall and
    /// kept on the strip.
    fn span(&self, from: f32, to: f32, min: f32) -> (f32, f32) {
        let (a, b) = (self.to_strip(from), self.to_strip(to));
        let (mut y0, mut y1) = (a.min(b), a.max(b));
        let least = (min * self.ppp).min(self.length);
        if y1 - y0 < least {
            let middle = (y0 + y1) / 2.0;
            y0 = middle - least / 2.0;
            y1 = middle + least / 2.0;
        }
        let (top, bottom) = self.strip();
        if y0 < top {
            (top, top + (y1 - y0))
        } else if y1 > bottom {
            (bottom - (y1 - y0), bottom)
        } else {
            (y0, y1)
        }
    }

    /// The thumb over the part on screen, `from` to `to` along the layout: its top and bottom, in pixels.
    pub fn thumb(&self, from: f32, to: f32) -> (f32, f32) {
        self.span(from, to, MIN_THUMB)
    }

    /// Whether the part on screen, `from` to `to`, is all of the document.
    pub fn shows_all(&self, from: f32, to: f32) -> bool {
        from <= self.start && to >= self.end
    }

    /// The ticks for rows of pages starting at `starts` (in order along the layout), in pixels; a tick too close to
    /// the last one kept, or at the strip's very top, is left out.
    pub fn ticks(&self, starts: impl IntoIterator<Item = f32>) -> Vec<f32> {
        let gap = MIN_TICK_GAP * self.ppp;
        let mut ticks: Vec<f32> = Vec::new();
        let mut placed: Vec<f32> = starts
            .into_iter()
            .map(|along| self.to_strip(along))
            .collect();
        placed.sort_by(f32::total_cmp);
        let mut last = self.top;
        for y in placed {
            if y - last >= gap {
                ticks.push(y.round());
                last = y;
            }
        }
        ticks
    }

    /// `marks` placed on the strip, each at least its kind's least height. Marks of one lane and colour that touch
    /// are drawn as one, so a page of hits costs a few rectangles, not hundreds. Bookmarks come first, then
    /// highlights, notes and hits, so the current hit is drawn last, over the others.
    pub fn place(&self, marks: &[Mark]) -> Vec<Placed> {
        let rank = |kind: MarkKind| match kind {
            MarkKind::Bookmark => 0,
            MarkKind::Highlight => 1,
            MarkKind::Note => 2,
            MarkKind::Hit => 3,
            MarkKind::CurrentHit => 4,
        };
        let mut spans: Vec<(u8, Placed)> = marks
            .iter()
            .filter(|mark| mark.from.is_finite() && mark.to.is_finite())
            .map(|mark| {
                let (y0, y1) = self.span(mark.from, mark.to, mark.kind.min_height());
                let placed = Placed {
                    y0,
                    y1,
                    lane: mark.kind.lane(),
                    color: mark.color & 0xff_ffff,
                };
                (rank(mark.kind), placed)
            })
            .collect();
        spans.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb)
                .then(a.color.cmp(&b.color))
                .then(a.y0.total_cmp(&b.y0))
        });
        let mut placed: Vec<(u8, Placed)> = Vec::with_capacity(spans.len());
        for (rank, span) in spans {
            match placed.last_mut() {
                Some((last_rank, last))
                    if *last_rank == rank
                        && last.lane == span.lane
                        && last.color == span.color
                        && span.y0 <= last.y1 =>
                {
                    last.y1 = last.y1.max(span.y1);
                }
                _ => placed.push((rank, span)),
            }
        }
        placed.into_iter().map(|(_, span)| span).collect()
    }

    /// Where the part on screen should start along the layout for the pointer at strip pixel `y`, when it holds the
    /// thumb `grab` pixels below the thumb's middle (0 for a press beside the thumb, which centres the screen there);
    /// `shown` is the length on screen along the layout.
    pub fn scroll_to(&self, y: f32, grab: f32, shown: f32) -> f32 {
        let middle = self.to_layout(y - grab);
        let latest = (self.end - shown).max(self.start);
        (middle - shown / 2.0).clamp(self.start, latest)
    }
}

/// The strip's colours, made from the colour behind the pages so it suits every theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The strip's own fill.
    pub track: u32,
    /// The ticks between rows of pages.
    pub tick: u32,
    /// The thumb: laid over the strip translucently.
    pub thumb: u32,
    /// Whether the background is dark: marks are then drawn as they are, and on light ones a little deeper so pale
    /// colours (a yellow highlight) still show.
    dark: bool,
}

impl Palette {
    /// The palette for pages shown on `background` (`0xRRGGBB`).
    pub fn for_background(background: u32) -> Self {
        let dark = luminance(background) < 0.5;
        let toward = if dark { 0xff_ffff } else { 0x00_0000 };
        Self {
            track: mix(background, toward, 0.06),
            tick: mix(background, toward, 0.22),
            thumb: toward,
            dark,
        }
    }

    /// The colour to draw a mark of colour `color` in.
    pub fn ink(&self, color: u32) -> u32 {
        if self.dark {
            color & 0xff_ffff
        } else {
            mix(color, 0x00_0000, 0.2)
        }
    }
}

/// Relative brightness of `rgb`, from 0 (black) to 1 (white), by the channels' weights in sRGB (unlinearized: close
/// enough to choose light or dark).
fn luminance(rgb: u32) -> f32 {
    let channel = |shift: u32| ((rgb >> shift) & 0xff) as f32 / 255.0;
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// `from` moved `amount` (0 to 1) of the way to `to`, channel by channel.
fn mix(from: u32, to: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let a = ((from >> shift) & 0xff) as f32;
        let b = ((to >> shift) & 0xff) as f32;
        ((a + (b - a) * amount).round() as u32).min(255) << shift
    };
    channel(16) | channel(8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(order: Order) -> Scale {
        // 1,000 points of document on a 500-pixel strip from y 10, at 96 dpi.
        Scale::new(0.0, 1000.0, order, 10.0, 500.0, 1.0).expect("a usable scale")
    }

    #[test]
    fn the_strip_stands_for_the_whole_document_either_way_round() {
        let forward = scale(Order::Forward);
        assert_eq!(forward.to_strip(0.0), 10.0);
        assert_eq!(forward.to_strip(500.0), 260.0);
        assert_eq!(
            forward.to_strip(2000.0),
            510.0,
            "past the end stays on the strip"
        );
        assert_eq!(forward.to_layout(260.0), 500.0);
        let reversed = scale(Order::Reversed);
        assert_eq!(
            reversed.to_strip(1000.0),
            10.0,
            "a right-to-left layout's first page is at its right"
        );
        assert_eq!(reversed.to_layout(10.0), 1000.0);
        assert!(
            Scale::new(5.0, 5.0, Order::Forward, 0.0, 100.0, 1.0).is_none(),
            "an empty document"
        );
        assert!(
            Scale::new(0.0, 5.0, Order::Forward, 0.0, 0.0, 1.0).is_none(),
            "no room for a strip"
        );
    }

    #[test]
    fn the_thumb_covers_the_screen_and_stays_big_enough_to_grab() {
        let s = scale(Order::Forward);
        assert_eq!(s.thumb(100.0, 300.0), (60.0, 160.0));
        let (y0, y1) = s.thumb(500.0, 501.0);
        assert_eq!(
            y1 - y0,
            MIN_THUMB,
            "a long document's thumb is still {MIN_THUMB} pixels"
        );
        assert_eq!(s.thumb(0.0, 1.0).0, 10.0, "kept on the strip at the start");
        assert_eq!(s.thumb(999.0, 1000.0).1, 510.0, "and at the end");
        assert!(s.shows_all(-8.0, 1008.0));
        assert!(!s.shows_all(0.0, 999.0));
    }

    #[test]
    fn ticks_too_close_together_are_left_out() {
        let s = scale(Order::Forward);
        assert_eq!(
            s.ticks([0.0, 100.0, 200.0]),
            vec![60.0, 110.0],
            "none at the very top"
        );
        // A thousand rows on 500 pixels: one tick at most every MIN_TICK_GAP pixels.
        let ticks = s.ticks((0..1000).map(|row| row as f32));
        assert!(
            ticks
                .windows(2)
                .all(|pair| pair[1] - pair[0] >= MIN_TICK_GAP - 1.0)
        );
        assert!(ticks.len() <= (500.0 / MIN_TICK_GAP) as usize);
    }

    #[test]
    fn marks_keep_their_lanes_and_touching_ones_merge() {
        let s = scale(Order::Forward);
        let mark = |from: f32, kind: MarkKind, color: u32| Mark {
            from,
            to: from + 1.0,
            kind,
            color,
        };
        let placed = s.place(&[
            mark(100.0, MarkKind::Hit, 0xf29900),
            mark(101.0, MarkKind::Hit, 0xf29900),
            mark(102.0, MarkKind::CurrentHit, 0xe8710a),
            mark(400.0, MarkKind::Highlight, 0xffff00),
            mark(600.0, MarkKind::Bookmark, 0x1a73e8),
            mark(800.0, MarkKind::Note, 0xffff00),
        ]);
        let lanes: Vec<Lane> = placed.iter().map(|p| p.lane).collect();
        assert_eq!(
            lanes,
            vec![
                Lane::Across,
                Lane::Left,
                Lane::Middle,
                Lane::Right,
                Lane::Right
            ],
            "bookmarks first and the current hit last; the two touching hits are one"
        );
        let current = placed.last().expect("the current hit");
        assert_eq!(
            current.y1 - current.y0,
            MIN_CURRENT,
            "the current hit is taller"
        );
        assert!(placed.iter().all(|p| p.y1 - p.y0 >= MIN_MARK));
    }

    #[test]
    fn pressing_centres_the_screen_and_dragging_keeps_the_grip() {
        let s = scale(Order::Forward);
        // 200 points on screen; a press at y 260 (the middle) puts the middle of the screen there.
        assert_eq!(s.scroll_to(260.0, 0.0, 200.0), 400.0);
        // Held 20 pixels below the thumb's middle, the screen's middle is 20 pixels above the pointer.
        assert_eq!(s.scroll_to(260.0, 20.0, 200.0), 360.0);
        assert_eq!(s.scroll_to(10.0, 0.0, 200.0), 0.0, "not before the start");
        assert_eq!(s.scroll_to(510.0, 0.0, 200.0), 800.0, "not past the end");
        let reversed = scale(Order::Reversed);
        assert_eq!(
            reversed.scroll_to(10.0, 0.0, 200.0),
            800.0,
            "the first pages of a right-to-left layout"
        );
    }

    #[test]
    fn the_palette_follows_the_background() {
        let light = Palette::for_background(0xf1f3f4);
        let dark = Palette::for_background(0x202124);
        assert!(
            luminance(light.tick) < luminance(light.track),
            "ticks darker on light"
        );
        assert!(
            luminance(dark.tick) > luminance(dark.track),
            "lighter on dark"
        );
        assert_eq!(dark.ink(0xffff00), 0xffff00);
        assert!(
            luminance(light.ink(0xffff00)) < luminance(0xffff00),
            "pale marks deepened on light"
        );
    }
}
