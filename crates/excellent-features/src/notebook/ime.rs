//! Canvas input method (IME) composition state and bridge model (plan U.8).
//!
//! Platform hosts (Windows IMM32 `WM_IME_STARTCOMPOSITION`/`WM_IME_COMPOSITION`/TSF,
//! macOS `NSTextInputClient`, GTK `GtkIMContext`) translate OS composition events into
//! [`ImeEvent`]s fed into [`ImeBridge`]. In-canvas text editors (N.6 typed text blocks,
//! A.6 text boxes, O.5/O.11 form/text editors) read preedit clauses for inline rendering and
//! supply the caret rectangle so the OS places the CJK candidate window directly at the caret.

use excellent_mupdf::PageRect;

/// Visual underline style of a composition clause during IME input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClauseStyle {
    /// Raw unconverted input (thin dotted/dashed underline).
    Input,
    /// Clause currently targeted for candidate selection (thick solid underline).
    TargetConverted,
    /// Already-converted clause awaiting final commit (thin solid underline).
    Converted,
    /// Targeted clause not yet converted.
    TargetNotConverted,
}

/// A UTF-8 byte span within the active preedit string with an underline style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionClause {
    /// Start byte offset within the preedit string.
    pub start: usize,
    /// End byte offset within the preedit string.
    pub end: usize,
    /// Visual underline style.
    pub style: ClauseStyle,
}

/// Active uncommitted IME composition state (preedit text).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImeComposition {
    /// Uncommitted preedit text shown inline at the caret.
    pub preedit: String,
    /// Caret byte offset within `preedit` (always on a UTF-8 char boundary).
    pub cursor_bytes: usize,
    /// Styled clauses describing conversion segments (e.g., Japanese Bunsetsu segments).
    pub clauses: Vec<CompositionClause>,
}

/// Platform-agnostic IME event delivered by Windows IMM32/TSF, macOS, or GTK hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// Composition session started (`WM_IME_STARTCOMPOSITION` / `preedit-start`).
    Start,
    /// Preedit text, cursor, or clauses updated (`WM_IME_COMPOSITION` GCS_COMPSTR / `setMarkedText`).
    Preedit {
        /// Current uncommitted preedit string.
        text: String,
        /// Cursor byte offset inside `text`.
        cursor_bytes: usize,
        /// Conversion clause spans.
        clauses: Vec<CompositionClause>,
    },
    /// Finalized text committed (`WM_IME_COMPOSITION` GCS_RESULTSTR / `insertText` / `commit`).
    Commit(String),
    /// Composition ended or cancelled (`WM_IME_ENDCOMPOSITION` / `unmarkText` / `preedit-end`).
    End,
}

/// Outcome produced by [`ImeBridge::handle_event`] for the focused canvas text editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeAction {
    /// No visible document text change (preedit updated or session started/ended).
    PreeditChanged,
    /// Commit finalized text into the target editor at its caret, clearing any preedit.
    CommitText {
        /// Identifier of the focused in-canvas editor receiving the text.
        target_id: String,
        /// Finalized UTF-8 text to insert.
        text: String,
    },
    /// Ignored because no in-canvas editor currently has IME focus.
    Ignored,
}

/// Bridges OS input method events to an active in-canvas text editor (U.8).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImeBridge {
    focused_target: Option<String>,
    caret_rect: Option<PageRect>,
    composition: Option<ImeComposition>,
}

impl ImeBridge {
    /// Creates an unfocused IME bridge.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Focuses an in-canvas editor (`target_id`) at `caret_rect` in page coordinates.
    pub fn focus(&mut self, target_id: impl Into<String>, caret_rect: PageRect) {
        self.focused_target = Some(target_id.into());
        self.caret_rect = Some(caret_rect);
        self.composition = None;
    }

    /// Clears focus and cancels any active composition.
    pub fn blur(&mut self) {
        self.focused_target = None;
        self.caret_rect = None;
        self.composition = None;
    }

    /// Updates the caret anchor rectangle in page coordinates so the host places the IME candidate
    /// window right beside the caret.
    pub fn update_caret_rect(&mut self, rect: PageRect) {
        if self.focused_target.is_some() {
            self.caret_rect = Some(rect);
        }
    }

    /// Returns the currently focused editor ID, if any.
    #[must_use]
    pub fn focused_target(&self) -> Option<&str> {
        self.focused_target.as_deref()
    }

    /// Returns the candidate window anchor rectangle in page points, if focused.
    #[must_use]
    pub fn candidate_window_rect(&self) -> Option<PageRect> {
        self.caret_rect
    }

