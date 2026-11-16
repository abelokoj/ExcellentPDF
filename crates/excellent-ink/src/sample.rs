//! A pen sample as the hardware reports it (plan §8.2, step 1): where, how hard, how tilted, when, and with what.

/// What made a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    /// A pen's tip.
    #[default]
    Pen,
    /// A pen's eraser end, or the pen turned over.
    Eraser,
    /// A finger.
    Touch,
    /// A mouse.
    Mouse,
}

impl Tool {
    /// Its one-word name, as session files write it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Pen => "pen",
            Self::Eraser => "eraser",
            Self::Touch => "touch",
            Self::Mouse => "mouse",
        }
    }

    /// The tool named `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Pen, Self::Eraser, Self::Touch, Self::Mouse]
            .into_iter()
            .find(|tool| tool.name() == name)
    }
}

/// How far the pen leans, as Windows and the web report it: the angle between the pen and the surface's normal,
/// seen along each axis, in degrees from -90 to 90 (positive leaning right and towards the reader).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tilt {
    /// Leaning right (positive) or left, in degrees.
    pub x: f32,
    /// Leaning towards the reader (positive) or away, in degrees.
    pub y: f32,
}

impl Tilt {
    /// The pen's angle to the surface: 90 upright, 0 lying flat.
    #[must_use]
    pub fn altitude(self) -> f32 {
        let (tx, ty) = (self.x.to_radians().tan(), self.y.to_radians().tan());
        90.0 - tx.hypot(ty).atan().to_degrees()
    }

    /// Which way the pen leans, in degrees clockwise from the right (screen y grows down); 0 when it stands upright.
    #[must_use]
    pub fn azimuth(self) -> f32 {
        let (tx, ty) = (self.x.to_radians().tan(), self.y.to_radians().tan());
        if tx == 0.0 && ty == 0.0 {
            return 0.0;
        }
        ty.atan2(tx).to_degrees().rem_euclid(360.0)
    }
}

/// One sample of a stroke, in whatever space the caller works in (page points once on a page).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Across.
    pub x: f32,
    /// Down.
    pub y: f32,
    /// 0 to 1; `None` when the device doesn't say.
    pub pressure: Option<f32>,
    /// How the pen leans; `None` when the device doesn't say.
    pub tilt: Option<Tilt>,
    /// When, in milliseconds on any clock that only goes forward.
    pub time: f64,
    /// What made it.
    pub tool: Tool,
}

impl Sample {
    /// A sample at (`x`, `y`) at `time`, from a pen tip that reports nothing else.
    #[must_use]
    pub fn at(x: f32, y: f32, time: f64) -> Self {
        Self {
            x,
            y,
            pressure: None,
            tilt: None,
            time,
            tool: Tool::Pen,
        }
    }

    /// How far it is from `other`.
    #[must_use]
    pub fn distance(&self, other: &Self) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilt_gives_altitude_and_azimuth() {
        let upright = Tilt { x: 0.0, y: 0.0 };
        assert_eq!(upright.altitude(), 90.0);
        assert_eq!(upright.azimuth(), 0.0);
        let right = Tilt { x: 30.0, y: 0.0 };
        assert!((right.altitude() - 60.0).abs() < 1e-3);
        assert!(right.azimuth().abs() < 1e-3);
        let towards = Tilt { x: 0.0, y: 45.0 };
        assert!((towards.altitude() - 45.0).abs() < 1e-3);
        assert!((towards.azimuth() - 90.0).abs() < 1e-3);
        let left = Tilt { x: -20.0, y: 0.0 };
        assert!((left.azimuth() - 180.0).abs() < 1e-3);
    }

    #[test]
    fn tools_go_by_name() {
        for tool in [Tool::Pen, Tool::Eraser, Tool::Touch, Tool::Mouse] {
            assert_eq!(Tool::parse(tool.name()), Some(tool));
        }
        assert_eq!(Tool::parse("brush"), None);
    }
}
