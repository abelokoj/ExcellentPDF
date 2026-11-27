//! Raw sample timestamp storage and timed stroke replay geometry (H.9 step 3 & H.22, plan §8.3, §27.7).
//!
//! Stores raw pen samples (position, pressure, tilt, timestamp, tool) in a compact little-endian binary format with
//! microsecond-resolution delta timestamps, and replays a page's strokes in exact recorded order and timing with
//! smooth sub-sample tip interpolation.

use std::fmt;

use crate::pens::Blend;
use crate::sample::{Sample, Tilt, Tool};
use crate::stroke::InkPoint;

/// Magic header for encoded raw sample blobs (`"EXS1"` = EcclesiaPDF Samples v1).
const MAGIC: &[u8; 4] = b"EXS1";

/// Flag bit: sample carries a pressure value.
const FLAG_PRESSURE: u8 = 1 << 0;
/// Flag bit: sample carries a tilt value.
const FLAG_TILT: u8 = 1 << 1;

/// Error decoding a compact raw-sample blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeSamplesError {
    /// Missing or unrecognized header magic.
    BadMagic,
    /// Unexpected end of input while decoding samples.
    Truncated,
    /// Unknown tool discriminant in a sample flag byte.
    UnknownTool(u8),
}

impl fmt::Display for DecodeSamplesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("invalid raw sample header"),
            Self::Truncated => f.write_str("truncated raw sample stream"),
            Self::UnknownTool(code) => write!(f, "unknown sample tool code {code}"),
        }
    }
}

impl std::error::Error for DecodeSamplesError {}

/// Encodes raw `samples` into a compact binary representation suitable for storing in a PDF annotation's private key
/// (H.9 step 3) and replaying later (H.22).
///
/// Timestamps are stored as a base `f64` millisecond origin followed by zigzag-encoded microsecond deltas (1 µs
/// resolution = 0.001 ms).
#[must_use]
pub fn encode_samples(samples: &[Sample]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + samples.len() * 14);
    out.extend_from_slice(MAGIC);
    write_varint(&mut out, samples.len() as u64);
    let Some(first) = samples.first() else {
        return out;
    };
    let origin = first.time;
    out.extend_from_slice(&origin.to_le_bytes());
    let mut prev_us: i64 = 0;

    for sample in samples {
        let cur_us = ((sample.time - origin) * 1000.0).round() as i64;
        let delta_us = cur_us.saturating_sub(prev_us);
        prev_us = cur_us;
        write_varint(&mut out, zigzag_encode(delta_us));

        let tool_bits = match sample.tool {
            Tool::Pen => 0u8,
            Tool::Eraser => 1u8,
            Tool::Touch => 2u8,
            Tool::Mouse => 3u8,
        };
        let mut flags = tool_bits << 2;
        if sample.pressure.is_some() {
            flags |= FLAG_PRESSURE;
        }
        if sample.tilt.is_some() {
            flags |= FLAG_TILT;
        }
        out.push(flags);
        out.extend_from_slice(&sample.x.to_le_bytes());
        out.extend_from_slice(&sample.y.to_le_bytes());
        if let Some(p) = sample.pressure {
            out.extend_from_slice(&p.to_le_bytes());
        }
        if let Some(t) = sample.tilt {
            out.extend_from_slice(&t.x.to_le_bytes());
            out.extend_from_slice(&t.y.to_le_bytes());
        }
    }
    out
}

