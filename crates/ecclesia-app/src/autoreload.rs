//! SumatraPDF-style automatic reload when an open document changes on disk outside Ecclesia PDF.
//!
//! Polls the active document's file metadata (and any sibling `.synctex.gz` / `.synctex` sidecar)
//! on a lightweight interval, waits for writers such as `pdflatex`, `xelatex`, `lualatex`, or
//! `typst` to finish writing (`%%EOF` present and metadata settled across consecutive ticks),
//! ignores Ecclesia PDF's own saves, and either reloads silently in place or shows a non-blocking
//! confirmation banner above the pages.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ecclesia_core::shared::registry;
use nsh_base::theme;
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::editing::Editing;
use crate::reading_theme_bar::bar_button;
use crate::settings::{Live, Settings};
use crate::tabs::TabState;

/// `settings.toml` key controlling external file auto-reload behaviour.
pub(crate) const AUTO_RELOAD_MODE: &str = "auto_reload_mode";

/// How often each pane checks its active file's metadata for external modifications.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Number of trailing bytes scanned for the PDF `%%EOF` end-of-file marker.
const PDF_EOF_TAIL_BYTES: u64 = 1024;

/// Maximum consecutive retries after a transient read or lock error before waiting for a newer
/// on-disk modification.
const MAX_RETRIES: u8 = 6;

/// How Ecclesia PDF reacts when an open document changes on disk outside the app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum AutoReloadMode {
    /// Reload automatically without prompting, preserving page, scroll, zoom, and SyncTeX
    /// (SumatraPDF default).
    #[default]
    Silent,
    /// Show a non-blocking banner asking the user to Reload or Ignore.
    Ask,
    /// Do not watch for external file modifications.
    Off,
}

impl AutoReloadMode {
    /// Parses a persisted `settings.toml` value into an [`AutoReloadMode`], defaulting to
    /// [`AutoReloadMode::Silent`].
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("ask") => Self::Ask,
            Some("off") => Self::Off,
            _ => Self::Silent,
        }
    }

    /// Serialized string written to `settings.toml` under [`AUTO_RELOAD_MODE`].
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Silent => "silent",
            Self::Ask => "ask",
            Self::Off => "off",
        }
    }

    /// Reads the configured [`AutoReloadMode`] from `settings`.
    pub(crate) fn from_settings(settings: &Settings) -> Self {
        Self::parse(settings.text(AUTO_RELOAD_MODE))
    }
}

/// Lightweight on-disk change fingerprint for the active file and its optional SyncTeX sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: SystemTime,
    synctex_len: Option<u64>,
    synctex_modified: Option<SystemTime>,
}

/// Mutable watcher bookkeeping held inside [`AutoReloadState`].
#[derive(Debug, Default)]
struct WatchBookkeeping {
    /// Last resolved path watched in this pane.
    path: Option<PathBuf>,
    /// Last accepted on-disk stamp for `path`.
    baseline: Option<FileStamp>,
    /// Candidate new stamp waiting one poll interval to confirm the write has settled.
    pending: Option<FileStamp>,
    /// Whether a background reload task is currently running for this pane.
    reloading: bool,
    /// Consecutive transient reload errors on the current stamp.
    retries: u8,
}

/// Per-pane watcher state for external file modifications and reload confirmation prompts.
#[derive(Clone, Copy)]
pub(crate) struct AutoReloadState {
    tabs: TabState,
    editing: Editing,
    live: Live,
    /// Resolved path awaiting user confirmation in [`autoreload_bar`], if any.
    prompt: Signal<Option<PathBuf>>,
    watch: Stored<WatchBookkeeping>,
}

impl AutoReloadState {
    /// Starts watching `tabs`' active document for external disk changes while the current
    /// component lives.
    pub(crate) fn new(tabs: TabState, editing: Editing, live: Live, canvas: ElementRef) -> Self {
        let state = Self {
            tabs,
            editing,
            live,
            prompt: signal(None),
            watch: stored(WatchBookkeeping::default()),
        };
        // Whenever the shown tab changes, immediately record the new file's baseline stamp and
        // clear any stale confirmation banner from the previous tab.
        effect(move || {
            let shown = tabs.shown_path();
            untracked(|| state.on_shown_changed(shown.as_deref()));
        });
        set_interval(POLL_INTERVAL, move || {
            state.poll_tick(canvas);
        });
        state
    }

