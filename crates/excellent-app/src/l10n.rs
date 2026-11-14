//! Localization (plan U.7). Every string the UI shows is a key in `app/locales/`, English first, read through `t!`.

use nsh_core::i18n::Translations;

/// `app/locales/en.json`: keys and their English text.
pub(crate) const EN: &str = include_str!("../../../app/locales/en.json");

// shortcut: English only, wrapped by hand; read every file in app/locales/ (a build script) when U.7 adds languages.
pub(crate) fn translations() -> Translations {
    Translations::from_json(&format!("{{\"en\":{EN}}}"))
        .expect("app/locales/en.json is one JSON object of strings")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_loads() {
        assert_eq!(translations().locales(), ["en"]);
    }
}
