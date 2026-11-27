//! Read aloud (plan U.5): maps K.13 `PageText` into cleaned spoken utterances with exact character-level reverse
//! maps, translates TTS `SpeechEvent::WordBoundary` offsets back into K.13 highlight quads (word and sentence), and
//! drives continuous multi-page playback over any [`SpeechSynth`] backend.

use std::ops::Range;

use ecclesia_mupdf::{PagePoint, Quad};
use ecclesia_platform::speech::{SpeechError, SpeechEvent, SpeechSynth, VoiceInfo, clamp_speed};
use ecclesia_text::{Granularity, PageText};

/// Cleaned spoken utterance built from one K.13 [`PageText`], paired with a per-character reverse map back to
/// character indices on that page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpokenPage {
    /// Zero-based page index.
    pub page: usize,
    /// Cleaned UTF-8 text fed to [`SpeechSynth::speak`].
    pub spoken_text: String,
    /// For each Unicode scalar index `k` in `spoken_text`, the corresponding character index in `PageText`.
    /// Synthetic separators (e.g. spaces inserted for line breaks) store `None`.
    char_to_page_index: Vec<Option<usize>>,
}

impl SpokenPage {
    /// Builds a [`SpokenPage`] for `page_text` covering character range `range` in reading order.
    ///
    /// Line breaks become single spaces, paragraph breaks become double spaces (which [`sentence_range`] also treats
    /// as sentence boundaries), and hyphenated line endings (`line.joined`) are joined seamlessly just as
    /// `PageText::text` does.
    #[must_use]
    pub fn from_page_range(page: usize, page_text: &PageText, range: Range<usize>) -> Self {
        let layout = page_text.layout();
        let mut spoken_text = String::new();
        let mut char_to_page_index = Vec::new();
        let mut previous: Option<(usize, bool)> = None;

        for line in &layout.lines {
            let start = line.chars.start.max(range.start);
            let mut end = line.chars.end.min(range.end);
            if start >= end {
                continue;
            }
            match previous {
                Some((_, true)) | None => {}
                Some((prev_block, false)) => {
                    if prev_block == line.block {
                        if !spoken_text.ends_with(' ') {
                            spoken_text.push(' ');
                            char_to_page_index.push(None);
                        }
                    } else {
                        while spoken_text.ends_with(' ') {
                            spoken_text.pop();
                            char_to_page_index.pop();
                        }
                        if !spoken_text.is_empty() {
                            spoken_text.push_str("  ");
                            char_to_page_index.push(None);
                            char_to_page_index.push(None);
                        }
                    }
                }
            }

            let joined = line.joined && end == line.chars.end;
            if joined && is_hyphen(layout.chars[end - 1].c) {
                end -= 1;
            }
            for idx in start..end {
                let ch = layout.chars[idx].c;
                if ch.is_control() && ch != '\t' {
                    continue;
                }
                let emit = if ch == '\t' { ' ' } else { ch };
                spoken_text.push(emit);
                char_to_page_index.push(Some(idx));
            }
            previous = Some((line.block, joined));
        }

        Self {
            page,
            spoken_text,
            char_to_page_index,
        }
    }

