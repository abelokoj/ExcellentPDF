# Ph1: 59/60 | U.7: Localization

**Tag:** U.7; **Priority:** P0 infrastructure, P1 languages; **Phase:** 1; **Needs:** K.1; **Code:** `app/locales/*`,
`ecclesia-app::l10n`
**Plan:** §27.18 entry `Ph1: 59/60 | U.7`.

## Problem

A reader who doesn't read English can't use the app, and a reader of Arabic expects the window to run right to left.
Text that skips the localization layer stays English in every language.

## User story

As a reader of Arabic, I want the app in my language, laid out from the right, whatever language Windows is in.

## UI sketch

- **Settings, Appearance, Language:** *Same as the system* (the default), *English*, *العربية*. Each language is
  named in its own script, as every language menu names them. The choice applies at once; Arabic mirrors the window
  (title bar, docks, rows and dialogs run from the right).
- With *Same as the system*, the window follows Windows' display language, and its direction.

## Mode

All modes.

## Speed and size impact

- Startup: each language file is parsed once when the root is made (432 keys each; **not measured**). Debug builds
  also build the pseudo-locale; release builds don't.
- Installer: about 30 KB of text per language.

## Design

- One file per language, `app/locales/<tag>.json`, keys and text, built in through `l10n::LOCALES`; NSH's
  translations (rust-i18n's lookup) fall back to English for a key a language lacks. A unit test reads the folder:
  every file must be in `LOCALES`, have every English key and no other, and keep each text's `%{name}` arguments.
- Pseudo-locale `en-XA` (debug builds): each English text with its letters accented and bracketed, `⟦Ĥöɱé⟧`, its
  arguments untouched. The headless test starts the window in it and fails on any shown text without brackets.
- Language choice: the `language` setting (a tag, or absent for the system's) goes to NSH's `set_locale`; the
  shell's root view takes `LayoutDirection::Rtl` for a right-to-left language, `Ltr` for another, and inherits the
  system's when none is chosen. NSH's `set_locale` changes the text, not the direction, so the app sets that itself.
- Arabic: every key translated (`app/locales/ar.json`), by the agent; **a native speaker has not reviewed it**.
- Vertical CJK in ebooks: MuPDF 1.28.5's HTML engine has no `writing-mode` (no match in `source/html/`), so vertical
  Japanese and Chinese books lay out horizontally. Recorded as a gap; it needs a MuPDF patch.
- Also fixed: `settings.theme_high` used `{name}`, not rust-i18n's `%{name}`, so the name never showed.
- Not built: plural rules (no text needs them yet; Fluent stays the fallback the plan names); other languages (P1,
  as translations come in); vertical CJK (above).

## Acceptance tests

- [x] `every_locale_file_is_built_in_and_has_every_english_key`
- [x] `the_pseudo_locale_accents_letters_and_keeps_arguments`
- [x] `arabic_mirrors_the_window_and_english_does_not`
- [x] `every_text_goes_through_the_localization_layer` (the pseudo-locale test)
- [x] `the_window_works_in_arabic`
- [ ] the UI works in Arabic: the headless test passes; **not looked at by hand**, and the translation is not
  reviewed by a native speaker
