//! Markdown table-of-contents extraction with GitHub slug rules (plan N.14 step 2).
//!
//! Ported from SumatraPDF Enhanced's `src/MarkdownToc.cpp` (`MarkdownHeadingSlug`):
//! - Trim leading and trailing ASCII whitespace.
//! - Lowercase ASCII `A-Z` to `a-z`.
//! - Keep ASCII letters, digits, `_`, `-`, and all non-ASCII UTF-8 characters.
//! - Turn each whitespace character into its own `-` without collapsing runs (so
//!   `"adc_intr_ctl . TRANS_EN"` becomes `"adc_intr_ctl--trans_en"`).
//! - Drop all other ASCII punctuation.
//! - Disambiguate duplicate slugs with `-1`, `-2`, ... as GitHub does.

use std::collections::HashMap;

/// A single heading entry extracted from a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocHeading {
    /// Heading depth `1..=6`.
    pub level: u8,
    /// Plain heading text with inline Markdown markers stripped.
    pub title: String,
    /// Unique GitHub-compatible anchor slug (without leading `#`).
    pub slug: String,
    /// One-based source line number in the Markdown document.
    pub line: usize,
}

/// Computes the GitHub anchor slug for a heading title (matching `github-slugger` and
/// SumatraPDF Enhanced's `MarkdownHeadingSlug`).
#[must_use]
pub fn github_slug(title: &str) -> String {
    let trimmed = title.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r'));
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if !ch.is_ascii() {
            // Keep non-ASCII Unicode letters/characters intact (lowercase where simple).
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            continue;
        }
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '_' || lower == '-' {
            out.push(lower);
        } else if matches!(lower, ' ' | '\t' | '\n' | '\r') {
            out.push('-');
        }
        // All other ASCII punctuation is dropped.
    }
    out
}

/// Stateful slug disambiguator producing unique GitHub anchors across a document.
#[derive(Debug, Default)]
pub(crate) struct Slugger {
    seen: HashMap<String, usize>,
}

impl Slugger {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn slug(&mut self, title: &str) -> String {
        let base = github_slug(title);
        let count = self.seen.entry(base.clone()).or_insert(0);
        let result = if *count == 0 {
            base
        } else {
            format!("{base}-{}", *count)
        };
        *count += 1;
        result
    }
}

/// Extracts the ordered table of contents (ATX and Setext headings) from `markdown`, ignoring
/// headings inside fenced code blocks (` ``` ` or `~~~`).
#[must_use]
pub fn extract_toc(markdown: &str) -> Vec<TocHeading> {
    let mut out = Vec::new();
    let mut slugger = Slugger::new();
    let lines: Vec<&str> = markdown.lines().collect();

    let mut fence: Option<(char, usize)> = None;
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed_start = line.trim_start();

        if let Some((fch, flen)) = fence {
            if is_closing_fence(trimmed_start, fch, flen) {
                fence = None;
            }
            i += 1;
            continue;
        }
        if let Some(opened) = parse_opening_fence(trimmed_start) {
            fence = Some(opened);
            i += 1;
            continue;
        }

        // ATX heading: up to 3 leading spaces, 1..=6 `#`, followed by space/tab/EOL.
        if let Some((level, raw_title)) = parse_atx_heading(line) {
            let title = strip_inline_markdown(raw_title);
            if !title.is_empty() {
                let slug = slugger.slug(&title);
                out.push(TocHeading {
                    level,
                    title,
                    slug,
                    line: i + 1,
                });
            }
            i += 1;
            continue;
        }

        // Setext heading: non-empty paragraph line followed by `===` (h1) or `---` (h2).
        if i + 1 < lines.len()
            && !trimmed_start.is_empty()
            && let Some(level) = parse_setext_underline(lines[i + 1])
        {
            let title = strip_inline_markdown(line.trim());
            if !title.is_empty() {
                let slug = slugger.slug(&title);
                out.push(TocHeading {
                    level,
                    title,
                    slug,
                    line: i + 1,
                });
            }
            i += 2;
            continue;
        }

        i += 1;
    }
    out
}

pub(crate) fn parse_opening_fence(trimmed_start: &str) -> Option<(char, usize)> {
    let first = trimmed_start.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed_start.chars().take_while(|&c| c == first).count();
    if count >= 3 {
        // Backtick info strings cannot contain backticks per CommonMark.
        if first == '`' && trimmed_start[count..].contains('`') {
            return None;
        }
        Some((first, count))
    } else {
        None
    }
}