    /// Whether there is no speakable text in this slice.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spoken_text.trim().is_empty()
    }

    /// Maps a character range in `spoken_text` (e.g. from a TTS `WordBoundary` event) back to a contiguous character
    /// range in K.13 [`PageText`], returning `None` if no glyphs fall inside `spoken_chars`.
    #[must_use]
    pub fn page_char_range(&self, spoken_chars: Range<usize>) -> Option<Range<usize>> {
        let end = spoken_chars.end.min(self.char_to_page_index.len());
        let start = spoken_chars.start.min(end);
        let mut min_idx: Option<usize> = None;
        let mut max_idx: Option<usize> = None;
        for &mapped in &self.char_to_page_index[start..end] {
            if let Some(page_idx) = mapped {
                min_idx = Some(min_idx.map_or(page_idx, |m| m.min(page_idx)));
                max_idx = Some(max_idx.map_or(page_idx, |m| m.max(page_idx)));
            }
        }
        Some(min_idx?..max_idx? + 1)
    }

    /// Resolves K.13 highlight quads for a spoken word event (`char_offset`, `char_len`).
    ///
    /// When `char_len == 0` (some TTS engines report start offsets only), expands to the next whitespace-delimited
    /// word in `spoken_text`.
    #[must_use]
    pub fn word_quads(
        &self,
        page_text: &PageText,
        char_offset: usize,
        char_len: usize,
    ) -> (Vec<Quad>, String) {
        let chars: Vec<char> = self.spoken_text.chars().collect();
        if char_offset >= chars.len() {
            return (Vec::new(), String::new());
        }
        let end = if char_len > 0 {
            (char_offset + char_len).min(chars.len())
        } else {
            let mut s = char_offset;
            while s < chars.len() && chars[s].is_whitespace() {
                s += 1;
            }
            let mut e = s;
            while e < chars.len() && !chars[e].is_whitespace() {
                e += 1;
            }
            e
        };
        let word_text: String = chars[char_offset..end].iter().collect();
        let quads = self
            .page_char_range(char_offset..end)
            .map(|r| page_text.highlight(r))
            .unwrap_or_default();
        (quads, word_text)
    }

    /// Resolves K.13 highlight quads for the sentence containing spoken character offset `char_offset`.
    #[must_use]
    pub fn sentence_quads(&self, page_text: &PageText, char_offset: usize) -> Vec<Quad> {
        let byte_pos = char_to_byte_offset(&self.spoken_text, char_offset);
        let Some(byte_range) = sentence_range(&self.spoken_text, byte_pos) else {
            return Vec::new();
        };
        let start_char = self.spoken_text[..byte_range.start].chars().count();
        let end_char = start_char + self.spoken_text[byte_range].chars().count();
        self.page_char_range(start_char..end_char)
            .map(|r| page_text.highlight(r))
            .unwrap_or_default()
    }

    /// Returns the page character index corresponding to `char_offset` (or the nearest following mapped character),
    /// used to resume reading after stop/pause.
    #[must_use]
    pub fn page_char_at(&self, char_offset: usize) -> Option<usize> {
        let start = char_offset.min(self.char_to_page_index.len());
        self.char_to_page_index[start..]
            .iter()
            .copied()
            .flatten()
            .next()
    }
}

/// Finds the sentence byte range containing `byte_pos` inside `text` (ported from SumatraPDF Enhanced's
/// `ReadAloudSentenceRange`), handling abbreviations like `"e.g. the"`, CJK punctuation (`。！？`), ellipses (`…`)
/// and double-space paragraph breaks.
#[must_use]
pub fn sentence_range(text: &str, byte_pos: usize) -> Option<Range<usize>> {
    if text.is_empty() {
        return None;
    }
    let pos = clamp_to_char_boundary(text, byte_pos.min(text.len()));

    let mut start = 0_usize;
    let mut iter = text[..pos].char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        let next = i + ch.len_utf8();
        if is_sentence_punct(ch) {
            if let Some(after) = after_sentence_end(text, next)
                && after <= pos
            {
                start = after;
            }
        } else if ch == ' ' && text.as_bytes().get(next) == Some(&b' ') {
            let mut after = next;
            while text.as_bytes().get(after) == Some(&b' ') {
                after += 1;
            }
            if after <= pos {
                start = after;
            }
            while iter.peek().is_some_and(|&(idx, _)| idx < after) {
                iter.next();
            }
        }
    }

    let mut end = text.len();
    for (i, ch) in text[pos..].char_indices() {
        let abs = pos + i;
        let next = abs + ch.len_utf8();
        if is_sentence_punct(ch) {
            if let Some(after) = after_sentence_end(text, next) {
                end = after;
                break;
            }
        } else if ch == ' ' && text.as_bytes().get(next) == Some(&b' ') {
            end = next;
            break;
        }
    }

    (end > start).then_some(start..end)
}

/// Resolves the starting K.13 character index on `page_text` for "Read from cursor" at page point `point` (snaps to
/// the start of the word under the pointer).
#[must_use]
pub fn cursor_start_char(page_text: &PageText, point: PagePoint) -> Option<usize> {
    if !page_text.is_over_text(point) {
        return None;
    }
    let ch = page_text.nearest_char(point)?;
    Some(page_text.unit(ch, Granularity::Word).start)
}