    /// The resolved path currently awaiting confirmation in the banner, if any (tracked).
    pub(crate) fn pending_prompt(self) -> Option<PathBuf> {
        self.prompt.get()
    }

    /// Whether a reload confirmation banner is currently shown for this pane (tracked).
    pub(crate) fn has_prompt(self) -> bool {
        self.prompt.with(Option::is_some)
    }

    /// Confirms the pending banner prompt and reloads the modified file from disk.
    pub(crate) fn confirm_reload(self, canvas: ElementRef) {
        if let Some(resolved) = self.prompt.get_untracked() {
            self.trigger_reload(resolved, canvas);
        }
    }

    /// Dismisses the confirmation banner without reloading this version of the file.
    pub(crate) fn dismiss(self) {
        self.prompt.set(None);
    }

    /// Reloads `resolved` in place on a background thread and notifies views and `canvas`.
    pub(crate) fn trigger_reload(self, resolved: PathBuf, canvas: ElementRef) {
        self.trigger_reload_inner(resolved, canvas, None);
    }

    fn trigger_reload_inner(
        self,
        resolved: PathBuf,
        canvas: ElementRef,
        rollback_baseline: Option<FileStamp>,
    ) {
        self.prompt.set(None);
        let already_reloading = self.watch.with(|watch| {
            if watch.reloading {
                true
            } else {
                watch.reloading = true;
                false
            }
        });
        if already_reloading {
            return;
        }
        let target = resolved.clone();
        spawn(async move {
            let outcome = blocking(move || registry().reload(&target)).await;
            self.watch.with(|watch| {
                watch.reloading = false;
            });
            match outcome {
                Ok(Ok(_reloaded)) => {
                    self.watch.with(|watch| {
                        watch.retries = 0;
                        if watch.path.as_deref() == Some(resolved.as_path())
                            && let Some(fresh) = read_stamp(&resolved)
                        {
                            watch.baseline = Some(fresh);
                        }
                    });
                    self.editing.on_external_reload(&resolved);
                    canvas.command("reload", JsonValue::Null);
                }
                Ok(Err(err)) => {
                    log::warn!("autoreload: cannot reload {}: {err}", resolved.display());
                    self.maybe_rollback_for_retry(&resolved, rollback_baseline);
                }
                Err(err) => {
                    log::warn!(
                        "autoreload: reload task failed for {}: {err}",
                        resolved.display()
                    );
                    self.maybe_rollback_for_retry(&resolved, rollback_baseline);
                }
            }
        });
    }

    /// Restores `rollback_baseline` so the next poll tick retries when a compiler briefly held a
    /// lock on the file, up to [`MAX_RETRIES`] attempts.
    fn maybe_rollback_for_retry(self, resolved: &Path, rollback_baseline: Option<FileStamp>) {
        let Some(previous) = rollback_baseline else {
            return;
        };
        self.watch.with(|watch| {
            if watch.path.as_deref() != Some(resolved) {
                return;
            }
            if watch.retries < MAX_RETRIES {
                watch.retries += 1;
                watch.baseline = Some(previous);
                watch.pending = None;
            }
        });
    }

    fn on_shown_changed(self, shown: Option<&Path>) {
        let resolved = shown.map(crate::expdf::resolve_open_path);
        let changed = self.watch.with(|watch| {
            if watch.path == resolved {
                return false;
            }
            watch.path = resolved.clone();
            watch.baseline = resolved.as_deref().and_then(read_stamp);
            watch.pending = None;
            watch.retries = 0;
            true
        });
        if changed && self.prompt.with_untracked(Option::is_some) {
            self.prompt.set(None);
        }
    }