    /// Returns the active uncommitted preedit composition, if any.
    #[must_use]
    pub fn composition(&self) -> Option<&ImeComposition> {
        self.composition.as_ref()
    }

    /// Whether an IME composition session is currently active.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    /// Processes an incoming platform [`ImeEvent`] and returns the resulting [`ImeAction`].
    pub fn handle_event(&mut self, event: ImeEvent) -> ImeAction {
        let Some(target_id) = self.focused_target.clone() else {
            return ImeAction::Ignored;
        };

        match event {
            ImeEvent::Start => {
                self.composition = Some(ImeComposition::default());
                ImeAction::PreeditChanged
            }
            ImeEvent::Preedit {
                text,
                cursor_bytes,
                clauses,
            } => {
                let safe_cursor = clamp_to_char_boundary(&text, cursor_bytes);
                let safe_clauses = clauses
                    .into_iter()
                    .filter_map(|c| {
                        let start = clamp_to_char_boundary(&text, c.start);
                        let end = clamp_to_char_boundary(&text, c.end);
                        (start <= end).then_some(CompositionClause {
                            start,
                            end,
                            style: c.style,
                        })
                    })
                    .collect();
                self.composition = Some(ImeComposition {
                    preedit: text,
                    cursor_bytes: safe_cursor,
                    clauses: safe_clauses,
                });
                ImeAction::PreeditChanged
            }
            ImeEvent::Commit(text) => {
                self.composition = None;
                ImeAction::CommitText { target_id, text }
            }
            ImeEvent::End => {
                self.composition = None;
                ImeAction::PreeditChanged
            }
        }
    }
}

fn clamp_to_char_boundary(text: &str, idx: usize) -> usize {
    let mut pos = idx.min(text.len());
    while pos > 0 && !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARET: PageRect = PageRect {
        x0: 120.0,
        y0: 200.0,
        x1: 121.0,
        y1: 214.0,
    };

    #[test]
    fn cjk_chinese_japanese_and_korean_composition_round_trip() {
        let mut ime = ImeBridge::new();
        assert_eq!(ime.handle_event(ImeEvent::Start), ImeAction::Ignored);

        ime.focus("block-1", CARET);
        assert_eq!(ime.candidate_window_rect(), Some(CARET));

        // Chinese Pinyin: "zhongwen" -> commit "中文"
        assert_eq!(ime.handle_event(ImeEvent::Start), ImeAction::PreeditChanged);
        assert!(ime.is_composing());
        ime.handle_event(ImeEvent::Preedit {
            text: "zhong'wen".into(),
            cursor_bytes: 9,
            clauses: vec![CompositionClause {
                start: 0,
                end: 9,
                style: ClauseStyle::Input,
            }],
        });
        assert_eq!(ime.composition().expect("comp").preedit, "zhong'wen");
        let commit_zh = ime.handle_event(ImeEvent::Commit("中文".into()));
        assert_eq!(
            commit_zh,
            ImeAction::CommitText {
                target_id: "block-1".into(),
                text: "中文".into()
            }
        );
        assert!(!ime.is_composing());

        // Japanese with conversion clauses: "日本語" + "入力"
        ime.handle_event(ImeEvent::Start);
        ime.handle_event(ImeEvent::Preedit {
            text: "日本語入力".into(),
            cursor_bytes: 9,
            clauses: vec![
                CompositionClause {
                    start: 0,
                    end: 9,
                    style: ClauseStyle::TargetConverted,
                },
                CompositionClause {
                    start: 9,
                    end: 15,
                    style: ClauseStyle::Input,
                },
            ],
        });
        let comp = ime.composition().expect("jp comp");
        assert_eq!(comp.clauses.len(), 2);
        assert_eq!(comp.clauses[0].style, ClauseStyle::TargetConverted);
        let commit_ja = ime.handle_event(ImeEvent::Commit("日本語入力".into()));
        assert_eq!(
            commit_ja,
            ImeAction::CommitText {
                target_id: "block-1".into(),
                text: "日本語入力".into()
            }
        );

        // Korean Hangul composition cancelled cleanly on End.
        ime.handle_event(ImeEvent::Start);
        ime.handle_event(ImeEvent::Preedit {
            text: "한".into(),
            cursor_bytes: 2, // mid-UTF8 byte index must clamp safely to char boundary!
            clauses: vec![],
        });
        assert_eq!(ime.composition().expect("ko").cursor_bytes, 0);
        ime.handle_event(ImeEvent::End);
        assert!(!ime.is_composing());
    }
}
