//! Named tab groups (plan R.13): tabs gathered under a coloured, named label in the strip, collapsed and expanded
//! together. The idea of naming a set of tabs is Enhanced's (`src/TabGroupsManage.cpp` saves and reopens named sets);
//! the groups live in the strip as browsers' do.
//!
//! The rules the model keeps: a group's tabs sit together in the strip; pinned tabs belong to no group; a group with
//! no tabs is gone. Collapsing hides a group's tabs from the strip and nothing else: their documents, pages and
//! unsaved changes stay as they were.

use super::model::{TabId, Tabs};

/// A group, for as long as the window keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GroupId(pub(super) u64);

/// A group's colour: named, so screen readers can say it, and the same in every theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum GroupColor {
    #[default]
    Grey,
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
}

impl GroupColor {
    /// Every colour, in the order they are offered and given to new groups.
    pub(crate) const ALL: [Self; 8] = [
        Self::Blue,
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Pink,
        Self::Purple,
        Self::Cyan,
        Self::Grey,
    ];

    /// Its name in the library and its translation key's last part.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Grey => "grey",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Cyan => "cyan",
        }
    }

    /// Its chip's fill, as `#rrggbb`, and the text drawn on it: every pair has a contrast of at least 4.5:1, and
    /// the fill reads on both light and dark title bars.
    pub(crate) fn swatch(self) -> (&'static str, &'static str) {
        const WHITE: &str = "#ffffff";
        match self {
            Self::Grey => ("#5f6368", WHITE),
            Self::Blue => ("#1a73e8", WHITE),
            Self::Red => ("#d93025", WHITE),
            Self::Yellow => ("#f9ab00", "#202124"),
            Self::Green => ("#188038", WHITE),
            Self::Pink => ("#d01884", WHITE),
            Self::Purple => ("#9334e6", WHITE),
            Self::Cyan => ("#007b83", WHITE),
        }
    }

    /// The colour named `id`; grey for a name this version doesn't know.
    pub(crate) fn parse(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|color| color.id() == id)
            .unwrap_or_default()
    }
}

/// A group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) id: GroupId,
    /// Its name; empty shows the colour alone.
    pub(crate) name: String,
    pub(crate) color: GroupColor,
    /// Its tabs are hidden from the strip.
    pub(crate) collapsed: bool,
}