/// Active word and sentence highlight produced by [`ReadAloudSession`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReadAloudHighlight {
    /// Zero-based page index where the spoken word lies.
    pub page: usize,
    /// K.13 highlight quads covering the currently spoken word.
    pub word_quads: Vec<Quad>,
    /// K.13 highlight quads covering the currently spoken sentence.
    pub sentence_quads: Vec<Quad>,
    /// Text of the currently spoken word.
    pub word_text: String,
    /// Exact K.13 character index on `page` for resume tracking.
    pub page_char_index: usize,
}

/// Saved resume cursor so stopping and continuing picks up on the exact word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumePoint {
    /// Zero-based page index.
    pub page: usize,
    /// Character index inside `PageText`.
    pub char_index: usize,
}

/// Lifecycle state of a read-aloud playback session (plan U.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    /// Idle / stopped.
    Stopped,
    /// Actively speaking.
    Playing,
    /// Paused mid-utterance.
    Paused,
    /// Reached the end of the selection or document.
    Finished,
}

/// Multi-page read-aloud controller parameterized over any [`SpeechSynth`] backend (plan U.5).
#[derive(Debug)]
pub struct ReadAloudSession<S: SpeechSynth> {
    synth: S,
    pages: Vec<PageText>,
    state: PlaybackState,
    current_spoken: Option<SpokenPage>,
    current_page: usize,
    end_page: usize,
    end_char_on_last_page: Option<usize>,
    active_highlight: Option<ReadAloudHighlight>,
    resume_point: Option<ResumePoint>,
    speed: f32,
    voice_id: Option<String>,
    last_error: Option<String>,
}

impl<S: SpeechSynth> ReadAloudSession<S> {
    /// Creates a stopped read-aloud session over `pages` (one K.13 [`PageText`] per page in reading order).
    #[must_use]
    pub fn new(synth: S, pages: Vec<PageText>) -> Self {
        Self {
            synth,
            pages,
            state: PlaybackState::Stopped,
            current_spoken: None,
            current_page: 0,
            end_page: 0,
            end_char_on_last_page: None,
            active_highlight: None,
            resume_point: None,
            speed: 1.0,
            voice_id: None,
            last_error: None,
        }
    }

    /// Borrows the underlying synthesizer (useful in tests with `MockSpeechSynth`).
    #[must_use]
    pub fn synth(&self) -> &S {
        &self.synth
    }

    /// Mutably borrows the underlying synthesizer.
    pub fn synth_mut(&mut self) -> &mut S {
        &mut self.synth
    }

    /// Lists voices reported by the backend.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] from the backend.
    pub fn voices(&self) -> Result<Vec<VoiceInfo>, SpeechError> {
        self.synth.voices()
    }

    /// Sets playback speed (`0.5`–`3.0`) on the backend.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] from the backend.
    pub fn set_speed(&mut self, speed: f32) -> Result<(), SpeechError> {
        let clamped = clamp_speed(speed);
        self.synth.set_rate(clamped)?;
        self.speed = clamped;
        Ok(())
    }

    /// Current playback speed multiplier.
    #[must_use]
    pub const fn speed(&self) -> f32 {
        self.speed
    }