/// Decodes a raw sample blob produced by [`encode_samples`].
///
/// # Errors
///
/// Returns [`DecodeSamplesError`] if the header is invalid, the data is truncated, or a tool code is unrecognized.
pub fn decode_samples(bytes: &[u8]) -> Result<Vec<Sample>, DecodeSamplesError> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err(DecodeSamplesError::BadMagic);
    }
    let mut cursor = &bytes[MAGIC.len()..];
    let count =
        usize::try_from(read_varint(&mut cursor)?).map_err(|_| DecodeSamplesError::Truncated)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let origin = read_f64(&mut cursor)?;
    let mut samples = Vec::with_capacity(count.min(65_536));
    let mut cur_us: i64 = 0;

    for _ in 0..count {
        let delta = zigzag_decode(read_varint(&mut cursor)?);
        cur_us = cur_us.saturating_add(delta);
        let flags = read_u8(&mut cursor)?;
        let tool = match (flags >> 2) & 0x07 {
            0 => Tool::Pen,
            1 => Tool::Eraser,
            2 => Tool::Touch,
            3 => Tool::Mouse,
            other => return Err(DecodeSamplesError::UnknownTool(other)),
        };
        let x = read_f32(&mut cursor)?;
        let y = read_f32(&mut cursor)?;
        let pressure = if flags & FLAG_PRESSURE != 0 {
            Some(read_f32(&mut cursor)?)
        } else {
            None
        };
        let tilt = if flags & FLAG_TILT != 0 {
            Some(Tilt {
                x: read_f32(&mut cursor)?,
                y: read_f32(&mut cursor)?,
            })
        } else {
            None
        };
        samples.push(Sample {
            x,
            y,
            pressure,
            tilt,
            time: origin + (cur_us as f64) / 1000.0,
            tool,
        });
    }
    Ok(samples)
}

/// A single timestamped point ready for timed replay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedPoint {
    /// Position and width in page points.
    pub point: InkPoint,
    /// Original sample timestamp in milliseconds.
    pub time_ms: f64,
}

/// One recorded stroke on a replay timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedStroke {
    /// Stable identifier (typically the PDF annotation's `/NM`).
    pub id: String,
    /// Color as `0xRRGGBB`.
    pub color: u32,
    /// Opacity, `0.0..=1.0`.
    pub opacity: f32,
    /// How the stroke blends with the page (`Blend::Multiply` for highlighters).
    pub blend: Blend,
    /// Points in chronological order with timestamps.
    pub points: Vec<TimedPoint>,
}

impl TimedStroke {
    /// Builds a [`TimedStroke`] from raw [`Sample`]s and matching widths (falling back to `default_width`).
    #[must_use]
    pub fn from_samples(
        id: impl Into<String>,
        color: u32,
        opacity: f32,
        blend: Blend,
        samples: &[Sample],
        widths: &[f32],
        default_width: f32,
    ) -> Self {
        let points = samples
            .iter()
            .enumerate()
            .map(|(i, s)| TimedPoint {
                point: InkPoint {
                    x: s.x,
                    y: s.y,
                    width: widths.get(i).copied().unwrap_or(default_width),
                },
                time_ms: s.time,
            })
            .collect();
        Self {
            id: id.into(),
            color: color & 0xff_ffff,
            opacity,
            blend,
            points,
        }
    }

    /// Timestamp of the first point, or `None` if empty.
    #[must_use]
    pub fn start_ms(&self) -> Option<f64> {
        self.points.first().map(|p| p.time_ms)
    }

    /// Timestamp of the last point, or `None` if empty.
    #[must_use]
    pub fn end_ms(&self) -> Option<f64> {
        self.points.last().map(|p| p.time_ms)
    }

    /// Duration of this stroke in milliseconds.
    #[must_use]
    pub fn duration_ms(&self) -> f64 {
        match (self.start_ms(), self.end_ms()) {
            (Some(a), Some(b)) => (b - a).max(0.0),
            _ => 0.0,
        }
    }
}

/// One visible stroke (complete or partially drawn) at a given point on the replay timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayedStroke {
    /// Index of the stroke on the timeline.
    pub stroke_index: usize,
    /// Stable stroke identifier.
    pub id: String,
    /// Color as `0xRRGGBB`.
    pub color: u32,
    /// Opacity, `0.0..=1.0`.
    pub opacity: f32,
    /// How the stroke blends with the page.
    pub blend: Blend,
    /// Visible points up to the current replay timestamp (including interpolated tip when in progress).
    pub points: Vec<InkPoint>,
    /// Whether this stroke has finished drawing at the queried timestamp.
    pub complete: bool,
}

