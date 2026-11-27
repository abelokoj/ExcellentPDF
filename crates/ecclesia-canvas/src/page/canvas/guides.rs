//! Drawing guides on the canvas (H.11): the guide chosen (the `guide` prop: `ruler`, `protractor`, `compass` or
//! none) lies on the page in view, drawn over it. A mouse drag on its body moves it; one on its handle (the round
//! grip at the ruler's end, the protractor's base or the compass's pencil) turns it, or sets the compass's radius.
//! A pen stroke along it is drawn true on it, and what it measures goes to the app (`guideReading`: `{kind, length,
//! degrees, radius}`). The model is `ecclesia-ink`'s `guides`.

use ecclesia_ink::erase::Point;
use ecclesia_ink::guides::{Guide, Reading};
use ecclesia_ink::stroke::InkPoint;
use ecclesia_mupdf::PagePoint;

use super::{InkPixel, PageCanvas, ScreenQuad};
use crate::page::host::{Host, Wash};
use crate::page::placement::Placement;

/// The guides' colour: a cool grey-blue that reads on light and dark paper.
const GUIDE: u32 = 0x5b7fa6;
/// A ruler's width beside its edge, in page points.
const RULER_WIDTH: f32 = 28.0;
/// How near the pointer must come to the handle, in page points.
const HANDLE_REACH: f32 = 10.0;
/// Ticks: every this many points along a ruler, every this many degrees round a protractor.
const TICK_POINTS: f32 = 10.0;
const TICK_DEGREES: f32 = 10.0;

/// The guide on the page, and a drag on it under way.
#[derive(Debug, Default)]
pub(super) struct Guides {
    on: Option<(usize, Guide)>,
    drag: Option<Drag>,
}

/// What a drag holds: the body (moving it) or the handle (turning it or sizing it), from where it last was.
#[derive(Debug, Clone, Copy)]
struct Drag {
    handle: bool,
    last: PagePoint,
}

impl<H: Host> PageCanvas<H> {
    /// The guide to lay (the `guide` prop): `ruler`, `protractor`, `compass`, or anything else for none. A new one
    /// lies in the middle of the page in view.
    pub(crate) fn set_guide(&mut self, kind: Option<&str>) {
        let wanted = kind.filter(|k| matches!(*k, "ruler" | "protractor" | "compass"));
        let same = matches!(
            (wanted, &self.guides.on),
            (Some("ruler"), Some((_, Guide::Ruler { .. })))
                | (Some("protractor"), Some((_, Guide::Protractor { .. })))
                | (Some("compass"), Some((_, Guide::Compass { .. })))
                | (None, None)
        );
        if same {
            return;
        }
        self.guides.drag = None;
        self.guides.on = wanted.and_then(|kind| {
            let page = self.reported_page.unwrap_or(0);
            let size = self.bounds.get(page)?;
            let (w, h) = (size.x1 - size.x0, size.y1 - size.y0);
            let centre = Point {
                x: size.x0 + w / 2.0,
                y: size.y0 + h / 2.0,
            };
            Some((
                page,
                match kind {
                    "ruler" => Guide::Ruler {
                        origin: Point {
                            x: centre.x - w * 0.3,
                            y: centre.y,
                        },
                        angle: 0.0,
                        length: w * 0.6,
                    },
                    "protractor" => Guide::Protractor {
                        centre,
                        angle: 0.0,
                        radius: w * 0.2,
                    },
                    _ => Guide::Compass {
                        centre,
                        radius: w * 0.12,
                    },
                },
            ))
        });
        self.invalidate();
    }

    /// The left button went down at client pixel (`x`, `y`): on the guide's handle it turns or sizes it, on its body
    /// it moves. Returns whether it took hold.
    pub(super) fn guide_down(&mut self, x: i32, y: i32) -> bool {
        let Some((page, guide)) = self.guides.on else {
            return false;
        };
        let Some(at) = self.point_on(page, x, y) else {
            return false;
        };
        let p = Point { x: at.x, y: at.y };
        let handle = distance(p, handle_of(&guide)) <= HANDLE_REACH;
        if !handle && !on_body(&guide, p) {
            return false;
        }
        self.guides.drag = Some(Drag { handle, last: at });
        true
    }