    /// Selects a voice by id (or system default when `None`).
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] from the backend.
    pub fn set_voice(&mut self, voice_id: Option<&str>) -> Result<(), SpeechError> {
        self.synth.set_voice(voice_id)?;
        self.voice_id = voice_id.map(str::to_owned);
        Ok(())
    }

    /// Currently selected voice id.
    #[must_use]
    pub fn voice_id(&self) -> Option<&str> {
        self.voice_id.as_deref()
    }

    /// Current playback state.
    #[must_use]
    pub const fn state(&self) -> PlaybackState {
        self.state
    }

    /// Current word + sentence highlight in K.13 quads, if playing or paused on a word.
    #[must_use]
    pub fn active_highlight(&self) -> Option<&ReadAloudHighlight> {
        self.active_highlight.as_ref()
    }

    /// Saved resume point from the most recently spoken word.
    #[must_use]
    pub const fn resume_point(&self) -> Option<ResumePoint> {
        self.resume_point
    }

    /// Last backend error message, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Begins reading continuously from `(start_page, start_char)` through the end of the document.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] if the synthesizer fails to start.
    pub fn start_from(
        &mut self,
        start_page: usize,
        start_char: usize,
    ) -> Result<bool, SpeechError> {
        if self.pages.is_empty() || start_page >= self.pages.len() {
            self.state = PlaybackState::Finished;
            return Ok(false);
        }
        self.end_page = self.pages.len() - 1;
        self.end_char_on_last_page = None;
        self.last_error = None;
        self.speak_from_page(start_page, start_char)
    }

    /// Begins reading from the word under `cursor` on `page`.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] if the synthesizer fails to start.
    pub fn start_from_cursor(
        &mut self,
        page: usize,
        cursor: PagePoint,
    ) -> Result<bool, SpeechError> {
        let Some(page_text) = self.pages.get(page) else {
            return Ok(false);
        };
        let Some(start_char) = cursor_start_char(page_text, cursor) else {
            return Ok(false);
        };
        self.start_from(page, start_char)
    }

    /// Speaks only the selection `range` on `page` (does not advance to subsequent pages).
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] if the synthesizer fails to start.
    pub fn start_selection(
        &mut self,
        page: usize,
        range: Range<usize>,
    ) -> Result<bool, SpeechError> {
        if page >= self.pages.len() || range.is_empty() {
            return Ok(false);
        }
        self.end_page = page;
        self.end_char_on_last_page = Some(range.end);
        self.last_error = None;
        self.speak_from_page(page, range.start)
    }

    /// Continues reading from [`Self::resume_point`] if one was recorded.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] if the synthesizer fails to start.
    pub fn continue_reading(&mut self) -> Result<bool, SpeechError> {
        let Some(point) = self.resume_point else {
            return Ok(false);
        };
        self.start_from(point.page, point.char_index)
    }

    /// Pauses active playback.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] from the backend.
    pub fn pause(&mut self) -> Result<(), SpeechError> {
        if self.state == PlaybackState::Playing {
            self.synth.pause()?;
            self.state = PlaybackState::Paused;
        }
        Ok(())
    }

    /// Resumes paused playback.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] from the backend.
    pub fn resume(&mut self) -> Result<(), SpeechError> {
        if self.state == PlaybackState::Paused {
            self.synth.resume()?;
            self.state = PlaybackState::Playing;
        }
        Ok(())
    }

    /// Stops playback while retaining [`Self::resume_point`] so the user can continue later.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] from the backend.
    pub fn stop(&mut self) -> Result<(), SpeechError> {
        self.synth.stop()?;
        self.state = PlaybackState::Stopped;
        self.current_spoken = None;
        self.active_highlight = None;
        Ok(())
    }

    /// Drains backend [`SpeechEvent`]s, updates word/sentence K.13 highlight quads, and automatically advances to the
    /// next non-empty page when the current page finishes (plan U.5).
    ///
    /// Returns every [`ReadAloudHighlight`] observed during this poll step.
    ///
    /// # Errors
    ///
    /// Propagates [`SpeechError`] if starting the next page fails.
    pub fn poll(&mut self) -> Result<Vec<ReadAloudHighlight>, SpeechError> {
        if self.state != PlaybackState::Playing {
            return Ok(Vec::new());
        }
        let events = self.synth.poll_events();
        let mut observed = Vec::new();
        let mut page_finished = false;

        for event in events {
            match event {
                SpeechEvent::WordBoundary {
                    char_offset,
                    char_len,
                } => {
                    if let Some(spoken) = self.current_spoken.as_ref()
                        && let Some(page_text) = self.pages.get(spoken.page)
                    {
                        let (word_quads, word_text) =
                            spoken.word_quads(page_text, char_offset, char_len);
                        let sentence_quads = spoken.sentence_quads(page_text, char_offset);
                        let page_char_index = spoken.page_char_at(char_offset).unwrap_or(0);
                        self.resume_point = Some(ResumePoint {
                            page: spoken.page,
                            char_index: page_char_index,
                        });
                        let hl = ReadAloudHighlight {
                            page: spoken.page,
                            word_quads,
                            sentence_quads,
                            word_text,
                            page_char_index,
                        };
                        self.active_highlight = Some(hl.clone());
                        observed.push(hl);
                    }
                }
                SpeechEvent::Finished => {
                    page_finished = true;
                }
                SpeechEvent::Error(msg) => {
                    self.last_error = Some(msg);
                    self.state = PlaybackState::Stopped;
                    self.current_spoken = None;
                    self.active_highlight = None;
                    return Ok(observed);
                }
            }
        }

        if page_finished {
            if self.current_page < self.end_page {
                let next_page = self.current_page + 1;
                self.speak_from_page(next_page, 0)?;
            } else {
                self.state = PlaybackState::Finished;
                self.current_spoken = None;
                self.active_highlight = None;
            }
        }
        Ok(observed)
    }

    fn speak_from_page(
        &mut self,
        start_page: usize,
        start_char: usize,
    ) -> Result<bool, SpeechError> {
        let last = self.end_page.min(self.pages.len().saturating_sub(1));
        for page_idx in start_page..=last {
            let page_text = &self.pages[page_idx];
            let from = if page_idx == start_page {
                start_char.min(page_text.len())
            } else {
                0
            };
            let to = if page_idx == self.end_page {
                self.end_char_on_last_page
                    .unwrap_or(page_text.len())
                    .min(page_text.len())
            } else {
                page_text.len()
            };
            if from >= to {
                continue;
            }
            let spoken = SpokenPage::from_page_range(page_idx, page_text, from..to);
            if spoken.is_empty() {
                continue;
            }
            self.synth.speak(&spoken.spoken_text)?;
            self.current_page = page_idx;
            self.resume_point = Some(ResumePoint {
                page: page_idx,
                char_index: from,
            });
            self.current_spoken = Some(spoken);
            self.state = PlaybackState::Playing;
            return Ok(true);
        }
        self.state = PlaybackState::Finished;
        self.current_spoken = None;
        self.active_highlight = None;
        Ok(false)
    }
}

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{ad}' | '\u{2010}' | '\u{2011}')
}