/// Snapshot of a page's strokes at a specific elapsed replay timestamp.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReplayFrame {
    /// Current elapsed time on the timeline, in milliseconds.
    pub elapsed_ms: f64,
    /// Total duration of the timeline, in milliseconds.
    pub total_ms: f64,
    /// Normalized progress in `0.0..=1.0`.
    pub progress: f32,
    /// All strokes that have begun by `elapsed_ms`, in chronological order.
    pub strokes: Vec<ReplayedStroke>,
    /// Index of the stroke currently being drawn, if any.
    pub active_stroke: Option<usize>,
}

/// Chronological replay timeline of strokes on a page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Timeline {
    strokes: Vec<TimedStroke>,
    origin_ms: f64,
    total_ms: f64,
}

impl Timeline {
    /// Creates a timeline from `strokes`, sorting them chronologically by their first sample timestamp and normalizing
    /// time so `0.0 ms` is the first sample of the earliest stroke. Empty strokes are omitted.
    ///
    /// When `max_pause_ms` is `Some(cap)`, idle gaps between consecutive strokes longer than `cap` are compressed to
    /// `cap`; pass `None` to preserve exact verbatim recording timing.
    #[must_use]
    pub fn new(strokes: Vec<TimedStroke>, max_pause_ms: Option<f64>) -> Self {
        let mut non_empty: Vec<(usize, TimedStroke)> = strokes
            .into_iter()
            .filter(|s| !s.points.is_empty())
            .enumerate()
            .collect();
        if non_empty.is_empty() {
            return Self::default();
        }
        non_empty.sort_by(|(idx_a, a), (idx_b, b)| {
            let ta = a.start_ms().unwrap_or(0.0);
            let tb = b.start_ms().unwrap_or(0.0);
            ta.total_cmp(&tb).then_with(|| idx_a.cmp(idx_b))
        });

        let origin_ms = non_empty[0].1.start_ms().unwrap_or(0.0);
        let mut shifted_strokes: Vec<TimedStroke> = Vec::with_capacity(non_empty.len());
        let mut removed_gap = 0.0_f64;
        let mut prev_end = origin_ms;

        for (i, (_, mut stroke)) in non_empty.into_iter().enumerate() {
            if i > 0
                && let Some(cap) = max_pause_ms
                && cap.is_finite()
                && cap >= 0.0
            {
                let start = stroke.start_ms().unwrap_or(prev_end);
                let gap = start - prev_end;
                if gap > cap {
                    removed_gap += gap - cap;
                }
            }
            if let Some(end) = stroke.end_ms() {
                prev_end = prev_end.max(end);
            }
            for pt in &mut stroke.points {
                pt.time_ms = (pt.time_ms - origin_ms - removed_gap).max(0.0);
            }
            shifted_strokes.push(stroke);
        }

        let total_ms = shifted_strokes
            .iter()
            .filter_map(TimedStroke::end_ms)
            .fold(0.0_f64, f64::max);

        Self {
            strokes: shifted_strokes,
            origin_ms,
            total_ms,
        }
    }

    /// The strokes in chronological order (with timestamps normalized to start at `0.0 ms`).
    #[must_use]
    pub fn strokes(&self) -> &[TimedStroke] {
        &self.strokes
    }

    /// Original timestamp of the earliest sample before normalization.
    #[must_use]
    pub fn origin_ms(&self) -> f64 {
        self.origin_ms
    }

    /// Total duration of the recording, in milliseconds.
    #[must_use]
    pub fn total_duration_ms(&self) -> f64 {
        self.total_ms
    }

    /// Whether the timeline holds no strokes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    /// Number of strokes on the timeline.
    #[must_use]
    pub fn len(&self) -> usize {
        self.strokes.len()
    }

