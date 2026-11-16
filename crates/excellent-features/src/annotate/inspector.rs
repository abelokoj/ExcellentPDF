//! The annotation inspector (A.12): what the panel beside the pages shows of an annotation (its kind, author, dates,
//! colour, opacity and lock) and the edits it makes, each one change for one step of undo. Its note, text style,
//! border, fill and endings are the other tools' models (`notes`, `freetext`, `shapes`, `sticky`).
//!
//! A locked annotation (PDF's `/F` Locked) keeps its place, size, look and existence: [`allows`] says which changes
//! may still be made to it, its note and its lock.

use excellent_core::annot::Annotation;
use excellent_core::edit::Change;

use super::colors::MIN_OPACITY;
use super::shapes::changed;

/// The opacities the inspector offers.
pub const OPACITIES: [f32; 4] = [0.25, 0.5, 0.75, 1.0];

/// What the inspector shows of an annotation, beyond what its kind's own controls do.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    /// Who made it.
    pub author: String,
    /// When it was made and last changed, seconds since 1970 (UTC); 0 when the file doesn't say.
    pub created: i64,
    /// As `created`.
    pub modified: i64,
    /// Its colour, RGB from 0 to 1; `None` when it has none, or when its colour is a text box's background, which
    /// the text box's own controls set.
    pub color: Option<[f32; 3]>,
    /// From [`MIN_OPACITY`] to 1.
    pub opacity: f32,
    /// Whether it is locked.
    pub locked: bool,
}

impl Details {
    /// What the inspector shows of `annotation`.
    #[must_use]
    pub fn of(annotation: &Annotation) -> Self {
        Self {
            author: annotation.author.clone(),
            created: annotation.created,
            modified: annotation.modified,
            color: has_color(annotation)
                .then_some(annotation.style.color)
                .flatten(),
            opacity: annotation.style.opacity,
            locked: annotation.locked,
        }
    }
}

/// Whether the inspector offers a colour for `annotation`: all but a text box, whose text colour is its own row.
#[must_use]
pub fn has_color(annotation: &Annotation) -> bool {
    !annotation.is_text_box()
}

/// The change that names `author` as who made `annotation`, trimmed; none when it is the same, or empty.
#[must_use]
pub fn set_author(annotation: &Annotation, author: &str) -> Option<Change> {
    let author = author.trim();
    if author.is_empty() {
        return None;
    }
    changed(annotation, |a| author.clone_into(&mut a.author))
}

/// The change that gives `annotation` the colour `color`; none when it has it, or takes none here.
#[must_use]
pub fn set_color(annotation: &Annotation, color: [f32; 3]) -> Option<Change> {
    if !has_color(annotation) {
        return None;
    }
    let color = color.map(|channel| channel.clamp(0.0, 1.0));
    changed(annotation, |a| a.style.color = Some(color))
}

/// The change that draws `annotation` at `opacity`, kept from [`MIN_OPACITY`] to 1; none when it is.
#[must_use]
pub fn set_opacity(annotation: &Annotation, opacity: f32) -> Option<Change> {
    let opacity = if opacity.is_finite() {
        opacity.clamp(MIN_OPACITY, 1.0)
    } else {
        1.0
    };
    changed(annotation, |a| a.style.opacity = opacity)
}

/// The change that locks or unlocks `annotation`; none when it is so already.
#[must_use]
pub fn set_locked(annotation: &Annotation, locked: bool) -> Option<Change> {
    changed(annotation, |a| a.locked = locked)
}

/// Whether `change` may be made to `before`, the annotation it is about as it is now: anything to an unlocked one;
/// to a locked one, only its note and its lock (and the time it changed), never a deletion. Changes to other
/// annotations are not its to judge.
#[must_use]
pub fn allows(before: &Annotation, change: &Change) -> bool {
    if !before.locked {
        return true;
    }
    match change {
        Change::UpdateAnnotation(after) if after.id == before.id => {
            let mut kept = after.clone();
            kept.contents.clone_from(&before.contents);
            kept.locked = before.locked;
            kept.modified = before.modified;
            kept == *before
        }
        Change::DeleteAnnotation { id, .. } | Change::StampArt { id, .. } => *id != before.id,
        _ => true,
    }
}

