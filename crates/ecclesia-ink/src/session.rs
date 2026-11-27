//! Recorded pen sessions (plan §16): the samples of strokes as a pen drew them, written as text so a session
//! recorded on a real device becomes a test fixture that CI replays.
//!
//! The format: a first line `ecclesiapdf-ink-session 1`, then for each stroke a line `stroke` followed by one line a
//! sample: `time x y pressure tilt-x tilt-y tool`, with `-` for what the device didn't report. Blank lines and lines
//! starting with `#` are ignored.

use std::fmt::{self, Write as _};

use crate::sample::{Sample, Tilt, Tool};

/// The first line of every session file.
const HEADER: &str = "ecclesiapdf-ink-session 1";

/// The strokes of a recorded session, each its samples in the order drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    /// The strokes, in the order drawn.
    pub strokes: Vec<Vec<Sample>>,
}

/// A session file that doesn't read: which line, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSessionError {
    line: usize,
    reason: &'static str,
}

impl fmt::Display for ParseSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ParseSessionError {}

impl Session {
    /// The session as a session file.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{HEADER}\n");
        for stroke in &self.strokes {
            text.push_str("stroke\n");
            for sample in stroke {
                let optional =
                    |value: Option<f32>| value.map_or_else(|| "-".to_owned(), |v| v.to_string());
                let _ = writeln!(
                    text,
                    "{} {} {} {} {} {} {}",
                    sample.time,
                    sample.x,
                    sample.y,
                    optional(sample.pressure),
                    optional(sample.tilt.map(|tilt| tilt.x)),
                    optional(sample.tilt.map(|tilt| tilt.y)),
                    sample.tool.name()
                );
            }
        }
        text
    }

    /// Reads a session file.
    ///
    /// # Errors
    ///
    /// The header is missing, a sample comes before any `stroke`, or a sample line doesn't read.
    pub fn parse(text: &str) -> Result<Self, ParseSessionError> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));
        match lines.next() {
            Some((_, HEADER)) => {}
            Some((line, _)) => {
                return Err(ParseSessionError {
                    line,
                    reason: "not an ink session",
                });
            }
            None => {
                return Err(ParseSessionError {
                    line: 1,
                    reason: "empty",
                });
            }
        }
        let mut session = Self::default();
        for (line, text) in lines {
            if text == "stroke" {
                session.strokes.push(Vec::new());
                continue;
            }
            let Some(stroke) = session.strokes.last_mut() else {
                return Err(ParseSessionError {
                    line,
                    reason: "a sample before any stroke",
                });
            };
            stroke.push(sample(text).ok_or(ParseSessionError {
                line,
                reason: "a sample that doesn't read",
            })?);
        }
        Ok(session)
    }
}

/// One sample line.
fn sample(line: &str) -> Option<Sample> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let [time, x, y, pressure, tilt_x, tilt_y, tool] = fields.as_slice() else {
        return None;
    };
    let optional = |field: &str| -> Option<Option<f32>> {
        if field == "-" {
            Some(None)
        } else {
            field.parse().ok().map(Some)
        }
    };
    let tilt = match (optional(tilt_x)?, optional(tilt_y)?) {
        (Some(x), Some(y)) => Some(Tilt { x, y }),
        (None, None) => None,
        _ => return None,
    };
    Some(Sample {
        x: x.parse().ok()?,
        y: y.parse().ok()?,
        pressure: optional(pressure)?,
        tilt,
        time: time.parse().ok()?,
        tool: Tool::parse(tool)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_goes_out_and_back() {
        let session = Session {
            strokes: vec![
                vec![
                    Sample {
                        pressure: Some(0.25),
                        tilt: Some(Tilt { x: 12.5, y: -3.0 }),
                        ..Sample::at(1.5, 2.0, 0.0)
                    },
                    Sample::at(3.0, 4.25, 8.333),
                ],
                vec![Sample {
                    tool: Tool::Eraser,
                    ..Sample::at(0.0, 0.0, 100.0)
                }],
            ],
        };
        assert_eq!(Session::parse(&session.to_text()), Ok(session));
    }

    #[test]
    fn a_broken_file_says_where() {
        assert_eq!(Session::parse("").unwrap_err().line, 1);
        let no_stroke = format!("{HEADER}\n0 1 2 - - - pen\n");
        assert_eq!(Session::parse(&no_stroke).unwrap_err().line, 2);
        let half_tilt = format!("{HEADER}\nstroke\n# a comment\n0 1 2 0.5 10 - pen\n");
        assert_eq!(Session::parse(&half_tilt).unwrap_err().line, 4);
    }
}
