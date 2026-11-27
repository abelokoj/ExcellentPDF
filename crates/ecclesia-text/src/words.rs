//! Which characters make a word: what a double-click selects.

/// What a character is to word selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// Spaces: a run of them is selected together.
    Space,
    /// Letters, digits and the joiners inside words ("don't", "snake_case").
    Word,
    /// A character that is a word by itself: punctuation, and ideographs and kana, which are written without spaces.
    Alone,
}

/// The class of `c`.
pub(crate) fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if is_written_without_spaces(c) {
        Class::Alone
    } else if c.is_alphanumeric() || matches!(c, '_' | '\'' | '’') {
        Class::Word
    } else {
        Class::Alone
    }
}

/// Han ideographs, kana and their symbols: one character per word until ICU4X segments them (K.13 step 3).
fn is_written_without_spaces(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF00}'..='\u{FFEF}'
            | '\u{20000}'..='\u{2FA1F}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_inner_joiners_make_words() {
        for c in ['a', 'Z', '7', 'é', 'ß', 'я', '_', '\''] {
            assert_eq!(class(c), Class::Word, "{c}");
        }
        for c in [' ', '\t', '\u{a0}'] {
            assert_eq!(class(c), Class::Space, "{c:?}");
        }
        for c in [',', '(', '—', '中', 'あ', 'カ'] {
            assert_eq!(class(c), Class::Alone, "{c}");
        }
    }
}
