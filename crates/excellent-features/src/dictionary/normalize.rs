//! Normalization, hashing, HTML/entity cleaning and English stemming helpers ported from SumatraPDF Enhanced's
//! `OfflineDictionary.cpp` and `KaikkiDictionary.cpp` (plan S.6).

use std::fmt::Write as _;

/// Maximum byte length of a normalized dictionary word key (matching Enhanced's `kDictionaryMaxWord`).
pub const MAX_WORD_BYTES: usize = 512;

/// Punctuation trimmed from both ends of a raw lookup term (matching Enhanced's `WordKey`).
const TRIM_PUNCT: &[char] = &[
    '"', '\'', '.', ',', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}',
];

/// Normalizes a lookup word: trims whitespace and outer punctuation, replaces `_` with spaces, and lowercases using
/// Unicode case mapping (matching Enhanced's `WordKey`). Returns `None` if empty or over [`MAX_WORD_BYTES`].
#[must_use]
pub fn word_key(text: &str) -> Option<String> {
    let trimmed = text.trim().trim_matches(TRIM_PUNCT);
    if trimmed.is_empty() || trimmed.len() > MAX_WORD_BYTES {
        return None;
    }
    let spaced = trimmed.replace('_', " ");
    let lowered = spaced.to_lowercase();
    if lowered.is_empty() || lowered.len() > MAX_WORD_BYTES {
        return None;
    }
    Some(lowered)
}

/// FNV-1a 64-bit hash over UTF-8 bytes (matching Enhanced's `KaikkiDictionary.cpp::WordHash`).
#[must_use]
pub fn word_hash(word: &str) -> u64 {
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for &byte in word.as_bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
    }
    hash
}

/// Strips HTML markup, resolves named and numeric HTML entities, and normalizes whitespace (matching Enhanced's
/// `PlainDefinition`).
#[must_use]
pub fn plain_definition(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = text[i..].chars().next().unwrap_or('\0');
        let ch_len = ch.len_utf8();
        if ch == '<' {
            if let Some(rel_end) = text[i + 1..].find('>') {
                let end = i + 1 + rel_end;
                let mut tag = text[i + 1..end].trim();
                if let Some(stripped) = tag.strip_prefix('/') {
                    tag = stripped.trim();
                }
                let tag_name = tag.split_whitespace().next().unwrap_or("");
                if matches!(
                    tag_name.to_ascii_lowercase().as_str(),
                    "p" | "div" | "br" | "br/" | "li" | "ol" | "ul"
                ) && !plain.is_empty()
                    && !plain.ends_with('\n')
                {
                    plain.push('\n');
                }
                i = end + 1;
                continue;
            }
            plain.push('<');
        } else if ch != '\0' && (ch as u32 >= 32 || ch == '\n' || ch == '\t') {
            plain.push(ch);
        }
        i += ch_len;
    }

    let decoded = decode_html_entities(&plain);
    let mut clean = String::with_capacity(decoded.len());
    let mut pending_space = false;
    for ch in decoded.chars() {
        if ch == '\r' {
            continue;
        }
        if ch == '\n' {
            if !clean.is_empty() && !clean.ends_with('\n') {
                clean.push('\n');
            }
            pending_space = false;
            continue;
        }
        if ch == ' ' || ch == '\t' {
            pending_space = !clean.is_empty() && !clean.ends_with('\n');
            continue;
        }
        if pending_space {
            clean.push(' ');
        }
        pending_space = false;
        clean.push(ch);
    }
    clean.trim().to_owned()
}