    /// The mouse moved: a drag on the guide follows it. Returns whether one is under way.
    pub(super) fn guide_moved(&mut self, at: Option<(i32, i32)>) -> bool {
        let (Some(drag), Some((page, guide))) = (self.guides.drag, self.guides.on) else {
            return false;
        };
        let Some(to) = at.and_then(|(x, y)| self.point_on(page, x, y)) else {
            return true;
        };
        let (dx, dy) = (to.x - drag.last.x, to.y - drag.last.y);
        let to_p = Point { x: to.x, y: to.y };
        let moved = |p: Point| Point {
            x: p.x + dx,
            y: p.y + dy,
        };
        let next = match (guide, drag.handle) {
            (Guide::Ruler { origin, length, .. }, true) => Guide::Ruler {
                origin,
                angle: (to.y - origin.y).atan2(to.x - origin.x),
                length,
            },
            (
                Guide::Ruler {
                    origin,
                    angle,
                    length,
                },
                false,
            ) => Guide::Ruler {
                origin: moved(origin),
                angle,
                length,
            },
            (Guide::Protractor { centre, radius, .. }, true) => Guide::Protractor {
                centre,
                angle: (to.y - centre.y).atan2(to.x - centre.x),
                radius,
            },
            (
                Guide::Protractor {
                    centre,
                    angle,
                    radius,
                },
                false,
            ) => Guide::Protractor {
                centre: moved(centre),
                angle,
                radius,
            },
            (Guide::Compass { centre, .. }, true) => Guide::Compass {
                centre,
                radius: distance(centre, to_p).max(2.0),
            },
            (Guide::Compass { centre, radius }, false) => Guide::Compass {
                centre: moved(centre),
                radius,
            },
        };
        self.guides.on = Some((page, next));
        self.guides.drag = Some(Drag { last: to, ..drag });
        self.invalidate();
        true
    }

    /// The button came up: a drag on the guide ends. Returns whether one was under way.
    pub(super) fn guide_up(&mut self) -> bool {
        self.guides.drag.take().is_some()
    }

    /// `points` of a stroke on page `page` drawn true along the guide, at their mean width, when it lies along it;
    /// what it measures goes to the app.
    pub(super) fn guided(&self, page: usize, points: &[InkPoint]) -> Option<Vec<InkPoint>> {
        let (on, guide) = self.guides.on?;
        if on != page || points.is_empty() {
            return None;
        }
        let stroke: Vec<Point> = points.iter().map(|p| Point { x: p.x, y: p.y }).collect();
        let guided = guide.guide(&stroke)?;
        let reading = match guided.reading {
            Reading::Line { length, degrees } => serde_json::json!({
                "kind": "line", "length": length, "degrees": degrees,
            }),
            Reading::Arc { radius, degrees } => serde_json::json!({
                "kind": "arc", "radius": radius, "degrees": degrees,
            }),
        };
        self.emit("guideReading", reading);
        let width = points.iter().map(|p| p.width).sum::<f32>() / points.len() as f32;
        Some(
            guided
                .path
                .into_iter()
                .map(|p| InkPoint {
                    x: p.x,
                    y: p.y,
                    width,
                })
                .collect(),
        )
    }

