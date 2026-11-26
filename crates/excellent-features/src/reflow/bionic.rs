//! Bionic reading for reflow mode (plan R.22): bolds the leading portion (~40 %) of each
//! word in scripts where fixation-point bolding aids reading (Latin, Cyrillic, and Greek) and
//! automatically disables itself for scripts where word-prefix bolding does not apply (such as
//! CJK, Arabic, Hebrew, Thai, and Indic scripts).

use std::fmt::Write as _;

/// Default fixation ratio (40 % of each word).
pub const DEFAULT_STRENGTH: f32 = 0.4;

/// Whether bionic reading is enabled and at what bold-prefix ratio.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BionicMode {
    /// Bionic bolding disabled.
    #[default]
    Off,
    /// Bionic bolding enabled with `strength` in `0.1..=0.8` (default `0.4`).
    On {
        /// Fraction of leading characters to bold per word.
        strength: f32,
    },
}

impl BionicMode {
    /// Clamped strength when enabled, or `None` when [`Self::Off`].
    #[must_use]
    pub fn strength(self) -> Option<f32> {
        match self {
            Self::Off => None,
            Self::On { strength } => {
                let s = if strength.is_finite() {
                    strength.clamp(0.1, 0.8)
                } else {
                    DEFAULT_STRENGTH
                };
                Some(s)
            }
        }
    }
}

/// A formatted piece of text produced by bionic reading transformation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BionicSegment {
    /// Leading fixation prefix rendered in bold (`<strong>`).
    Bold(String),
    /// Remaining unbolded characters or whitespace/punctuation.
    Normal(String),
}

/// Returns `true` when `ch` belongs to a bicameral / alphabetic script supported by bionic
/// reading (Latin, Cyrillic, or Greek).
#[must_use]
pub fn is_bionic_script_char(ch: char) -> bool {
    let cp = ch as u32;
    // Basic Latin letters + Latin-1 Supplement + Latin Extended-A/B/Additional
    let is_latin = ch.is_ascii_alphabetic()
        || (0x00C0..=0x024F).contains(&cp) && ch.is_alphabetic()
        || (0x1E00..=0x1EFF).contains(&cp) && ch.is_alphabetic();
    // Greek and Coptic + Greek Extended
    let is_greek = (0x0370..=0x03FF).contains(&cp) && ch.is_alphabetic()
        || (0x1F00..=0x1FFF).contains(&cp) && ch.is_alphabetic();
    // Cyrillic + Cyrillic Supplement
    let is_cyrillic = (0x0400..=0x052F).contains(&cp) && ch.is_alphabetic();

    is_latin || is_greek || is_cyrillic
}

/// Returns `true` when every alphabetic character in `word` belongs to a supported script
/// (Latin, Cyrillic, or Greek) and there is at least one such letter.
#[must_use]
pub fn supports_bionic_word(word: &str) -> bool {
    let mut has_letter = false;
    for ch in word.chars() {
        if ch.is_alphabetic() {
            if !is_bionic_script_char(ch) {
                return false;
            }
            has_letter = true;
        }
    }
    has_letter
}

/// Number of leading Unicode scalar characters to bold for a word of `char_len` letters at
/// `strength`.
#[must_use]
pub fn bold_prefix_chars(char_len: usize, strength: f32) -> usize {
    if char_len == 0 {
        return 0;
    }
    let s = if strength.is_finite() {
        strength.clamp(0.1, 0.8)
    } else {
        DEFAULT_STRENGTH
    };
    if char_len == 1 {
        return 1;
    }
    if char_len <= 3 {
        return 1;
    }
    let raw = (char_len as f32 * s).ceil() as usize;
    raw.clamp(1, char_len - 1)
}