    /// Computes the exact frame at `elapsed_ms` (clamped to `0.0..=total_duration_ms()`).
    #[must_use]
    pub fn frame_at(&self, elapsed_ms: f64) -> ReplayFrame {
        let elapsed = if elapsed_ms.is_finite() {
            elapsed_ms.clamp(0.0, self.total_ms)
        } else {
            0.0
        };
        let progress = if self.total_ms <= f64::EPSILON {
            if self.strokes.is_empty() { 0.0 } else { 1.0 }
        } else {
            (elapsed / self.total_ms) as f32
        };

        let mut visible = Vec::new();
        let mut active_stroke = None;

        for (stroke_index, stroke) in self.strokes.iter().enumerate() {
            let Some(start) = stroke.start_ms() else {
                continue;
            };
            if elapsed < start {
                continue;
            }
            let end = stroke.end_ms().unwrap_or(start);
            if elapsed >= end {
                visible.push(ReplayedStroke {
                    stroke_index,
                    id: stroke.id.clone(),
                    color: stroke.color,
                    opacity: stroke.opacity,
                    blend: stroke.blend,
                    points: stroke.points.iter().map(|p| p.point).collect(),
                    complete: true,
                });
            } else {
                let mut pts: Vec<InkPoint> = Vec::new();
                for pair in stroke.points.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    if a.time_ms <= elapsed {
                        if pts.is_empty() {
                            pts.push(a.point);
                        }
                        if b.time_ms <= elapsed {
                            pts.push(b.point);
                        } else {
                            let span = b.time_ms - a.time_ms;
                            if span > f64::EPSILON {
                                let t = ((elapsed - a.time_ms) / span).clamp(0.0, 1.0) as f32;
                                pts.push(InkPoint {
                                    x: a.point.x + (b.point.x - a.point.x) * t,
                                    y: a.point.y + (b.point.y - a.point.y) * t,
                                    width: a.point.width + (b.point.width - a.point.width) * t,
                                });
                            }
                            break;
                        }
                    }
                }
                if pts.is_empty()
                    && let Some(first) = stroke.points.first()
                {
                    pts.push(first.point);
                }
                active_stroke = Some(stroke_index);
                visible.push(ReplayedStroke {
                    stroke_index,
                    id: stroke.id.clone(),
                    color: stroke.color,
                    opacity: stroke.opacity,
                    blend: stroke.blend,
                    points: pts,
                    complete: false,
                });
            }
        }

        ReplayFrame {
            elapsed_ms: elapsed,
            total_ms: self.total_ms,
            progress,
            strokes: visible,
            active_stroke,
        }
    }
}

fn zigzag_encode(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

fn zigzag_decode(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(cursor: &mut &[u8]) -> Result<u64, DecodeSamplesError> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = read_u8(cursor)?;
        result |= u64::from(byte & 0x7f)
            .checked_shl(shift)
            .ok_or(DecodeSamplesError::Truncated)?;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err(DecodeSamplesError::Truncated);
        }
    }
}

fn read_u8(cursor: &mut &[u8]) -> Result<u8, DecodeSamplesError> {
    let (&first, rest) = cursor.split_first().ok_or(DecodeSamplesError::Truncated)?;
    *cursor = rest;
    Ok(first)
}

fn read_f32(cursor: &mut &[u8]) -> Result<f32, DecodeSamplesError> {
    let (head, rest) = cursor
        .split_at_checked(4)
        .ok_or(DecodeSamplesError::Truncated)?;
    *cursor = rest;
    let bytes: [u8; 4] = head.try_into().expect("slice of length 4");
    Ok(f32::from_le_bytes(bytes))
}

