//! Quick note from a global shortcut (plan N.10).
//!
//! Manages the `"Inbox.expdf"` notebook and abstracts system-wide shortcut registration
//! (wired to NSH's `register_global_shortcut` / `set_tray` on desktop) so conflicting shortcuts
//! are cleanly reported to settings and triggering the shortcut opens a fresh writable Inbox page
//! within 300 ms.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use excellent_core::notebook::{Notebook, Orientation, PageSize, PaperTemplateId, Result};

use crate::notebook::pages::{InsertPageSource, InsertPosition, insert_page_in_notebook};

/// Default system-wide shortcut for opening a quick note.
pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+N";

/// Target latency budget for opening a quick note page (plan N.10 Done when).
pub const QUICK_NOTE_LATENCY_BUDGET_MS: u64 = 300;

/// Whether the app keeps a system tray icon when the main window is closed (N.10 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TrayBehavior {
    /// Do not keep a tray icon (default; respects user preference).
    #[default]
    Disabled,
    /// Keep a tray icon so the global shortcut still works with the main window closed.
    KeepInTray,
}

/// Settings for Quick Note (plan N.10).
#[derive(Debug, Clone, PartialEq)]
pub struct QuickNoteConfig {
    /// Global shortcut accelerator string (e.g., `"Ctrl+Alt+N"`).
    pub shortcut: String,
    /// Path to the Inbox `.expdf` notebook.
    pub inbox_path: PathBuf,
    /// Paper template applied to new quick-note pages.
    pub default_template: PaperTemplateId,
    /// Page size for new quick-note pages.
    pub default_page_size: PageSize,
    /// Page orientation for new quick-note pages.
    pub default_orientation: Orientation,
    /// Whether to keep a tray icon when the main window closes.
    pub tray: TrayBehavior,
    /// Small popup window size `(width_px, height_px)`.
    pub window_size: (u32, u32),
}

impl QuickNoteConfig {
    /// Creates default quick-note settings pointing at `inbox_path`.
    #[must_use]
    pub fn with_inbox_path(inbox_path: impl Into<PathBuf>) -> Self {
        Self {
            shortcut: DEFAULT_SHORTCUT.into(),
            inbox_path: inbox_path.into(),
            default_template: PaperTemplateId::Lined,
            default_page_size: PageSize::A5,
            default_orientation: Orientation::Portrait,
            tray: TrayBehavior::Disabled,
            window_size: (520, 680),
        }
    }
}

/// Result of registering the system-wide quick-note shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutStatus {
    /// Shortcut registered cleanly with the desktop environment.
    Registered(String),
    /// Shortcut was already claimed by another application (`register_global_shortcut` returned false).
    Conflict {
        /// The shortcut string that could not be claimed.
        shortcut: String,
    },
    /// No shortcut is currently registered.
    Unregistered,
}

/// Platform hook implemented by the desktop shell (`nsh_core::element::desktop::register_global_shortcut`).
pub trait GlobalShortcutHost {
    /// Attempts to register `shortcut` system-wide; returns `true` if claimed, `false` if already taken.
    fn register_global_shortcut(&mut self, shortcut: &str) -> bool;

    /// Unregisters a previously registered `shortcut`.
    fn unregister_global_shortcut(&mut self, shortcut: &str);

    /// Updates tray icon visibility according to `behavior`.
    fn set_tray_behavior(&mut self, behavior: TrayBehavior);
}

/// Result returned when the quick-note shortcut triggers and prepares a writable Inbox page.
#[derive(Debug, Clone, PartialEq)]
pub struct QuickNoteTriggerResult {
    /// Path to the Inbox `.expdf` file on disk.
    pub inbox_path: PathBuf,
    /// Zero-based index of the newly appended writable page.
    pub page_index: usize,
    /// Total pages now in the Inbox notebook.
    pub page_count: usize,
    /// Wall-clock time taken to prepare and persist the writable page.
    pub elapsed: Duration,
}

/// Coordinator managing the Inbox notebook and global shortcut registration (N.10).
#[derive(Debug)]
pub struct QuickNoteInbox {
    config: QuickNoteConfig,
    status: ShortcutStatus,
    cached_notebook: Option<Notebook>,
}

impl QuickNoteInbox {
    /// Creates a new Quick Note coordinator with `config`.
    #[must_use]
    pub fn new(config: QuickNoteConfig) -> Self {
        Self {
            config,
            status: ShortcutStatus::Unregistered,
            cached_notebook: None,
        }
    }

    /// Current configuration.
    #[must_use]
    pub fn config(&self) -> &QuickNoteConfig {
        &self.config
    }

    /// Current global shortcut registration status.
    #[must_use]
    pub fn shortcut_status(&self) -> &ShortcutStatus {
        &self.status
    }

    /// Applies the configured shortcut and tray behavior through `host`.
    pub fn bind_shortcut<H: GlobalShortcutHost>(&mut self, host: &mut H) -> &ShortcutStatus {
        if let ShortcutStatus::Registered(prev) = &self.status {
            host.unregister_global_shortcut(prev);
        }
        host.set_tray_behavior(self.config.tray);
        if host.register_global_shortcut(&self.config.shortcut) {
            self.status = ShortcutStatus::Registered(self.config.shortcut.clone());
        } else {
            self.status = ShortcutStatus::Conflict {
                shortcut: self.config.shortcut.clone(),
            };
        }
        &self.status
    }