    fn poll_tick(self, canvas: ElementRef) {
        let mode = untracked(|| self.live.read(AutoReloadMode::from_settings));
        if mode == AutoReloadMode::Off {
            if self.prompt.with_untracked(Option::is_some) {
                self.prompt.set(None);
            }
            self.watch.with(|watch| {
                watch.baseline = None;
                watch.pending = None;
                watch.retries = 0;
            });
            return;
        }
        let Some(shown) = untracked(|| self.tabs.shown_path()) else {
            return;
        };
        let resolved = crate::expdf::resolve_open_path(&shown);
        if self.watch.with(|watch| watch.reloading) {
            return;
        }
        let Some(current_stamp) = read_stamp(&resolved) else {
            return;
        };

        let decision = self.watch.with(|watch| {
            if watch.path.as_deref() != Some(resolved.as_path()) {
                watch.path = Some(resolved.clone());
                watch.baseline = Some(current_stamp);
                watch.pending = None;
                watch.retries = 0;
                return PollAction::ClearPrompt;
            }
            let Some(baseline) = watch.baseline else {
                watch.baseline = Some(current_stamp);
                watch.pending = None;
                return PollAction::None;
            };
            if current_stamp == baseline {
                watch.pending = None;
                watch.retries = 0;
                return PollAction::None;
            }
            let same_synctex = baseline.synctex_len == current_stamp.synctex_len
                && baseline.synctex_modified == current_stamp.synctex_modified;
            if same_synctex
                && self
                    .editing
                    .was_recently_saved(&resolved, current_stamp.modified)
            {
                watch.baseline = Some(current_stamp);
                watch.pending = None;
                watch.retries = 0;
                return PollAction::None;
            }
            if watch.pending != Some(current_stamp) {
                watch.pending = Some(current_stamp);
                return PollAction::None;
            }
            // Stamp remained stable across two consecutive ticks: verify the file is completely
            // written before reloading or prompting.
            if !is_pdf_write_complete(&resolved) {
                return PollAction::None;
            }
            watch.baseline = Some(current_stamp);
            watch.pending = None;
            if mode == AutoReloadMode::Silent && !self.editing.has_unsaved_changes(&resolved) {
                PollAction::Reload {
                    previous_baseline: baseline,
                }
            } else {
                PollAction::Prompt
            }
        });

        match decision {
            PollAction::None => {}
            PollAction::ClearPrompt => {
                if self.prompt.with_untracked(Option::is_some) {
                    self.prompt.set(None);
                }
            }
            PollAction::Reload { previous_baseline } => {
                self.trigger_reload_inner(resolved, canvas, Some(previous_baseline));
            }
            PollAction::Prompt => {
                self.prompt.set(Some(resolved));
            }
        }
    }
}

/// Outcome of a single watcher poll tick.
#[derive(Debug, Clone, Copy)]
enum PollAction {
    None,
    ClearPrompt,
    Reload { previous_baseline: FileStamp },
    Prompt,
}