/// `seconds` since 1970 as `YYYY-MM-DD HH:MM` in UTC; empty for 0, a file that doesn't say.
///
/// shortcut: UTC, not the reader's time zone; upgrade when the app takes a date and time crate (U.7's formats).
#[must_use]
pub fn date(seconds: i64) -> String {
    if seconds == 0 {
        return String::new();
    }
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// The year, month and day `days` after 1970-01-01, in the proleptic Gregorian calendar (Howard Hinnant's
/// `civil_from_days`).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use excellent_core::annot::{Boxed, Shape, Style};
    use excellent_mupdf::PageRect;

    use super::*;

    fn square() -> Annotation {
        Annotation::new(
            1,
            Shape::Boxed(Boxed::Square, PageRect::default()),
            Style::default(),
            "Ada",
        )
    }

    fn updated(change: Option<Change>) -> Annotation {
        match change {
            Some(Change::UpdateAnnotation(annotation)) => annotation,
            other => panic!("expected an update, got {other:?}"),
        }
    }

    #[test]
    fn each_property_set_here_is_one_change_and_none_when_it_is_the_same() {
        let a = square();
        assert_eq!(updated(set_author(&a, "  Grace ")).author, "Grace");
        assert!(set_author(&a, "Ada").is_none());
        assert!(
            set_author(&a, "   ").is_none(),
            "an author can't be emptied"
        );
        assert_eq!(
            updated(set_color(&a, [0.0, 0.5, 2.0])).style.color,
            Some([0.0, 0.5, 1.0])
        );
        assert!(set_color(&a, Style::default().color.expect("a colour")).is_none());
        assert_eq!(updated(set_opacity(&a, 0.5)).style.opacity, 0.5);
        assert_eq!(updated(set_opacity(&a, 0.0)).style.opacity, MIN_OPACITY);
        assert!(set_opacity(&a, f32::NAN).is_none(), "NaN is taken as 1");
        assert!(updated(set_locked(&a, true)).locked);
        assert!(set_locked(&a, false).is_none());
    }

    #[test]
    fn a_text_box_s_colour_is_not_the_inspector_s() {
        let boxed = Annotation::new(
            0,
            Shape::Boxed(Boxed::FreeText, PageRect::default()),
            Style::default(),
            "Ada",
        );
        assert!(set_color(&boxed, [0.0; 3]).is_none());
        assert_eq!(Details::of(&boxed).color, None);
        assert_eq!(Details::of(&square()).color, Style::default().color);
    }

    #[test]
    fn a_locked_annotation_takes_only_its_note_and_its_lock() {
        let mut a = square();
        a.locked = true;
        let mut noted = a.clone();
        noted.contents = "a note".to_owned();
        noted.modified += 10;
        assert!(allows(&a, &Change::UpdateAnnotation(noted)));
        assert!(allows(&a, &set_locked(&a, false).expect("unlock")));
        assert!(!allows(&a, &set_opacity(&a, 0.5).expect("restyle")));
        assert!(!allows(&a, &set_author(&a, "Grace").expect("rename")));
        let gone = Change::DeleteAnnotation {
            page: a.page,
            id: a.id.clone(),
        };
        assert!(!allows(&a, &gone));
        let other = square();
        assert!(allows(
            &a,
            &Change::DeleteAnnotation {
                page: other.page,
                id: other.id.clone()
            }
        ));
        a.locked = false;
        assert!(allows(&a, &gone), "unlocked, anything goes");
    }

    #[test]
    fn dates_read_as_utc_and_a_missing_one_is_empty() {
        assert_eq!(date(0), "");
        assert_eq!(date(1), "1970-01-01 00:00");
        assert_eq!(date(951_782_400), "2000-02-29 00:00");
        assert_eq!(date(1_794_799_500), "2026-11-16 03:25");
        assert_eq!(date(-86_400), "1969-12-31 00:00");
    }
}
