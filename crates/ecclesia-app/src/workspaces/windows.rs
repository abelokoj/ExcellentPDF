//! The app's windows and the workspace each shows (W.8): the main window, and those "Open workspace in new window"
//! opened. All of them run on the one UI thread, so the registry is that thread's. [`super::claims::Claims`] keeps
//! the rule; this keeps what acting on it needs: each window's ref, to focus it, and its [`Workspaces`], to hand it
//! files and tabs.

use std::cell::RefCell;

use ecclesia_store::workspace::WorkspaceId;
use nsh_core::element::desktop::WindowRef;

use super::Workspaces;
use super::claims::Claims;

/// A window's number, the app's own: the main window's is [`MAIN`].
pub(crate) type WindowNo = u64;

/// The main window's number.
pub(crate) const MAIN: WindowNo = 0;

/// An open window.
struct Entry {
    no: WindowNo,
    /// `None` until NSH opened it.
    window: Option<WindowRef>,
    workspaces: Workspaces,
}

#[derive(Default)]
struct Registry {
    claims: Claims<WindowNo, WorkspaceId>,
    open: Vec<Entry>,
    last: WindowNo,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::default();
}

fn with<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    REGISTRY.with(|registry| f(&mut registry.borrow_mut()))
}

/// A number for a new window.
pub(super) fn next_number() -> WindowNo {
    with(|r| {
        r.last += 1;
        r.last
    })
}

/// Window `no` opened with `workspaces`; it shows nothing yet ([`claim`]).
pub(super) fn add(no: WindowNo, workspaces: Workspaces) {
    with(|r| {
        r.open.retain(|entry| entry.no != no);
        r.open.push(Entry {
            no,
            window: None,
            workspaces,
        });
    });
}

/// The ref of window `no`, to focus it.
pub(super) fn set_window(no: WindowNo, window: WindowRef) {
    with(|r| {
        if let Some(entry) = r.open.iter_mut().find(|entry| entry.no == no) {
            entry.window = Some(window);
        }
    });
}

/// Window `no` closed: its workspace is free.
pub(super) fn remove(no: WindowNo) {
    with(|r| {
        r.claims.release(no);
        r.open.retain(|entry| entry.no != no);
    });
}

/// Window `no` shows `workspace` from now on, unless another window does: then that window's number.
pub(super) fn claim(no: WindowNo, workspace: WorkspaceId) -> Result<(), WindowNo> {
    with(|r| r.claims.claim(no, workspace))
}

/// The window showing `workspace`, other than `except`.
pub(super) fn holder(workspace: WorkspaceId, except: WindowNo) -> Option<WindowNo> {
    with(|r| r.claims.holder(workspace)).filter(|&no| no != except)
}

/// The workspaces of window `no`.
pub(super) fn workspaces(no: WindowNo) -> Option<Workspaces> {
    with(|r| {
        r.open
            .iter()
            .find(|entry| entry.no == no)
            .map(|entry| entry.workspaces)
    })
}

/// Brings window `no` to the front with the keyboard.
pub(super) fn focus(no: WindowNo) {
    let window = with(|r| {
        r.open
            .iter()
            .find(|entry| entry.no == no)
            .and_then(|entry| entry.window.clone())
    });
    if let Some(window) = window {
        window.focus();
    }
}

/// Of `ids`, those no window but `no` shows.
pub(super) fn free_for(no: WindowNo, ids: &[WorkspaceId]) -> Vec<WorkspaceId> {
    with(|r| r.claims.free_for(no, ids))
}

/// The workspaces the windows other than the main one show, in the order they opened: what the next start opens.
pub(super) fn others() -> Vec<WorkspaceId> {
    with(|r| r.claims.others(MAIN))
}