/// Non-blocking confirmation banner rendered above the pages when the active document changed on
/// disk and requires confirmation (either [`AutoReloadMode::Ask`] or unsaved local edits exist).
pub(crate) fn autoreload_bar(state: AutoReloadState, canvas: ElementRef) -> impl IntoElement {
    let message = move || {
        let Some(path) = state.pending_prompt() else {
            return String::new();
        };
        let _ = state.editing.revision();
        let name = file_name(&path);
        if state.editing.has_unsaved_changes(&path) {
            t!("autoreload.banner_unsaved", name = name)
        } else {
            t!("autoreload.banner", name = name)
        }
    };
    view()
        .label(t!("autoreload.label"))
        .min_h(px(40.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py(px(6.0))
        .bg(move || theme().accent)
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            text(message)
                .flex_1()
                .number_of_lines(2u32)
                .text_size(px(13.0))
                .text_color(move || theme().accent_foreground),
        )
        .child(bar_button("autoreload.ignore", false, move || {
            state.dismiss()
        }))
        .child(bar_button("autoreload.reload", true, move || {
            state.confirm_reload(canvas)
        }))
}

/// Returns whether the file at `path` looks completely written on disk.
///
/// For `.pdf` files, verifies that `len > 0` and that the trailing 1024 bytes contain `b"%%EOF"`,
/// preventing partial reloads mid-compile from `pdflatex`, `xelatex`, `lualatex`, or `typst`. For
/// non-PDF formats, verifies that `len > 0`.
pub(crate) fn is_pdf_write_complete(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    let len = meta.len();
    if len == 0 {
        return false;
    }
    if !is_pdf_extension(path) {
        return true;
    }
    let start = len.saturating_sub(PDF_EOF_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let tail_len = usize::try_from(len - start).unwrap_or(1024);
    let mut tail = Vec::with_capacity(tail_len);
    if file.read_to_end(&mut tail).is_err() {
        return false;
    }
    tail.windows(b"%%EOF".len())
        .any(|window| window == b"%%EOF")
}

/// Reads the current [`FileStamp`] for `path` and any sibling SyncTeX file.
fn read_stamp(path: &Path) -> Option<FileStamp> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let (synctex_len, synctex_modified) = if is_pdf_extension(path) {
        synctex_stamp(path).map_or((None, None), |(len, time)| (Some(len), Some(time)))
    } else {
        (None, None)
    };
    Some(FileStamp {
        len: meta.len(),
        modified,
        synctex_len,
        synctex_modified,
    })
}

/// Reads the size and modified time of a sibling `.synctex.gz` or `.synctex` file if present.
fn synctex_stamp(pdf_path: &Path) -> Option<(u64, SystemTime)> {
    for ext in ["synctex.gz", "synctex"] {
        let candidate = pdf_path.with_extension(ext);
        if let Ok(meta) = fs::metadata(&candidate)
            && let Ok(modified) = meta.modified()
        {
            return Some((meta.len(), modified));
        }
    }
    None
}

/// Whether `path` carries a `.pdf` extension (case-insensitive).
fn is_pdf_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

/// Display name of `path` for the reload confirmation banner.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_reload_mode_round_trips_and_defaults_to_silent() {
        assert_eq!(AutoReloadMode::default(), AutoReloadMode::Silent);
        assert_eq!(AutoReloadMode::parse(None), AutoReloadMode::Silent);
        assert_eq!(
            AutoReloadMode::parse(Some("unknown")),
            AutoReloadMode::Silent
        );
        for mode in [
            AutoReloadMode::Silent,
            AutoReloadMode::Ask,
            AutoReloadMode::Off,
        ] {
            assert_eq!(
                AutoReloadMode::parse(Some(mode.as_str())),
                mode,
                "round-trip for {mode:?}"
            );
        }

        let mut settings = Settings::default();
        assert_eq!(
            AutoReloadMode::from_settings(&settings),
            AutoReloadMode::Silent
        );
        settings.set_text(AUTO_RELOAD_MODE, Some("ask"));
        assert_eq!(
            AutoReloadMode::from_settings(&settings),
            AutoReloadMode::Ask
        );
        settings.set_text(AUTO_RELOAD_MODE, Some("off"));
        assert_eq!(
            AutoReloadMode::from_settings(&settings),
            AutoReloadMode::Off
        );
    }

    #[test]
    fn pdf_write_completion_rejects_empty_or_truncated_and_accepts_eof() {
        let dir =
            std::env::temp_dir().join(format!("ecclesiapdf-autoreload-eof-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");

        let pdf = dir.join("paper.pdf");
        assert!(!is_pdf_write_complete(&pdf), "missing file is incomplete");

        fs::write(&pdf, b"").expect("write empty");
        assert!(!is_pdf_write_complete(&pdf), "empty PDF is incomplete");

        fs::write(&pdf, b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n")
            .expect("write partial");
        assert!(
            !is_pdf_write_complete(&pdf),
            "truncated PDF without %%EOF is incomplete"
        );

        fs::write(
            &pdf,
            b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n",
        )
        .expect("write complete");
        assert!(
            is_pdf_write_complete(&pdf),
            "PDF ending with %%EOF is complete"
        );

        let epub = dir.join("book.epub");
        fs::write(&epub, b"").expect("write empty epub");
        assert!(!is_pdf_write_complete(&epub), "empty non-PDF is incomplete");
        fs::write(&epub, b"PK\x03\x04content").expect("write non-empty epub");
        assert!(
            is_pdf_write_complete(&epub),
            "non-empty non-PDF is complete"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