    /// The guide as the frame draws it: its translucent body as quads, its lines, ticks and handle as polylines; on
    /// screen.
    pub(super) fn guide_shapes(&self) -> Option<(Vec<ScreenQuad>, Vec<Vec<InkPixel>>)> {
        let (page, guide) = self.guides.on?;
        let placement = Placement::of_page(&self.layout, &self.bounds, self.view.rotation, page)?;
        let screen = |p: Point| {
            let (x, y) = placement.to_layout(PagePoint { x: p.x, y: p.y });
            self.viewport.to_screen_point(x, y)
        };
        let line = |points: &[Point], width: f32| -> Vec<InkPixel> {
            points
                .iter()
                .map(|&p| {
                    let (x, y) = screen(p);
                    (x, y, width)
                })
                .collect()
        };
        let mut quads = Vec::new();
        let mut lines = Vec::new();
        match guide {
            Guide::Ruler {
                origin,
                angle,
                length,
            } => {
                let (cos, sin) = (angle.cos(), angle.sin());
                let at = |t: f32, off: f32| Point {
                    x: origin.x + cos * t - sin * off,
                    y: origin.y + sin * t + cos * off,
                };
                let corners = [
                    at(0.0, 0.0),
                    at(length, 0.0),
                    at(length, -RULER_WIDTH),
                    at(0.0, -RULER_WIDTH),
                ];
                quads.push(corners.map(screen));
                lines.push(line(&[corners[0], corners[1]], 1.5));
                let mut t = 0.0;
                while t <= length {
                    let long = (t / TICK_POINTS).round() as i64 % 5 == 0;
                    lines.push(line(
                        &[at(t, 0.0), at(t, if long { -10.0 } else { -5.0 })],
                        1.0,
                    ));
                    t += TICK_POINTS;
                }
            }
            Guide::Protractor {
                centre,
                angle,
                radius,
            } => {
                let arc: Vec<Point> = (0..=36)
                    .map(|i| {
                        let t = angle - std::f32::consts::PI * i as f32 / 36.0;
                        Point {
                            x: centre.x + t.cos() * radius,
                            y: centre.y + t.sin() * radius,
                        }
                    })
                    .collect();
                lines.push(line(&arc, 1.5));
                lines.push(line(&[arc[0], arc[arc.len() - 1]], 1.5));
                let mut d: f32 = 0.0;
                while d <= 180.0 {
                    let t = angle - d.to_radians();
                    let inner = if (d / TICK_DEGREES).round() as i64 % 3 == 0 {
                        0.85
                    } else {
                        0.92
                    };
                    lines.push(line(
                        &[
                            Point {
                                x: centre.x + t.cos() * radius * inner,
                                y: centre.y + t.sin() * radius * inner,
                            },
                            Point {
                                x: centre.x + t.cos() * radius,
                                y: centre.y + t.sin() * radius,
                            },
                        ],
                        1.0,
                    ));
                    d += TICK_DEGREES;
                }
            }
            Guide::Compass { centre, radius } => {
                let circle: Vec<Point> = (0..=72)
                    .map(|i| {
                        let t = std::f32::consts::TAU * i as f32 / 72.0;
                        Point {
                            x: centre.x + t.cos() * radius,
                            y: centre.y + t.sin() * radius,
                        }
                    })
                    .collect();
                lines.push(line(&circle, 1.0));
                lines.push(line(&[centre, handle_of(&guide)], 1.0));
                lines.push(line(&[centre, centre], 4.0));
            }
        }
        let grip = handle_of(&guide);
        lines.push(line(&[grip, grip], 9.0));
        Some((quads, lines))
    }

    /// Draws the guide's shapes on `host`.
    pub(super) fn draw_guide(
        host: &mut H,
        shapes: Option<(Vec<ScreenQuad>, Vec<Vec<InkPixel>>)>,
    ) -> Result<(), H::Error> {
        let Some((quads, lines)) = shapes else {
            return Ok(());
        };
        if !quads.is_empty() {
            host.highlight(&quads, 0xdde6f0, Wash::Translucent)?;
        }
        for line in lines {
            host.polyline(&line, GUIDE)?;
        }
        Ok(())
    }
}

/// Where `guide`'s handle is: the ruler's far end, the protractor's base end, the compass's pencil.
fn handle_of(guide: &Guide) -> Point {
    match *guide {
        Guide::Ruler {
            origin,
            angle,
            length,
        } => Point {
            x: origin.x + angle.cos() * length,
            y: origin.y + angle.sin() * length,
        },
        Guide::Protractor {
            centre,
            angle,
            radius,
        } => Point {
            x: centre.x + angle.cos() * radius,
            y: centre.y + angle.sin() * radius,
        },
        Guide::Compass { centre, radius } => Point {
            x: centre.x + radius,
            y: centre.y,
        },
    }
}

/// Whether `p` is on `guide`'s body: within the ruler's band, the protractor's half disc, near the compass's centre.
fn on_body(guide: &Guide, p: Point) -> bool {
    match *guide {
        Guide::Ruler {
            origin,
            angle,
            length,
        } => {
            let (cos, sin) = (angle.cos(), angle.sin());
            let along = (p.x - origin.x) * cos + (p.y - origin.y) * sin;
            let across = -(p.x - origin.x) * sin + (p.y - origin.y) * cos;
            (0.0..=length).contains(&along) && (-RULER_WIDTH..=0.0).contains(&across)
        }
        Guide::Protractor { centre, radius, .. } => distance(p, centre) <= radius,
        Guide::Compass { centre, .. } => distance(p, centre) <= HANDLE_REACH * 1.5,
    }
}

fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}