/// Splits `text` into [`BionicSegment`]s (`Bold` / `Normal`) using `strength` (~`0.4` by default).
///
/// Unsupported scripts (CJK, Arabic, Hebrew, Thai, Devanagari, etc.) pass through as
/// [`BionicSegment::Normal`] untouched.
#[must_use]
pub fn transform_text(text: &str, strength: f32) -> Vec<BionicSegment> {
    let mut segments: Vec<BionicSegment> = Vec::new();
    let push_seg = |segments: &mut Vec<BionicSegment>, bold: bool, slice: &str| {
        if slice.is_empty() {
            return;
        }
        if bold {
            if let Some(BionicSegment::Bold(prev)) = segments.last_mut() {
                prev.push_str(slice);
            } else {
                segments.push(BionicSegment::Bold(slice.to_owned()));
            }
        } else if let Some(BionicSegment::Normal(prev)) = segments.last_mut() {
            prev.push_str(slice);
        } else {
            segments.push(BionicSegment::Normal(slice.to_owned()));
        }
    };

    let mut idx = 0usize;
    let bytes = text.as_bytes();
    while idx < bytes.len() {
        let rest = &text[idx..];
        let Some(first_ch) = rest.chars().next() else {
            break;
        };
        if first_ch.is_alphabetic() {
            let word_bytes = rest
                .char_indices()
                .find(|&(_, ch)| !ch.is_alphabetic() && ch != '\'' && ch != '’')
                .map_or(rest.len(), |(pos, _)| pos);
            let word = &rest[..word_bytes];
            if supports_bionic_word(word) {
                let letter_count = word.chars().filter(|c| c.is_alphabetic()).count();
                let bold_letters = bold_prefix_chars(letter_count, strength);
                let mut seen = 0usize;
                let mut split_byte = word.len();
                for (bpos, ch) in word.char_indices() {
                    if seen == bold_letters {
                        split_byte = bpos;
                        break;
                    }
                    if ch.is_alphabetic() {
                        seen += 1;
                    }
                }
                push_seg(&mut segments, true, &word[..split_byte]);
                push_seg(&mut segments, false, &word[split_byte..]);
            } else {
                push_seg(&mut segments, false, word);
            }
            idx += word_bytes;
        } else {
            let non_alpha_bytes = rest
                .char_indices()
                .find(|&(_, ch)| ch.is_alphabetic())
                .map_or(rest.len(), |(pos, _)| pos);
            push_seg(&mut segments, false, &rest[..non_alpha_bytes]);
            idx += non_alpha_bytes;
        }
    }

    segments
}

/// Formats `text` as HTML with `<strong>` tags on bionic fixation prefixes (HTML-escaping all
/// characters).
#[must_use]
pub fn to_html(text: &str, strength: f32) -> String {
    let segments = transform_text(text, strength);
    let mut out = String::with_capacity(text.len() + 16);
    for seg in segments {
        match seg {
            BionicSegment::Bold(s) => {
                out.push_str("<strong>");
                push_escaped_html(&mut out, &s);
                out.push_str("</strong>");
            }
            BionicSegment::Normal(s) => push_escaped_html(&mut out, &s),
        }
    }
    out
}

pub(super) fn push_escaped_html(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => {
                let _ = write!(out, "{ch}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bolds_latin_cyrillic_and_greek_prefixes() {
        // Latin
        assert_eq!(to_html("Reading", 0.4), "<strong>Rea</strong>ding");
        // Cyrillic ("Привет")
        assert_eq!(to_html("Привет", 0.4), "<strong>При</strong>вет");
        // Greek ("Αλφαβητο")
        assert_eq!(to_html("Αλφαβητο", 0.4), "<strong>Αλφα</strong>βητο");
    }

    #[test]
    fn turns_itself_off_for_cjk_arabic_and_thai() {
        for sample in ["日本語のテキスト", "مرحبا بالعالم", "ภาษาไทย", "שלום"]
        {
            assert_eq!(
                to_html(sample, 0.4),
                sample,
                "bionic must turn off for {sample}"
            );
        }
    }
}