fn read_f64(cursor: &mut &[u8]) -> Result<f64, DecodeSamplesError> {
    let (head, rest) = cursor
        .split_at_checked(8)
        .ok_or(DecodeSamplesError::Truncated)?;
    *cursor = rest;
    let bytes: [u8; 8] = head.try_into().expect("slice of length 8");
    Ok(f64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_samples_encode_and_decode_within_one_microsecond() {
        let samples = vec![
            Sample {
                x: 10.25,
                y: 20.5,
                pressure: Some(0.45),
                tilt: Some(Tilt { x: 12.0, y: -5.5 }),
                time: 1000.0,
                tool: Tool::Pen,
            },
            Sample {
                x: 14.0,
                y: 22.0,
                pressure: None,
                tilt: None,
                time: 1004.167,
                tool: Tool::Touch,
            },
            Sample {
                x: 18.5,
                y: 25.0,
                pressure: Some(0.9),
                tilt: None,
                time: 1008.333,
                tool: Tool::Eraser,
            },
        ];
        let encoded = encode_samples(&samples);
        let decoded = decode_samples(&encoded).expect("decodes");
        assert_eq!(decoded.len(), samples.len());
        for (a, b) in samples.iter().zip(&decoded) {
            assert_eq!(a.x, b.x);
            assert_eq!(a.y, b.y);
            assert_eq!(a.pressure, b.pressure);
            assert_eq!(a.tilt, b.tilt);
            assert_eq!(a.tool, b.tool);
            assert!(
                (a.time - b.time).abs() < 0.002,
                "time drift: {} vs {}",
                a.time,
                b.time
            );
        }
        assert_eq!(decode_samples(b"NOPE"), Err(DecodeSamplesError::BadMagic));
        assert_eq!(decode_samples(b"EXS1"), Err(DecodeSamplesError::Truncated));
        assert_eq!(decode_samples(&encode_samples(&[])).expect("empty"), vec![]);
    }

    #[test]
    fn timeline_replays_in_exact_recorded_order_and_interpolates_mid_stroke() {
        let s1 = TimedStroke::from_samples(
            "first",
            0x1a3d8f,
            1.0,
            Blend::Normal,
            &[Sample::at(0.0, 0.0, 500.0), Sample::at(100.0, 0.0, 600.0)],
            &[2.0, 4.0],
            2.0,
        );
        let s2 = TimedStroke::from_samples(
            "second",
            0xc62828,
            1.0,
            Blend::Normal,
            &[Sample::at(0.0, 20.0, 750.0), Sample::at(50.0, 20.0, 850.0)],
            &[2.0, 2.0],
            2.0,
        );
        // Deliberately pass out of chronological order; timeline must sort them.
        let timeline = Timeline::new(vec![s2, s1], None);
        assert_eq!(timeline.origin_ms(), 500.0);
        assert!((timeline.total_duration_ms() - 350.0).abs() < 1e-6);
        assert_eq!(timeline.strokes()[0].id, "first");
        assert_eq!(timeline.strokes()[1].id, "second");

        // At t = 50 ms: halfway through "first", "second" not started.
        let mid = timeline.frame_at(50.0);
        assert_eq!(mid.strokes.len(), 1);
        assert_eq!(mid.active_stroke, Some(0));
        assert!(!mid.strokes[0].complete);
        let tip = *mid.strokes[0].points.last().expect("tip");
        assert!((tip.x - 50.0).abs() < 1e-4 && (tip.width - 3.0).abs() < 1e-4);

        // At t = 200 ms (in the idle gap between 100 ms and 250 ms): "first" complete, "second" not started.
        let gap = timeline.frame_at(200.0);
        assert_eq!(gap.strokes.len(), 1);
        assert!(gap.strokes[0].complete);
        assert_eq!(gap.active_stroke, None);

        // At t = 300 ms: "first" complete, "second" halfway.
        let second_mid = timeline.frame_at(300.0);
        assert_eq!(second_mid.strokes.len(), 2);
        assert_eq!(second_mid.active_stroke, Some(1));
        assert!((second_mid.strokes[1].points.last().expect("tip").x - 25.0).abs() < 1e-4);

        // At end: both complete.
        let end = timeline.frame_at(350.0);
        assert_eq!(end.strokes.len(), 2);
        assert!(end.strokes.iter().all(|s| s.complete));
        assert!((end.progress - 1.0).abs() < 1e-6);
    }

    #[test]
    fn max_pause_compresses_long_idle_gaps_without_changing_stroke_durations() {
        let s1 = TimedStroke::from_samples(
            "a",
            0,
            1.0,
            Blend::Normal,
            &[Sample::at(0.0, 0.0, 0.0), Sample::at(10.0, 0.0, 100.0)],
            &[],
            2.0,
        );
        let s2 = TimedStroke::from_samples(
            "b",
            0,
            1.0,
            Blend::Normal,
            &[
                Sample::at(0.0, 10.0, 10_100.0),
                Sample::at(10.0, 10.0, 10_200.0),
            ],
            &[],
            2.0,
        );
        let compressed = Timeline::new(vec![s1, s2], Some(500.0));
        assert!((compressed.total_duration_ms() - 700.0).abs() < 1e-6);
        assert!((compressed.strokes()[1].duration_ms() - 100.0).abs() < 1e-6);
    }
}