impl Tabs {
    /// The groups, in no particular order (the strip's order is their tabs').
    #[cfg(test)]
    pub(crate) fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The group `id`, while it has tabs.
    pub(crate) fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// The tabs of the group `id`, in strip order.
    pub(crate) fn members(&self, id: GroupId) -> impl Iterator<Item = TabId> + '_ {
        self.tabs
            .iter()
            .filter(move |tab| tab.group == Some(id))
            .map(|tab| tab.id)
    }

    /// Whether the tab `id` is hidden in a collapsed group.
    pub(crate) fn hidden(&self, id: TabId) -> bool {
        self.get(id)
            .and_then(|tab| tab.group)
            .and_then(|group| self.group(group))
            .is_some_and(|group| group.collapsed)
    }

    /// Puts the tab `id` in a new group of its own (unpinning it first), in the first colour no group has.
    pub(crate) fn new_group(&mut self, id: TabId) -> Option<GroupId> {
        self.get(id)?;
        self.set_pinned(id, false);
        self.leave_group(id);
        self.next_group += 1;
        let group = GroupId(self.next_group);
        let color = GroupColor::ALL
            .into_iter()
            .find(|color| self.groups.iter().all(|other| other.color != *color))
            .unwrap_or(GroupColor::ALL[self.groups.len() % GroupColor::ALL.len()]);
        self.groups.push(Group {
            id: group,
            name: String::new(),
            color,
            collapsed: false,
        });
        if let Some(tab) = self.tab_mut(id) {
            tab.group = Some(group);
        }
        Some(group)
    }

    /// Takes the tab `id` out of its group, to just after the group's other tabs so they stay together.
    pub(crate) fn leave_group(&mut self, id: TabId) {
        let Some(group) = self.get(id).and_then(|tab| tab.group) else {
            return;
        };
        let last = self
            .tabs
            .iter()
            .rposition(|tab| tab.group == Some(group))
            .expect("the tab is in its group");
        let from = self.index(id).expect("the tab is open");
        let mut tab = self.tabs.remove(from);
        tab.group = None;
        self.tabs.insert(last, tab);
        self.prune_groups();
    }

    /// Names the group `id`.
    pub(crate) fn rename_group(&mut self, id: GroupId, name: &str) {
        if let Some(group) = self.group_mut(id) {
            name.trim().clone_into(&mut group.name);
        }
    }

    /// Colours the group `id`.
    pub(crate) fn recolor_group(&mut self, id: GroupId, color: GroupColor) {
        if let Some(group) = self.group_mut(id) {
            group.color = color;
        }
    }

    /// Collapses or expands the group `id`. Collapsing a group that holds the tab shown shows the tab shown last
    /// outside it; with no such tab the group stays open, as something must be shown. Says whether it changed.
    pub(crate) fn toggle_group(&mut self, id: GroupId) -> bool {
        let Some(collapsed) = self.group(id).map(|group| group.collapsed) else {
            return false;
        };
        if !collapsed && self.active().is_some_and(|tab| tab.group == Some(id)) {
            let outside = self
                .recent
                .iter()
                .copied()
                .find(|&tab| self.get(tab).is_some_and(|tab| tab.group != Some(id)));
            let Some(outside) = outside else {
                return false;
            };
            self.activate(outside);
        }
        if let Some(group) = self.group_mut(id) {
            group.collapsed = !collapsed;
        }
        true
    }

    /// Gathers `ids` into one new group (unpinning them and moving them together at the first member's position).
    pub(crate) fn group_tabs(&mut self, ids: &[TabId], name: &str) -> Option<GroupId> {
        let mut valid: Vec<TabId> = ids
            .iter()
            .copied()
            .filter(|&id| self.get(id).is_some())
            .collect();
        if valid.is_empty() {
            return None;
        }
        let first = valid.remove(0);
        let group = self.new_group(first)?;
        self.rename_group(group, name);
        let mut insert_at = self.index(first).unwrap_or(0) + 1;
        for id in valid {
            self.set_pinned(id, false);
            self.leave_group(id);
            if let Some(from) = self.index(id) {
                let mut tab = self.tabs.remove(from);
                if from < insert_at {
                    insert_at = insert_at.saturating_sub(1);
                }
                tab.group = Some(group);
                let target = insert_at.min(self.tabs.len());
                self.tabs.insert(target, tab);
                insert_at = target + 1;
            }
        }
        self.prune_groups();
        Some(group)
    }

    /// Dissolves the group `id`: its tabs stay where they are, in no group.
    pub(crate) fn ungroup(&mut self, id: GroupId) {
        for tab in &mut self.tabs {
            if tab.group == Some(id) {
                tab.group = None;
            }
        }
        self.prune_groups();
    }

    /// Closes every tab of the group `id` (pinned tabs are never in one), and with them the group.
    pub(crate) fn close_group(&mut self, id: GroupId) {
        let doomed: Vec<TabId> = self.members(id).collect();
        for tab in doomed {
            self.close(tab);
        }
    }

    /// Puts back a group of a saved session (R.9); its tabs name it as they are restored.
    pub(crate) fn restore_group(
        &mut self,
        name: String,
        color: GroupColor,
        collapsed: bool,
    ) -> GroupId {
        self.next_group += 1;
        let id = GroupId(self.next_group);
        self.groups.push(Group {
            id,
            name,
            color,
            collapsed,
        });
        id
    }

    /// Moves the tab `id` one place along the strip, in or out of groups at their edges: a grouped tab at its
    /// group's edge leaves the group where it is; a tab beside an open group joins it where it is, and passes a
    /// collapsed group whole. Otherwise it changes places with its neighbour.
    pub(crate) fn step_tab(&mut self, id: TabId, forward: bool) {
        let Some(index) = self.index(id) else {
            return;
        };
        let tab = &self.tabs[index];
        let (own, pinned) = (tab.group, tab.pinned);
        let neighbour = if forward {
            self.tabs.get(index + 1)
        } else {
            index.checked_sub(1).and_then(|at| self.tabs.get(at))
        }
        .filter(|next| next.pinned == pinned);
        let next_group = neighbour.and_then(|next| next.group);
        match (own, next_group) {
            (Some(own), next) if next != Some(own) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.group = None;
                }
                self.prune_groups();
            }
            (None, Some(next)) if self.group(next).is_some_and(|group| group.collapsed) => {
                let run: Vec<usize> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, tab)| tab.group == Some(next))
                    .map(|(at, _)| at)
                    .collect();
                let to = if forward { run[run.len() - 1] } else { run[0] };
                let tab = self.tabs.remove(index);
                self.tabs.insert(to, tab);
            }
            (None, Some(next)) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.group = Some(next);
                }
            }
            _ if neighbour.is_some() => {
                let to = if forward { index + 1 } else { index - 1 };
                self.tabs.swap(index, to);
            }
            _ => {}
        }
    }

    /// The group a tab put at `index` belongs in: the group of the tabs on both sides when they share one, so a
    /// reopened or restored tab never splits a group.
    pub(super) fn group_at(&self, index: usize) -> Option<GroupId> {
        let before = index
            .checked_sub(1)
            .and_then(|at| self.tabs.get(at))
            .and_then(|tab| tab.group);
        let after = self.tabs.get(index).and_then(|tab| tab.group);
        before.filter(|_| before == after)
    }

    /// Forgets the groups left without tabs.
    pub(crate) fn prune_groups(&mut self) {
        let tabs = &self.tabs;
        self.groups
            .retain(|group| tabs.iter().any(|tab| tab.group == Some(group.id)));
    }

    fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups.iter_mut().find(|group| group.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Tabs a to f, f shown.
    fn six() -> (Tabs, Vec<TabId>) {
        let mut tabs = Tabs::default();
        let ids: Vec<TabId> = ["a", "b", "c", "d", "e", "f"]
            .into_iter()
            .map(|name| tabs.open(PathBuf::from(format!("{name}.pdf")), 0))
            .collect();
        (tabs, ids)
    }

    fn order(tabs: &Tabs) -> String {
        tabs.all()
            .iter()
            .map(|tab| {
                let name = tab.title().trim_end_matches(".pdf").to_owned();
                match tab.group {
                    Some(group) => format!("{name}{}", group.0),
                    None => name,
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_group_keeps_its_tabs_together_as_tabs_move_in_and_out() {
        let (mut tabs, ids) = six();
        let group = tabs.new_group(ids[2]).expect("group");
        assert_eq!(order(&tabs), "a b c1 d e f");
        // d joins at the group's edge, then e.
        tabs.step_tab(ids[3], false);
        tabs.step_tab(ids[4], false);
        assert_eq!(order(&tabs), "a b c1 d1 e1 f");
        // b joins from the left; moving inside the group swaps.
        tabs.step_tab(ids[1], true);
        tabs.step_tab(ids[1], true);
        assert_eq!(order(&tabs), "a c1 b1 d1 e1 f");
        // e leaves at the right edge, staying where it is.
        tabs.step_tab(ids[4], true);
        assert_eq!(order(&tabs), "a c1 b1 d1 e f");
        // Leaving from the middle goes past the group's other tabs.
        tabs.leave_group(ids[2]);
        assert_eq!(order(&tabs), "a b1 d1 c e f");
        assert_eq!(tabs.members(group).count(), 2);
    }

    #[test]
    fn a_collapsed_group_hides_its_tabs_but_keeps_their_work() {
        let (mut tabs, ids) = six();
        let group = tabs.new_group(ids[1]).expect("group");
        tabs.step_tab(ids[2], false);
        tabs.activate(ids[2]);
        tabs.tab_mut(ids[2]).expect("c").dirty = true;
        tabs.set_page(ids[2], 40);
        assert!(tabs.toggle_group(group));
        // The tab shown was in the group: the one shown before it outside the group is shown instead.
        assert_ne!(tabs.active().and_then(|tab| tab.group), Some(group));
        assert!(tabs.hidden(ids[2]));
        let c = tabs.get(ids[2]).expect("c is still open");
        assert!(c.dirty);
        assert_eq!(c.page, 40);
        // An ungrouped tab passes a collapsed group whole.
        tabs.step_tab(ids[3], false);
        assert_eq!(order(&tabs), "a d b1 c1 e f");
        // Showing a hidden tab opens its group.
        tabs.activate(ids[2]);
        assert!(!tabs.hidden(ids[2]));
    }

    #[test]
    fn the_only_group_with_the_shown_tab_does_not_collapse() {
        let mut tabs = Tabs::default();
        let only = tabs.open(PathBuf::from("a.pdf"), 0);
        let group = tabs.new_group(only).expect("group");
        assert!(!tabs.toggle_group(group));
        assert!(!tabs.hidden(only));
    }

    #[test]
    fn a_group_goes_with_its_last_tab_and_reopened_tabs_rejoin_it() {
        let (mut tabs, ids) = six();
        let group = tabs.new_group(ids[1]).expect("group");
        tabs.step_tab(ids[2], false);
        tabs.step_tab(ids[3], false);
        assert_eq!(order(&tabs), "a b1 c1 d1 e f");
        tabs.close(ids[2]);
        tabs.reopen_closed();
        assert_eq!(order(&tabs), "a b1 c1 d1 e f");
        tabs.close_group(group);
        assert_eq!(order(&tabs), "a e f");
        assert!(tabs.groups().is_empty());
    }

    #[test]
    fn pinning_takes_a_tab_out_of_its_group_and_new_groups_get_new_colours() {
        let (mut tabs, ids) = six();
        let first = tabs.new_group(ids[1]).expect("group");
        let second = tabs.new_group(ids[4]).expect("group");
        assert_ne!(
            tabs.group(first).map(|g| g.color),
            tabs.group(second).map(|g| g.color)
        );
        tabs.set_pinned(ids[1], true);
        assert_eq!(tabs.get(ids[1]).and_then(|tab| tab.group), None);
        assert!(tabs.group(first).is_none());
        tabs.rename_group(second, "  Thesis ");
        assert_eq!(tabs.group(second).map(|g| g.name.as_str()), Some("Thesis"));
        assert_eq!(GroupColor::parse("cyan"), GroupColor::Cyan);
        assert_eq!(GroupColor::parse("mauve"), GroupColor::Grey);
    }
}
