//! Localization (plan U.7). Every string the UI shows is a key in `app/locales/`, English first, read through `t!`.
//!
//! Each language is one file, `app/locales/<tag>.json`, built in ([`LOCALES`]); a key a language lacks shows in
//! English (NSH's fallback). The reader picks the language in Settings (`language`: a tag, or nothing for the
//! system's); right-to-left languages mirror the window ([`direction`]). Debug builds also carry `en-XA`, a
//! pseudo-locale made from English ([`pseudo`]), so a test can find text that doesn't go through `t!`.

use std::collections::HashMap;
use std::sync::OnceLock;

use nsh_core::element::{LayoutDirection, locale as current_locale, locale_settings, untracked};
use nsh_core::i18n::{Translations, add_translations, available_locales, set_locale};
use nsh_core::platform::Locale;

use crate::settings::{self, Settings};

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

/// Cached startup translations: English (the fallback), plus the pseudo-locale in debug builds.
/// Other locales in [`LOCALES`] are parsed on demand when selected or requested by the system.
static BASE_TRANSLATIONS: OnceLock<Translations> = OnceLock::new();

fn base_translations() -> &'static Translations {
    BASE_TRANSLATIONS.get_or_init(|| {
        let english = parse(LOCALES[0].0, LOCALES[0].1);
        let mut map = HashMap::with_capacity(if cfg!(debug_assertions) { 2 } else { 1 });
        if cfg!(debug_assertions) {
            let pseudo = english
                .iter()
                .map(|(key, text)| (key.clone(), self::pseudo(text)))
                .collect();
            map.insert(PSEUDO.to_owned(), pseudo);
        }
        map.insert(LOCALES[0].0.to_owned(), english);
        Translations::from_map(map)
    })
}

/// Finds the built-in locale entry matching `tag` or any of its shorter subtag prefixes (`ar-EG` -> `ar`).
fn matching_builtin(tag: &str) -> Option<(&'static str, &'static str)> {
    let mut candidate = Some(tag);
    while let Some(prefix) = candidate {
        if let Some(&entry) = LOCALES.iter().find(|(id, _)| *id == prefix) {
            return Some(entry);
        }
        candidate = prefix.rfind('-').map(|idx| &prefix[..idx]);
    }
    None
}

/// Ensures the built-in locale matching `tag` is loaded into the active surface's translations.
/// Returns `true` if `tag` matched a built-in locale (whether already loaded or newly merged).
fn ensure_loaded(tag: &str) -> bool {
    let Some((builtin_tag, json)) = matching_builtin(tag) else {
        return false;
    };
    if builtin_tag == LOCALES[0].0 {
        return true;
    }
    let already_loaded = untracked(available_locales)
        .iter()
        .any(|loaded| loaded == builtin_tag);
    if !already_loaded {
        let mut extra = HashMap::with_capacity(1);
        extra.insert(builtin_tag.to_owned(), parse(builtin_tag, json));
        add_translations(extra);
    }
    true
}

/// The app's startup translations: cached English (plus the pseudo-locale in debug builds), and
/// the user's configured non-English language if one is already chosen in settings.
/// Other built-in languages are loaded lazily by [`apply`].
pub(crate) fn translations() -> Translations {
    let base = base_translations().clone();
    let configured = chosen(&settings::shared());
    if let Some(locale) = configured
        && let Some((tag, json)) = matching_builtin(locale.as_str())
        && tag != LOCALES[0].0
    {
        let mut extra = HashMap::with_capacity(1);
        extra.insert(tag.to_owned(), parse(tag, json));
        return base.extend(extra);
    }
    base
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
    const LOWER: [char; 26] = [
        'å', 'ƀ', 'ç', 'đ', 'é', 'ƒ', 'ĝ', 'ĥ', 'î', 'ĵ', 'ķ', 'ļ', 'ɱ', 'ñ', 'ö', 'þ', 'ǫ', 'ŕ',
        'š', 'ţ', 'û', 'ṽ', 'ŵ', 'ẋ', 'ý', 'ž',
    ];
    const UPPER: [char; 26] = [
        'Å', 'ß', 'Ç', 'Đ', 'É', 'Ƒ', 'Ĝ', 'Ĥ', 'Î', 'Ĵ', 'Ķ', 'Ļ', 'Ṁ', 'Ñ', 'Ö', 'Þ', 'Ǫ', 'Ŕ',
        'Š', 'Ţ', 'Û', 'Ṽ', 'Ŵ', 'Ẋ', 'Ý', 'Ž',
    ];
    match c {
        'a'..='z' => LOWER.get((c as u8 - b'a') as usize).copied().unwrap_or(c),
        'A'..='Z' => UPPER.get((c as u8 - b'A') as usize).copied().unwrap_or(c),
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

/// Shows the window in the language `settings` choose, or the system's, lazily loading any
/// built-in locale that was deferred at cold start.
pub(crate) fn apply(settings: &Settings) {
    let picked = chosen(settings);
    match picked {
        Some(locale) => {
            ensure_loaded(locale.as_str());
        }
        None => {
            let mut matched = false;
            for lang in locale_settings().languages.iter() {
                if ensure_loaded(lang.as_str()) {
                    matched = true;
                    break;
                }
            }
            if !matched {
                ensure_loaded(current_locale().as_str());
            }
        }
    }
    set_locale(picked);
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
