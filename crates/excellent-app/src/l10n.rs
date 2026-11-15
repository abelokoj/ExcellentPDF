//! Localization (plan U.7). Every string the UI shows is a key in `app/locales/`, English first, read through `t!`.
//!
//! Each language is one file, `app/locales/<tag>.json`, built in ([`LOCALES`]); a key a language lacks shows in
//! English (NSH's fallback). The reader picks the language in Settings (`language`: a tag, or nothing for the
//! system's); right-to-left languages mirror the window ([`direction`]). Debug builds also carry `en-XA`, a
//! pseudo-locale made from English ([`pseudo`]), so a test can find text that doesn't go through `t!`.

use std::collections::HashMap;

use nsh_core::element::LayoutDirection;
use nsh_core::i18n::{Translations, set_locale};
use nsh_core::platform::Locale;

use crate::settings::Settings;

/// Every language file: its tag and its text. A new language is one line here and its file.
pub(crate) const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../../../app/locales/en.json")),
    ("ar", include_str!("../../../app/locales/ar.json")),
];

/// `app/locales/en.json`, the source every other language translates (the tests read it).
#[cfg(test)]
pub(crate) const EN: &str = LOCALES[0].1;

/// The languages the reader can pick, by tag, each named in its own language (as every language menu names them,
/// so a reader finds theirs whatever the window shows now).
pub(crate) const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("ar", "العربية")];

/// Languages written right to left (their base tags).
const RIGHT_TO_LEFT: &[&str] = &["ar", "fa", "he", "ur"];

/// The pseudo-locale's tag (U.7's test): English with every letter accented and each text bracketed.
pub(crate) const PSEUDO: &str = "en-XA";

/// The settings key of the reader's language: a tag from [`LANGUAGES`], or absent for the system's.
pub(crate) const LANGUAGE_KEY: &str = "language";

/// The app's translations: every file in [`LOCALES`], and the pseudo-locale in debug builds.
pub(crate) fn translations() -> Translations {
    let mut map: HashMap<String, HashMap<String, String>> = LOCALES
        .iter()
        .map(|(tag, json)| (tag.to_string(), parse(tag, json)))
        .collect();
    if cfg!(debug_assertions) {
        let english = &map["en"];
        let pseudo = english
            .iter()
            .map(|(key, text)| (key.clone(), self::pseudo(text)))
            .collect();
        map.insert(PSEUDO.to_owned(), pseudo);
    }
    Translations::from_map(map)
}

fn parse(tag: &str, json: &str) -> HashMap<String, String> {
    serde_json::from_str(json)
        .unwrap_or_else(|e| panic!("app/locales/{tag}.json is one JSON object of strings: {e}"))
}

/// `text` as the pseudo-locale shows it: `⟦Ŝéţţîñĝš⟧`, its `%{name}` arguments kept as they are.
pub(crate) fn pseudo(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2 + 6);
    out.push('⟦');
    let mut rest = text;
    while let Some(start) = rest.find("%{") {
        out.extend(rest[..start].chars().map(accented));
        let end = rest[start..]
            .find('}')
            .map_or(rest.len(), |end| start + end + 1);
        out.push_str(&rest[start..end]);
        rest = &rest[end..];
    }
    out.extend(rest.chars().map(accented));
    out.push('⟧');
    out
}

/// An ASCII letter with an accent, still readable; anything else as it is.
fn accented(c: char) -> char {
    const LOWER: &str = "åƀçđéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýž";
    const UPPER: &str = "ÅßÇĐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ";
    match c {
        'a'..='z' => LOWER.chars().nth(c as usize - 'a' as usize).unwrap_or(c),
        'A'..='Z' => UPPER.chars().nth(c as usize - 'A' as usize).unwrap_or(c),
        _ => c,
    }
}

/// The language the reader chose in `settings`, if it is one the app has.
pub(crate) fn chosen(settings: &Settings) -> Option<Locale> {
    let tag = settings.text(LANGUAGE_KEY)?;
    let known = LANGUAGES.iter().any(|(id, _)| *id == tag) || tag == PSEUDO;
    if !known {
        log::warn!("settings: no language `{tag}`; following the system's");
        return None;
    }
    Locale::parse(tag)
}

/// Shows the window in the language `settings` choose, or the system's.
pub(crate) fn apply(settings: &Settings) {
    set_locale(chosen(settings));
}

/// The window's direction for the language `settings` choose: the system's when they choose none.
pub(crate) fn direction(settings: &Settings) -> LayoutDirection {
    match chosen(settings) {
        None => LayoutDirection::Inherit,
        Some(locale) if is_right_to_left(locale.as_str()) => LayoutDirection::Rtl,
        Some(_) => LayoutDirection::Ltr,
    }
}

/// Whether `tag`'s language is written right to left.
fn is_right_to_left(tag: &str) -> bool {
    let base = tag.split('-').next().unwrap_or(tag);
    RIGHT_TO_LEFT.contains(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_file_is_built_in_and_has_every_english_key() {
        let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/locales");
        let mut on_disk: Vec<String> = std::fs::read_dir(folder)
            .expect("app/locales")
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                name.strip_suffix(".json").map(str::to_owned)
            })
            .collect();
        on_disk.sort();
        let mut built_in: Vec<String> = LOCALES.iter().map(|(tag, _)| tag.to_string()).collect();
        built_in.sort();
        assert_eq!(on_disk, built_in, "every file in app/locales is in LOCALES");

        let english = parse("en", LOCALES[0].1);
        for (tag, json) in &LOCALES[1..] {
            let other = parse(tag, json);
            let mut missing: Vec<&String> = english
                .keys()
                .filter(|key| !other.contains_key(*key))
                .collect();
            missing.sort();
            assert!(missing.is_empty(), "{tag} lacks {missing:?}");
            let mut extra: Vec<&String> = other
                .keys()
                .filter(|key| !english.contains_key(*key))
                .collect();
            extra.sort();
            assert!(
                extra.is_empty(),
                "{tag} has keys English doesn't: {extra:?}"
            );
            for (key, text) in &english {
                assert_eq!(
                    arguments(text),
                    arguments(&other[key]),
                    "{tag}: {key} keeps its arguments"
                );
            }
        }
    }

    /// The `%{name}`s of `text`, sorted.
    fn arguments(text: &str) -> Vec<&str> {
        let mut names: Vec<&str> = text
            .match_indices("%{")
            .filter_map(|(start, _)| {
                let end = text[start..].find('}')?;
                Some(&text[start + 2..start + end])
            })
            .collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn the_pseudo_locale_accents_letters_and_keeps_arguments() {
        assert_eq!(pseudo("Go to page %{page}"), "⟦Ĝö ţö þåĝé %{page}⟧");
        assert_eq!(pseudo("%{n} of %{total}"), "⟦%{n} öƒ %{total}⟧");
        assert!(translations().locales().iter().any(|tag| tag == PSEUDO));
    }

    #[test]
    fn arabic_mirrors_the_window_and_english_does_not() {
        let mut settings = Settings::default();
        assert_eq!(direction(&settings), LayoutDirection::Inherit);
        settings.set_text(LANGUAGE_KEY, Some("ar"));
        assert_eq!(direction(&settings), LayoutDirection::Rtl);
        settings.set_text(LANGUAGE_KEY, Some("en"));
        assert_eq!(direction(&settings), LayoutDirection::Ltr);
        settings.set_text(LANGUAGE_KEY, Some("xx"));
        assert_eq!(chosen(&settings), None);
    }
}
