//! W.8's rule: a workspace shows in one window at a time. Every way a window comes to show a workspace (opening the
//! app, a switch, a new workspace, opening one in a new window, a file another workspace has, a deleted one's
//! neighbour) asks [`Claims::claim`] first; when another window has it, that window is the answer, to focus instead.

/// Which window shows which workspace: one workspace per window, and one window per workspace.
#[derive(Debug, Clone)]
pub(crate) struct Claims<W, K> {
    /// Each window with the workspace it shows, in the order the windows opened.
    held: Vec<(W, K)>,
}

impl<W, K> Default for Claims<W, K> {
    fn default() -> Self {
        Self { held: Vec::new() }
    }
}

impl<W: Copy + PartialEq, K: Copy + PartialEq> Claims<W, K> {
    /// The window showing `workspace`, if any.
    pub(crate) fn holder(&self, workspace: K) -> Option<W> {
        self.held
            .iter()
            .find(|(_, k)| *k == workspace)
            .map(|(w, _)| *w)
    }

    /// `window` shows `workspace` from now on, and no longer the one it showed. When another window shows it, nothing
    /// changes and that window is the error.
    pub(crate) fn claim(&mut self, window: W, workspace: K) -> Result<(), W> {
        match self.holder(workspace) {
            Some(holder) if holder != window => Err(holder),
            Some(_) => Ok(()),
            None => {
                match self.held.iter_mut().find(|(w, _)| *w == window) {
                    Some(held) => held.1 = workspace,
                    None => self.held.push((window, workspace)),
                }
                Ok(())
            }
        }
    }

    /// `window` closed: its workspace is free.
    pub(crate) fn release(&mut self, window: W) {
        self.held.retain(|(w, _)| *w != window);
    }

    /// Of `ids`, those no window but `window` shows: where `window` can go.
    pub(crate) fn free_for(&self, window: W, ids: &[K]) -> Vec<K> {
        ids.iter()
            .copied()
            .filter(|&id| self.holder(id).is_none_or(|holder| holder == window))
            .collect()
    }

    /// The workspaces shown by the windows other than `main`, in the order the windows opened.
    pub(crate) fn others(&self, main: W) -> Vec<K> {
        self.held
            .iter()
            .filter(|(w, _)| *w != main)
            .map(|(_, k)| *k)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_shows_in_one_window_at_a_time() {
        let mut claims = Claims::default();
        // The main window opens on 1, a second window on 2.
        assert_eq!(claims.claim(0, 1), Ok(()));
        assert_eq!(claims.claim(7, 2), Ok(()));
        // Switching, a new window, a file's workspace: 1 is the main window's, so it is the answer.
        assert_eq!(claims.claim(7, 1), Err(0), "the window that has it");
        assert_eq!(claims.claim(9, 1), Err(0));
        assert_eq!(claims.holder(2), Some(7), "a refused claim changes nothing");
        // A switch to a free one gives up the one shown.
        assert_eq!(claims.claim(7, 3), Ok(()));
        assert_eq!(claims.holder(2), None);
        assert_eq!(claims.claim(0, 1), Ok(()), "showing it again is no change");
        assert_eq!(claims.free_for(0, &[1, 2, 3]), [1, 2]);
        assert_eq!(claims.others(0), [3]);
        // A closed window's workspace is free again.
        claims.release(7);
        assert_eq!(claims.claim(0, 3), Ok(()));
        assert!(claims.others(0).is_empty());
    }
}