/// Cleans a raw dictionary definition and strips repeated parent glosses separated by `.:` (matching Enhanced's
/// `DictionaryPlainText`).
#[must_use]
pub fn dictionary_plain_text(definition: &str) -> String {
    let plain = plain_definition(definition);
    let mut out = String::with_capacity(plain.len());
    for raw_line in plain.split('\n') {
        let mut line = raw_line.trim();
        if let Some(idx) = line.find(".:")
            && idx + 2 < line.len()
        {
            line = line[idx + 2..].trim();
        }
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

/// Expands short part-of-speech codes (`n`, `v`, `a`, `s`, `r`, `adj`, `adv`, …) into full names (matching Enhanced's
/// `FullPos`).
#[must_use]
pub fn full_pos(pos: &str) -> String {
    match pos.trim().to_ascii_lowercase().as_str() {
        "n" => "noun".to_owned(),
        "v" => "verb".to_owned(),
        "a" | "s" | "adj" => "adjective".to_owned(),
        "r" | "adv" => "adverb".to_owned(),
        "intj" => "interjection".to_owned(),
        "prep" => "preposition".to_owned(),
        "conj" => "conjunction".to_owned(),
        "det" => "determiner".to_owned(),
        "pron" => "pronoun".to_owned(),
        "num" => "numeral".to_owned(),
        _ => word_key(pos).unwrap_or_default(),
    }
}

/// Generates English base-form candidates for a normalized `key` by suffix stripping and doubled-consonant reduction
/// (matching Enhanced's `OfflineDictionary.cpp` fallback rules).
#[must_use]
pub fn english_stem_candidates(key: &str) -> Vec<String> {
    const SUFFIXES: &[(&str, &str)] = &[
        ("ies", "y"),
        ("s", ""),
        ("es", ""),
        ("ing", ""),
        ("ing", "e"),
        ("ied", "y"),
        ("ed", ""),
        ("ed", "e"),
        ("er", ""),
        ("er", "e"),
        ("est", ""),
        ("est", "e"),
        ("'s", ""),
    ];
    let mut out = Vec::new();
    for &(end, replace) in SUFFIXES {
        if key.len() > end.len() + 1
            && let Some(base) = key.strip_suffix(end)
        {
            let mut candidate = String::with_capacity(base.len() + replace.len());
            candidate.push_str(base);
            candidate.push_str(replace);
            if !out.contains(&candidate) {
                out.push(candidate);
            }
            if matches!(end, "ing" | "ed")
                && base.len() > 2
                && base.is_ascii()
                && base.as_bytes()[base.len() - 1] == base.as_bytes()[base.len() - 2]
            {
                let undoubled = base[..base.len() - 1].to_owned();
                if !out.contains(&undoubled) {
                    out.push(undoubled);
                }
            }
        }
    }
    out
}

/// Percent-encodes a headword for Wiktionary/API URLs (preserving unreserved ASCII characters).
#[must_use]
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn decode_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        if let Some(semi) = after.find(';')
            && semi <= 12
        {
            let entity = &after[..semi];
            if let Some(decoded) = resolve_entity(entity) {
                out.push(decoded);
                rest = &after[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn resolve_entity(entity: &str) -> Option<char> {
    if let Some(hex) = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"))
    {
        let code = u32::from_str_radix(hex, 16).ok()?;
        return char::from_u32(code);
    }
    if let Some(dec) = entity.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        return char::from_u32(code);
    }
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        "hellip" => Some('…'),
        "lsquo" => Some('‘'),
        "rsquo" => Some('’'),
        "ldquo" => Some('“'),
        "rdquo" => Some('”'),
        "eacute" => Some('é'),
        "egrave" => Some('è'),
        "aacute" => Some('á'),
        "agrave" => Some('à'),
        "ouml" => Some('ö'),
        "uuml" => Some('ü'),
        "ntilde" => Some('ñ'),
        "ccedil" => Some('ç'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_key_and_plain_definition_match_enhanced() {
        assert_eq!(word_key("  (CAFÉ). "), word_key("café"));
        assert_eq!(word_key("ice_cream").as_deref(), Some("ice cream"));
        assert_eq!(word_key("..."), None);

        assert_eq!(
            plain_definition("<p>caf&eacute; &#x27;quoted&#39; &#8212; sense</p>"),
            "café 'quoted' — sense"
        );
        assert_eq!(
            plain_definition("inter<b>national</b> &amp; &#39;quoted&#39;"),
            "international & 'quoted'"
        );
        assert_eq!(
            plain_definition("<p>First meaning.</p><p>Second meaning.</p>"),
            "First meaning.\nSecond meaning."
        );
        assert_eq!(
            dictionary_plain_text("Parent meaning.: A specific sense.\nAnother sense."),
            "A specific sense.\nAnother sense."
        );
    }

    #[test]
    fn english_stem_candidates_cover_inflections_and_doubled_consonants() {
        let running = english_stem_candidates("running");
        assert!(running.contains(&"run".to_owned()));
        let cities = english_stem_candidates("cities");
        assert!(cities.contains(&"city".to_owned()));
        let cats = english_stem_candidates("cats");
        assert!(cats.contains(&"cat".to_owned()));
        let stopped = english_stem_candidates("stopped");
        assert!(stopped.contains(&"stop".to_owned()));
    }
}