fn is_sentence_punct(c: char) -> bool {
    matches!(
        c,
        '.' | '!' | '?' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}' | '\u{2026}'
    )
}

fn is_sentence_closer(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | ')' | ']' | '}' | '\u{2019}' | '\u{201d}' | '\u{00bb}'
    )
}

fn after_sentence_end(text: &str, after_punct: usize) -> Option<usize> {
    let mut after = after_punct;
    while let Some(ch) = text[after..].chars().next() {
        if !is_sentence_closer(ch) {
            break;
        }
        after += ch.len_utf8();
    }
    while let Some(ch) = text[after..].chars().next() {
        if ch != ' ' && ch != '\t' {
            break;
        }
        after += ch.len_utf8();
    }
    if let Some(next_ch) = text[after..].chars().next()
        && next_ch.is_ascii_lowercase()
    {
        return None;
    }
    Some(after)
}

fn char_to_byte_offset(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map_or(s.len(), |(byte_idx, _)| byte_idx)
}

fn clamp_to_char_boundary(s: &str, mut byte_pos: usize) -> usize {
    while byte_pos > 0 && !s.is_char_boundary(byte_pos) {
        byte_pos -= 1;
    }
    byte_pos
}

#[cfg(test)]
mod tests {
    use ecclesia_mupdf::{PageRect, TextBlock, TextChar, TextLayout, TextLine};
    use ecclesia_platform::speech::MockSpeechSynth;

    use super::*;