    /// Updates the configured shortcut string and re-attempts registration via `host`.
    pub fn change_shortcut<H: GlobalShortcutHost>(
        &mut self,
        new_shortcut: impl Into<String>,
        host: &mut H,
    ) -> &ShortcutStatus {
        self.config.shortcut = new_shortcut.into();
        self.bind_shortcut(host)
    }

    /// Updates tray behavior and notifies `host`.
    pub fn set_tray<H: GlobalShortcutHost>(&mut self, behavior: TrayBehavior, host: &mut H) {
        self.config.tray = behavior;
        host.set_tray_behavior(behavior);
    }

    /// Handles a quick-note shortcut trigger: opens (or creates) `"Inbox.expdf"`, appends a fresh
    /// writable page, saves atomically, and returns the opened session details.
    ///
    /// # Errors
    ///
    /// Returns an error if creating, reading, modifying, or atomically saving the Inbox fails.
    pub fn trigger(&mut self) -> Result<QuickNoteTriggerResult> {
        let start = Instant::now();
        let mut nb = if let Some(cached) = self.cached_notebook.take() {
            cached
        } else if self.config.inbox_path.exists() {
            Notebook::open(&self.config.inbox_path)?
        } else {
            let fresh = Notebook::new_blank(
                "Inbox",
                self.config.default_page_size,
                self.config.default_orientation,
                self.config.default_template,
            )?;
            fresh.save_atomic(&self.config.inbox_path)?;
            let page_count = fresh.metadata.pages.len();
            self.cached_notebook = Some(fresh);
            return Ok(QuickNoteTriggerResult {
                inbox_path: self.config.inbox_path.clone(),
                page_index: 0,
                page_count,
                elapsed: start.elapsed(),
            });
        };

        let last_idx = nb.metadata.pages.len().saturating_sub(1);
        let page_index = insert_page_in_notebook(
            &mut nb,
            InsertPosition::After(last_idx),
            &InsertPageSource::Blank {
                size: self.config.default_page_size,
                orientation: self.config.default_orientation,
            },
        )?;
        if let Some(meta) = nb.metadata.pages.get_mut(page_index) {
            meta.template = self.config.default_template;
        }
        nb.save_atomic(&self.config.inbox_path)?;
        let page_count = nb.metadata.pages.len();
        self.cached_notebook = Some(nb);

        Ok(QuickNoteTriggerResult {
            inbox_path: self.config.inbox_path.clone(),
            page_index,
            page_count,
            elapsed: start.elapsed(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[derive(Default)]
    struct MockHost {
        taken: HashSet<String>,
        registered: HashSet<String>,
        tray: TrayBehavior,
    }

    impl GlobalShortcutHost for MockHost {
        fn register_global_shortcut(&mut self, shortcut: &str) -> bool {
            if self.taken.contains(shortcut) {
                return false;
            }
            self.registered.insert(shortcut.to_owned());
            true
        }

        fn unregister_global_shortcut(&mut self, shortcut: &str) {
            self.registered.remove(shortcut);
        }

        fn set_tray_behavior(&mut self, behavior: TrayBehavior) {
            self.tray = behavior;
        }
    }

    #[test]
    fn quick_note_reports_shortcut_conflicts_and_opens_writable_page_under_300ms() {
        let tmp = std::env::temp_dir().join(format!("excellent-quicknote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let inbox_path = tmp.join("Inbox.expdf");

        let mut host = MockHost::default();
        host.taken.insert(DEFAULT_SHORTCUT.into());

        let mut qn = QuickNoteInbox::new(QuickNoteConfig::with_inbox_path(&inbox_path));
        assert_eq!(
            qn.bind_shortcut(&mut host),
            &ShortcutStatus::Conflict {
                shortcut: DEFAULT_SHORTCUT.into()
            }
        );

        // User picks an available shortcut and enables tray icon.
        assert_eq!(
            qn.change_shortcut("Ctrl+Shift+Alt+N", &mut host),
            &ShortcutStatus::Registered("Ctrl+Shift+Alt+N".into())
        );
        qn.set_tray(TrayBehavior::KeepInTray, &mut host);
        assert_eq!(host.tray, TrayBehavior::KeepInTray);

        // First trigger creates Inbox.expdf; second trigger appends a new page.
        let first = qn.trigger().expect("first trigger");
        assert_eq!((first.page_index, first.page_count), (0, 1));
        assert!(
            first.elapsed < Duration::from_millis(QUICK_NOTE_LATENCY_BUDGET_MS),
            "first trigger took {:?} (budget 300ms)",
            first.elapsed
        );

        let second = qn.trigger().expect("second trigger");
        assert_eq!((second.page_index, second.page_count), (1, 2));
        assert!(
            second.elapsed < Duration::from_millis(QUICK_NOTE_LATENCY_BUDGET_MS),
            "second trigger took {:?} (budget 300ms)",
            second.elapsed
        );

        let reopened = Notebook::open(&inbox_path).expect("reopen inbox");
        assert_eq!(reopened.metadata.pages.len(), 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
