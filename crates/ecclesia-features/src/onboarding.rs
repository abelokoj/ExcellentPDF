//! The first-run guide and tips (plan U.9), the model: the guide's steps, when it opens by itself, and which tips the
//! reader has seen. The guide and the tip strip are drawn by the app (`ecclesia-app::onboarding`).

/// A step of the guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// Opening a document.
    Open,
    /// Reading it: layouts, colours, the reading aids.
    Read,
    /// The command palette, where every command is.
    Palette,
}

/// The guide's steps, in order.
pub const STEPS: [Step; 3] = [Step::Open, Step::Read, Step::Palette];

/// Where the reader is in the guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Guide {
    at: usize,
}

impl Guide {
    /// The step shown.
    pub fn step(self) -> Step {
        STEPS[self.at]
    }

    /// Its number, from 1.
    pub fn number(self) -> usize {
        self.at + 1
    }

    /// Whether it is the first step.
    pub fn is_first(self) -> bool {
        self.at == 0
    }

    /// Whether it is the last step, after which the guide is done.
    pub fn is_last(self) -> bool {
        self.at + 1 == STEPS.len()
    }

    /// The next step; `None` after the last.
    pub fn next(self) -> Option<Self> {
        (!self.is_last()).then_some(Self { at: self.at + 1 })
    }

    /// The step before; `None` before the first.
    pub fn back(self) -> Option<Self> {
        Some(Self {
            at: self.at.checked_sub(1)?,
        })
    }
}

/// Whether the guide opens with the window: the first time the app starts, but never over files the launch opens.
/// Those are what the reader came for, so the guide waits for a start without any (it isn't marked seen).
pub fn opens_at_start(seen: bool, launch_files: usize) -> bool {
    !seen && launch_files == 0
}

/// A tip, shown once, the first time its moment comes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tip {
    /// The first document shown: every command is in the palette.
    Palette,
    /// A second tab opened: the tab list finds any of them.
    TabList,
}

/// Every tip, in the order the settings file lists those seen.
pub const TIPS: [Tip; 2] = [Tip::Palette, Tip::TabList];

impl Tip {
    /// Its name in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Self::Palette => "palette",
            Self::TabList => "tab_list",
        }
    }
}

/// The tips the reader has seen. The settings file keeps them as their ids, separated by commas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeenTips(Vec<Tip>);

impl SeenTips {
    /// Reads `text` as the settings file writes it; unknown ids (a later version's tips) are left out.
    pub fn parse(text: &str) -> Self {
        Self(
            TIPS.into_iter()
                .filter(|tip| text.split(',').any(|id| id.trim() == tip.id()))
                .collect(),
        )
    }

    /// Whether the reader has seen `tip`.
    pub fn contains(&self, tip: Tip) -> bool {
        self.0.contains(&tip)
    }

    /// Records `tip` as seen; whether it is new.
    pub fn insert(&mut self, tip: Tip) -> bool {
        if self.contains(tip) {
            return false;
        }
        self.0.push(tip);
        self.0.sort_by_key(|tip| TIPS.iter().position(|t| t == tip));
        true
    }

    /// The text the settings file keeps.
    pub fn to_text(&self) -> String {
        self.0
            .iter()
            .map(|tip| tip.id())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Whether `tip` shows when its moment comes: tips are on (`on`) and the reader hasn't seen it.
    pub fn shows(&self, tip: Tip, on: bool) -> bool {
        on && !self.contains(tip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_steps_forward_and_back_within_its_steps() {
        let first = Guide::default();
        assert_eq!((first.step(), first.number()), (Step::Open, 1));
        assert!(first.is_first() && first.back().is_none());
        let last = first.next().and_then(Guide::next).expect("three steps");
        assert_eq!(last.step(), Step::Palette);
        assert!(last.is_last() && last.next().is_none());
        assert_eq!(last.back().map(Guide::step), Some(Step::Read));
    }

    #[test]
    fn the_guide_never_opens_over_files_from_the_command_line() {
        assert!(opens_at_start(false, 0));
        assert!(!opens_at_start(false, 1));
        assert!(!opens_at_start(true, 0));
    }

    #[test]
    fn each_tip_shows_once_and_the_seen_list_survives_the_settings_file() {
        let mut seen = SeenTips::parse("tab_list, from_a_later_version");
        assert!(seen.contains(Tip::TabList) && !seen.contains(Tip::Palette));
        assert!(seen.shows(Tip::Palette, true));
        assert!(!seen.shows(Tip::Palette, false));
        assert!(seen.insert(Tip::Palette));
        assert!(!seen.insert(Tip::Palette));
        assert!(!seen.shows(Tip::Palette, true));
        assert_eq!(seen.to_text(), "palette,tab_list");
        assert_eq!(SeenTips::parse(&seen.to_text()), seen);
        assert_eq!(SeenTips::parse(""), SeenTips::default());
    }
}