    fn synthetic_page(blocks: &[&[&str]]) -> PageText {
        let mut layout = TextLayout::default();
        let mut y = 0.0;
        for (b, lines) in blocks.iter().enumerate() {
            let first_line = layout.lines.len();
            for text in *lines {
                let first_char = layout.chars.len();
                for (x, c) in text.chars().enumerate() {
                    let x0 = x as f32 * 10.0;
                    let pt = |x, y| PagePoint { x, y };
                    layout.chars.push(TextChar {
                        c,
                        quad: Quad {
                            ul: pt(x0, y),
                            ur: pt(x0 + 10.0, y),
                            ll: pt(x0, y + 12.0),
                            lr: pt(x0 + 10.0, y + 12.0),
                        },
                        size: 12.0,
                    });
                }
                layout.lines.push(TextLine {
                    bbox: PageRect {
                        x0: 0.0,
                        y0: y,
                        x1: text.chars().count() as f32 * 10.0,
                        y1: y + 12.0,
                    },
                    dir: PagePoint { x: 1.0, y: 0.0 },
                    vertical: false,
                    joined: text.ends_with('-'),
                    block: b,
                    chars: first_char..layout.chars.len(),
                });
                y += 20.0;
            }
            layout.blocks.push(TextBlock {
                bbox: PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 200.0,
                    y1: y,
                },
                lines: first_line..layout.lines.len(),
            });
            y += 20.0;
        }
        PageText::new(layout)
    }

    /// Done when (U.5): highlighting stays accurately mapped onto every spoken word for a full multi-page chapter
    /// (including hyphenated wraps, empty/figure pages, sentence boundaries, pause/resume and voice/speed controls).
    #[test]
    fn highlighting_tracks_every_spoken_word_across_a_full_chapter() {
        let p0 = synthetic_page(&[
            &["Chapter One. Energy is con-", "served in closed systems."],
            &["Consider e.g. the ideal gas."],
        ]);
        let p1_empty = synthetic_page(&[]);
        let p2 = synthetic_page(&[&["Entropy never decreases!"], &["Final line."]]);

        let mut session = ReadAloudSession::new(MockSpeechSynth::new(), vec![p0, p1_empty, p2]);
        session.set_speed(1.5).expect("speed");
        session.set_voice(Some("mock-en-us")).expect("voice");
        assert!(session.start_from(0, 0).expect("start"));
        assert_eq!(session.state(), PlaybackState::Playing);

        let mut spoken_words: Vec<(usize, String)> = Vec::new();
        let mut safety_guard = 0;
        while session.state() == PlaybackState::Playing && safety_guard < 100 {
            safety_guard += 1;
            session.synth_mut().step();
            let highlights = session.poll().expect("poll");
            for hl in highlights {
                assert!(
                    !hl.word_quads.is_empty(),
                    "every spoken word must map to non-empty K.13 quads: {:?}",
                    hl.word_text
                );
                assert!(
                    !hl.sentence_quads.is_empty(),
                    "sentence highlight must cover the active word: {:?}",
                    hl.word_text
                );
                spoken_words.push((hl.page, hl.word_text));
            }
        }

        assert_eq!(session.state(), PlaybackState::Finished);
        // Page 0 words (note "con-" + "served" joined as "conserved" with 2-line quad!).
        assert_eq!(
            spoken_words,
            vec![
                (0, "Chapter".to_owned()),
                (0, "One.".to_owned()),
                (0, "Energy".to_owned()),
                (0, "is".to_owned()),
                (0, "conserved".to_owned()),
                (0, "in".to_owned()),
                (0, "closed".to_owned()),
                (0, "systems.".to_owned()),
                (0, "Consider".to_owned()),
                (0, "e.g.".to_owned()),
                (0, "the".to_owned()),
                (0, "ideal".to_owned()),
                (0, "gas.".to_owned()),
                // Page 1 was blank (figure page) and skipped cleanly to page 2!
                (2, "Entropy".to_owned()),
                (2, "never".to_owned()),
                (2, "decreases!".to_owned()),
                (2, "Final".to_owned()),
                (2, "line.".to_owned()),
            ]
        );
    }

    #[test]
    fn sentence_range_handles_abbreviations_and_cursor_stop_resume() {
        let text = "Use e.g. the ideal gas law. Next sentence!";
        let pos = text.find("ideal").expect("ideal");
        let r = sentence_range(text, pos).expect("sentence");
        assert_eq!(&text[r], "Use e.g. the ideal gas law. ");

        let page = synthetic_page(&[&["alpha beta gamma"]]);
        let mut session = ReadAloudSession::new(MockSpeechSynth::new(), vec![page]);
        // Click on "beta" (x=75 is inside char 7 'e').
        assert!(
            session
                .start_from_cursor(0, PagePoint { x: 75.0, y: 6.0 })
                .expect("start cursor")
        );
        session.synth_mut().step();
        let hl = session.poll().expect("poll");
        assert_eq!(hl[0].word_text, "beta");

        session.pause().expect("pause");
        assert_eq!(session.state(), PlaybackState::Paused);
        session.resume().expect("resume");
        assert_eq!(session.state(), PlaybackState::Playing);

        session.stop().expect("stop");
        assert_eq!(session.state(), PlaybackState::Stopped);
        assert_eq!(
            session.resume_point(),
            Some(ResumePoint {
                page: 0,
                char_index: 6
            })
        );
        assert!(session.continue_reading().expect("continue"));
        session.synth_mut().step();
        let resumed = session.poll().expect("poll resumed");
        assert_eq!(resumed[0].word_text, "beta");
    }
}