pub(crate) fn is_closing_fence(trimmed_start: &str, ch: char, min_len: usize) -> bool {
    let count = trimmed_start.chars().take_while(|&c| c == ch).count();
    count >= min_len && trimmed_start[count..].trim().is_empty()
}

pub(crate) fn parse_atx_heading(line: &str) -> Option<(u8, &str)> {
    let leading_spaces = line.bytes().take_while(|&b| b == b' ').count();
    if leading_spaces > 3 {
        return None;
    }
    let rest = &line[leading_spaces..];
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after = &rest[hashes..];
    if !after.is_empty() && !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let mut title = after.trim();
    // Strip optional closing `###` sequence preceded by whitespace.
    let without_trailing_hashes = title.trim_end_matches('#');
    if without_trailing_hashes.len() < title.len()
        && (without_trailing_hashes.is_empty()
            || without_trailing_hashes.ends_with(' ')
            || without_trailing_hashes.ends_with('\t'))
    {
        title = without_trailing_hashes.trim_end();
    }
    Some((hashes as u8, title))
}

pub(crate) fn parse_setext_underline(line: &str) -> Option<u8> {
    let leading_spaces = line.bytes().take_while(|&b| b == b' ').count();
    if leading_spaces > 3 {
        return None;
    }
    let trimmed = line[leading_spaces..].trim_end();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.bytes().all(|b| b == b'=') {
        Some(1)
    } else if trimmed.bytes().all(|b| b == b'-') && trimmed.len() >= 2 {
        Some(2)
    } else {
        None
    }
}

/// Strips inline Markdown syntax (`**`, `*`, `` ` ``, `~~`, `[text](url)`) to recover heading plain text.
pub(crate) fn strip_inline_markdown(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '`' => {
                let tick_start = i;
                while i < chars.len() && chars[i] == '`' {
                    i += 1;
                }
                let tick_len = i - tick_start;
                let code_start = i;
                while i < chars.len() {
                    if chars[i] == '`'
                        && chars[i..].iter().take_while(|&&c| c == '`').count() == tick_len
                    {
                        for &c in &chars[code_start..i] {
                            out.push(c);
                        }
                        i += tick_len;
                        break;
                    }
                    i += 1;
                }
            }
            '[' => {
                // Parse `[label](url)` -> `label`.
                let mut j = i + 1;
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j + 1 < chars.len() && chars[j] == ']' && chars[j + 1] == '(' {
                    let mut k = j + 2;
                    while k < chars.len() && chars[k] != ')' {
                        k += 1;
                    }
                    if k < chars.len() && chars[k] == ')' {
                        let label: String = chars[i + 1..j].iter().collect();
                        out.push_str(&strip_inline_markdown(&label));
                        i = k + 1;
                        continue;
                    }
                }
                out.push('[');
                i += 1;
            }
            '*' | '~' => {
                i += 1;
            }
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_slug_matches_sumatrapdf_enhanced_rules() {
        // Exact example from SumatraPDF Enhanced MarkdownToc.cpp (#5883).
        assert_eq!(
            github_slug("adc_intr_ctl . TRANS_EN"),
            "adc_intr_ctl--trans_en"
        );
        assert_eq!(github_slug("  Hello, World!  "), "hello-world");
        assert_eq!(github_slug("C++ & Rust 2024"), "c--rust-2024");
        assert_eq!(github_slug("Café Résumé"), "café-résumé");
    }

    #[test]
    fn extract_toc_handles_atx_setext_duplicates_and_fenced_code() {
        let md = "\
# Overview

```markdown
# Not a heading inside code
```

## Setup & **Install**
### Setup & Install
Setext Section
--------------
";
        let toc = extract_toc(md);
        assert_eq!(toc.len(), 4);
        assert_eq!(
            (toc[0].level, toc[0].slug.as_str(), toc[0].line),
            (1, "overview", 1)
        );
        assert_eq!(
            (toc[1].level, toc[1].title.as_str(), toc[1].slug.as_str()),
            (2, "Setup & Install", "setup--install")
        );
        assert_eq!(
            (toc[2].level, toc[2].slug.as_str()),
            (3, "setup--install-1")
        );
        assert_eq!(
            (toc[3].level, toc[3].title.as_str(), toc[3].slug.as_str()),
            (2, "Setext Section", "setext-section")
        );
    }
}
