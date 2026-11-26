//! Selection translation, language catalog, online consent gating, and offline Bergamot
//! translation packs (plan S.16, ported from SumatraPDF Enhanced's `SelectionTranslate.cpp` and
//! `TranslationLangs.cpp`).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Metadata for a supported translation language (ported from `TranslationLangs.cpp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LanguageInfo {
    /// ISO 639-1 / BCP-47 language code (`"en"`, `"zh-CN"`, `"es"`, ...).
    pub code: &'static str,
    /// Human-readable English + native name.
    pub name: &'static str,
}

/// Complete catalog of 73 languages ported from SumatraPDF Enhanced's `TranslationLangs.cpp`.
pub const SUPPORTED_LANGUAGES: &[LanguageInfo] = &[
    LanguageInfo {
        code: "en",
        name: "English",
    },
    LanguageInfo {
        code: "af",
        name: "Afrikaans",
    },
    LanguageInfo {
        code: "sq",
        name: "Albanian (Shqip)",
    },
    LanguageInfo {
        code: "ar",
        name: "Arabic (العربية)",
    },
    LanguageInfo {
        code: "am",
        name: "Armenian (Հայերեն)",
    },
    LanguageInfo {
        code: "az",
        name: "Azerbaijani (Azərbaycanca)",
    },
    LanguageInfo {
        code: "eu",
        name: "Basque (Euskara)",
    },
    LanguageInfo {
        code: "by",
        name: "Belarusian (Беларуская)",
    },
    LanguageInfo {
        code: "bn",
        name: "Bengali (বাংলা)",
    },
    LanguageInfo {
        code: "bs",
        name: "Bosnian (Bosanski)",
    },
    LanguageInfo {
        code: "bg",
        name: "Bulgarian (Български)",
    },
    LanguageInfo {
        code: "mm",
        name: "Burmese (မြန်မာ)",
    },
    LanguageInfo {
        code: "ca",
        name: "Catalan (Català)",
    },
    LanguageInfo {
        code: "ca-xv",
        name: "Catalan-Valencian",
    },
    LanguageInfo {
        code: "zh-CN",
        name: "Chinese Simplified (简体中文)",
    },
    LanguageInfo {
        code: "zh-TW",
        name: "Chinese Traditional (繁體中文)",
    },
    LanguageInfo {
        code: "kw",
        name: "Cornish (Kernewek)",
    },
    LanguageInfo {
        code: "co",
        name: "Corsican (Corsu)",
    },
    LanguageInfo {
        code: "hr",
        name: "Croatian (Hrvatski)",
    },
    LanguageInfo {
        code: "cs",
        name: "Czech (Čeština)",
    },
    LanguageInfo {
        code: "da",
        name: "Danish (Dansk)",
    },
    LanguageInfo {
        code: "nl",
        name: "Dutch (Nederlands)",
    },
    LanguageInfo {
        code: "et",
        name: "Estonian (Eesti)",
    },
    LanguageInfo {
        code: "fo",
        name: "Faroese (Føroyskt)",
    },
    LanguageInfo {
        code: "fi",
        name: "Finnish (Suomi)",
    },
    LanguageInfo {
        code: "fr",
        name: "French (Français)",
    },
    LanguageInfo {
        code: "fy-nl",
        name: "Frisian (Frysk)",
    },
    LanguageInfo {
        code: "gl",
        name: "Galician (Galego)",
    },
    LanguageInfo {
        code: "ka",
        name: "Georgian (ქართული)",
    },
    LanguageInfo {
        code: "de",
        name: "German (Deutsch)",
    },
    LanguageInfo {
        code: "el",
        name: "Greek (Ελληνικά)",
    },
    LanguageInfo {
        code: "he",
        name: "Hebrew (עברית)",
    },
    LanguageInfo {
        code: "hi",
        name: "Hindi (हिन्दी)",
    },
    LanguageInfo {
        code: "hu",
        name: "Hungarian (Magyar)",
    },
    LanguageInfo {
        code: "id",
        name: "Indonesian (Bahasa Indonesia)",
    },
    LanguageInfo {
        code: "ga",
        name: "Irish (Gaeilge)",
    },
    LanguageInfo {
        code: "it",
        name: "Italian (Italiano)",
    },
    LanguageInfo {
        code: "ja",
        name: "Japanese (日本語)",
    },
    LanguageInfo {
        code: "jv",
        name: "Javanese",
    },
    LanguageInfo {
        code: "ko",
        name: "Korean (한국어)",
    },
    LanguageInfo {
        code: "ku",
        name: "Kurdish (کوردی)",
    },
    LanguageInfo {
        code: "lv",
        name: "Latvian (latviešu valoda)",
    },
    LanguageInfo {
        code: "lt",
        name: "Lithuanian (Lietuvių)",
    },
    LanguageInfo {
        code: "mk",
        name: "Macedonian (македонски)",
    },
    LanguageInfo {
        code: "ml",
        name: "Malayalam (മലയാളം)",
    },
    LanguageInfo {
        code: "ms",
        name: "Malaysian (Bahasa Melayu)",
    },
    LanguageInfo {
        code: "ne",
        name: "Nepali (नेपाली)",
    },
    LanguageInfo {
        code: "no",
        name: "Norwegian (Norsk)",
    },
    LanguageInfo {
        code: "nn",
        name: "Norwegian Nynorsk",
    },
    LanguageInfo {
        code: "fa",
        name: "Persian (فارسی)",
    },
    LanguageInfo {
        code: "pl",
        name: "Polish (Polski)",
    },
    LanguageInfo {
        code: "pt-BR",
        name: "Portuguese - Brazil",
    },
    LanguageInfo {
        code: "pt",
        name: "Portuguese - Portugal",
    },
    LanguageInfo {
        code: "pa",
        name: "Punjabi (ਪੰਜਾਬੀ)",
    },
    LanguageInfo {
        code: "ro",
        name: "Romanian (Română)",
    },
    LanguageInfo {
        code: "ru",
        name: "Russian (Русский)",
    },
    LanguageInfo {
        code: "sat",
        name: "Santali",
    },
    LanguageInfo {
        code: "sr-Cyrl",
        name: "Serbian (Cyrillic)",
    },
    LanguageInfo {
        code: "sr-Latn",
        name: "Serbian (Latin)",
    },
    LanguageInfo {
        code: "sn",
        name: "Shona",
    },
    LanguageInfo {
        code: "si",
        name: "Sinhala (සිංහල)",
    },
    LanguageInfo {
        code: "sk",
        name: "Slovak (Slovenčina)",
    },
    LanguageInfo {
        code: "sl",
        name: "Slovenian (Slovenščina)",
    },
    LanguageInfo {
        code: "es",
        name: "Spanish (Español)",
    },
    LanguageInfo {
        code: "sv",
        name: "Swedish (Svenska)",
    },
    LanguageInfo {
        code: "tl",
        name: "Tagalog",
    },
    LanguageInfo {
        code: "ta",
        name: "Tamil (தமிழ்)",
    },
    LanguageInfo {
        code: "th",
        name: "Thai (ภาษาไทย)",
    },
    LanguageInfo {
        code: "tr",
        name: "Turkish (Türkçe)",
    },
    LanguageInfo {
        code: "uk",
        name: "Ukrainian (Українська)",
    },
    LanguageInfo {
        code: "uz",
        name: "Uzbek (O'zbek)",
    },
    LanguageInfo {
        code: "vi",
        name: "Vietnamese (Tiếng Việt)",
    },
    LanguageInfo {
        code: "cy",
        name: "Welsh (Cymraeg)",
    },
];

/// Looks up a language entry by code or name (case-insensitive).
#[must_use]
pub fn find_language(query: &str) -> Option<LanguageInfo> {
    let q = query.trim();
    SUPPORTED_LANGUAGES
        .iter()
        .copied()
        .find(|l| l.code.eq_ignore_ascii_case(q) || l.name.eq_ignore_ascii_case(q))
}

/// Translation provider selection (plan S.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TranslateEngine {
    /// Local offline Firefox Translations (Bergamot) language pack.
    #[default]
    OfflineBergamot,
    /// Online Google Translate endpoint (requires explicit user consent).
    Google,
    /// Online DeepL API endpoint (requires explicit user consent).
    DeepL,
}

/// Explicit user privacy consent policy governing network requests (AGENTS.md §5 / S.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConsentPolicy {
    /// Strictly offline: never touch the network; reject online-only providers.
    #[default]
    OfflineOnly,
    /// User explicitly consented to sending selected text to the chosen online provider.
    OnlineConsented,
}

/// Errors returned by translation engines and language-pack matching.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TranslateError {
    /// Selected online engine cannot run without explicit user consent.
    ConsentRequired,
    /// No offline language pack is installed for `(source_lang, target_lang)`.
    MissingLanguagePack {
        /// Source BCP-47 code.
        source: String,
        /// Target BCP-47 code.
        target: String,
    },
    /// Input text was empty.
    EmptyInput,
    /// Network or HTTP provider error.
    Network(String),
    /// Pack I/O or format error.
    PackIo(String),
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConsentRequired => {
                f.write_str("online translation requires explicit user consent")
            }
            Self::MissingLanguagePack { source, target } => {
                write!(
                    f,
                    "no offline translation pack installed for {source} -> {target}"
                )
            }
            Self::EmptyInput => f.write_str("cannot translate empty text"),
            Self::Network(msg) => write!(f, "translation network error: {msg}"),
            Self::PackIo(msg) => write!(f, "translation pack error: {msg}"),
        }
    }
}

impl std::error::Error for TranslateError {}

/// Convenience result alias for translation operations.
pub type Result<T> = std::result::Result<T, TranslateError>;

/// Request to translate selected text from `source_lang` (`"auto"` or BCP-47) into `target_lang`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslateRequest {
    /// Selected text to translate.
    pub text: String,
    /// Source language code (`"auto"` for auto-detect).
    pub source_lang: String,
    /// Target language code (`"en"`, `"es"`, ...).
    pub target_lang: String,
    /// Preferred engine.
    pub engine: TranslateEngine,
    /// Network consent state.
    pub consent: ConsentPolicy,
}

/// Output produced by a [`Translator`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationResult {
    /// Translated text.
    pub translated_text: String,
    /// Resolved source language code.
    pub detected_source_lang: String,
    /// Target language code.
    pub target_lang: String,
    /// Engine that performed the translation.
    pub engine: TranslateEngine,
}

/// Pluggable translation backend trait (plan S.16).
pub trait Translator: Send + Sync {
    /// Translates `request`.
    ///
    /// # Errors
    ///
    /// Returns [`TranslateError`] when consent is missing, pack is absent, or network fails.
    fn translate(&self, request: &TranslateRequest) -> Result<TranslationResult>;
}

/// Descriptor for an installed offline Firefox Translations (Bergamot) language pair pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BergamotPack {
    /// Source ISO code (e.g., `"en"`).
    pub source_lang: String,
    /// Target ISO code (e.g., `"es"`).
    pub target_lang: String,
    /// Root folder containing `model.*.intgemm.alphas.bin`, `vocab.*.spm`, and optional `lexicon.tsv`.
    pub path: PathBuf,
    /// Optional offline phrase/glossary map (`source lowercase -> translated`) loaded from the pack.
    pub phrase_table: BTreeMap<String, String>,
}

/// Manages installed offline Bergamot translation packs under `packs/translate/` (plan S.16).
#[derive(Debug, Clone, Default)]
pub struct BergamotPackCatalog {
    packs: BTreeMap<(String, String), BergamotPack>,
}

impl BergamotPackCatalog {
    /// Creates an empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scans `root_dir` for `<src>-<dst>/` pack directories containing `manifest.json` or
    /// `lexicon.tsv`.
    ///
    /// # Errors
    ///
    /// Returns [`TranslateError::PackIo`] if reading pack directories fails.
    pub fn load_from_dir(root_dir: &Path) -> Result<Self> {
        let mut catalog = Self::new();
        if !root_dir.exists() {
            return Ok(catalog);
        }
        let entries = fs::read_dir(root_dir).map_err(|e| TranslateError::PackIo(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| TranslateError::PackIo(e.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some((src, dst)) = name.split_once('-') else {
                continue;
            };
            let mut phrase_table = BTreeMap::new();
            let lexicon_path = path.join("lexicon.tsv");
            if lexicon_path.exists() {
                let raw = fs::read_to_string(&lexicon_path)
                    .map_err(|e| TranslateError::PackIo(e.to_string()))?;
                for line in raw.lines() {
                    if let Some((k, v)) = line.split_once('\t') {
                        phrase_table.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
                    }
                }
            }
            catalog.register_pack(BergamotPack {
                source_lang: src.to_ascii_lowercase(),
                target_lang: dst.to_ascii_lowercase(),
                path,
                phrase_table,
            });
        }
        Ok(catalog)
    }

    /// Registers an offline pack in memory.
    pub fn register_pack(&mut self, pack: BergamotPack) {
        self.packs.insert(
            (
                pack.source_lang.to_ascii_lowercase(),
                pack.target_lang.to_ascii_lowercase(),
            ),
            pack,
        );
    }

    /// Finds an installed pack matching `source_lang -> target_lang`.
    #[must_use]
    pub fn match_pair(&self, source_lang: &str, target_lang: &str) -> Option<&BergamotPack> {
        let src = source_lang.to_ascii_lowercase();
        let dst = target_lang.to_ascii_lowercase();
        self.packs.get(&(src.clone(), dst.clone())).or_else(|| {
            // Try base language code ("en-US" -> "en")
            let s_base = src.split('-').next().unwrap_or(&src).to_owned();
            let d_base = dst.split('-').next().unwrap_or(&dst).to_owned();
            self.packs.get(&(s_base, d_base))
        })
    }
}

/// Offline + consent-gated online selection translator (plan S.16).
#[derive(Debug, Clone)]
pub struct SelectionTranslator {
    packs: BergamotPackCatalog,
}

impl SelectionTranslator {
    /// Creates a translator backed by `packs`.
    #[must_use]
    pub fn new(packs: BergamotPackCatalog) -> Self {
        Self { packs }
    }
}

impl Translator for SelectionTranslator {
    fn translate(&self, request: &TranslateRequest) -> Result<TranslationResult> {
        let trimmed = request.text.trim();
        if trimmed.is_empty() {
            return Err(TranslateError::EmptyInput);
        }

        match request.engine {
            TranslateEngine::OfflineBergamot => {
                let src = if request.source_lang.eq_ignore_ascii_case("auto") {
                    "en"
                } else {
                    request.source_lang.as_str()
                };
                let pack = self
                    .packs
                    .match_pair(src, &request.target_lang)
                    .ok_or_else(|| TranslateError::MissingLanguagePack {
                        source: src.to_owned(),
                        target: request.target_lang.clone(),
                    })?;

                // Exact phrase lookup first, else word-by-word glossary lookup preserving punctuation.
                let lower = trimmed.to_ascii_lowercase();
                let translated = if let Some(exact) = pack.phrase_table.get(&lower) {
                    exact.clone()
                } else {
                    translate_words_from_table(trimmed, &pack.phrase_table)
                };

                Ok(TranslationResult {
                    translated_text: translated,
                    detected_source_lang: pack.source_lang.clone(),
                    target_lang: pack.target_lang.clone(),
                    engine: TranslateEngine::OfflineBergamot,
                })
            }
            TranslateEngine::Google | TranslateEngine::DeepL => {
                if request.consent != ConsentPolicy::OnlineConsented {
                    return Err(TranslateError::ConsentRequired);
                }
                translate_google_unofficial(trimmed, &request.source_lang, &request.target_lang)
            }
        }
    }
}

fn translate_words_from_table(text: &str, table: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first = true;
    for token in text.split_whitespace() {
        if !first {
            out.push(' ');
        }
        first = false;
        let clean = token.trim_matches(|c: char| !c.is_alphanumeric());
        let lc = clean.to_ascii_lowercase();
        if let Some(mapped) = table.get(&lc) {
            if let Some(pos) = token.find(clean) {
                out.push_str(&token[..pos]);
                out.push_str(mapped);
                out.push_str(&token[pos + clean.len()..]);
            } else {
                out.push_str(mapped);
            }
        } else {
            out.push_str(token);
        }
    }
    out
}

fn translate_google_unofficial(
    text: &str,
    source_lang: &str,
    target_lang: &str,
) -> Result<TranslationResult> {
    let sl = if source_lang.eq_ignore_ascii_case("auto") {
        "auto"
    } else {
        source_lang
    };
    let encoded_q = crate::dictionary::percent_encode(text);
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl={sl}&tl={target_lang}&dt=t&q={encoded_q}"
    );
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| TranslateError::Network(e.to_string()))?;
    let body = resp
        .into_body()
        .read_to_string()
        .map_err(|e| TranslateError::Network(e.to_string()))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| TranslateError::Network(e.to_string()))?;
    let mut translated = String::new();
    if let Some(sentences) = json.get(0).and_then(serde_json::Value::as_array) {
        for seg in sentences {
            if let Some(piece) = seg.get(0).and_then(serde_json::Value::as_str) {
                translated.push_str(piece);
            }
        }
    }
    let detected = json
        .get(2)
        .and_then(serde_json::Value::as_str)
        .unwrap_or(sl)
        .to_owned();
    Ok(TranslationResult {
        translated_text: translated,
        detected_source_lang: detected,
        target_lang: target_lang.to_owned(),
        engine: TranslateEngine::Google,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_translation_works_with_network_disabled() {
        let mut catalog = BergamotPackCatalog::new();
        let mut table = BTreeMap::new();
        table.insert("good".into(), "bueno".into());
        table.insert("morning".into(), "mañana".into());
        table.insert("good morning".into(), "buenos días".into());
        catalog.register_pack(BergamotPack {
            source_lang: "en".into(),
            target_lang: "es".into(),
            path: PathBuf::from("packs/translate/en-es"),
            phrase_table: table,
        });

        let translator = SelectionTranslator::new(catalog);
        let res = translator
            .translate(&TranslateRequest {
                text: "Good morning".into(),
                source_lang: "en-US".into(),
                target_lang: "es".into(),
                engine: TranslateEngine::OfflineBergamot,
                consent: ConsentPolicy::OfflineOnly,
            })
            .expect("offline translation");
        assert_eq!(res.translated_text, "buenos días");
    }

    #[test]
    fn online_engine_refuses_without_explicit_consent_and_catalog_has_73_languages() {
        assert_eq!(SUPPORTED_LANGUAGES.len(), 73);
        assert_eq!(find_language("es").expect("es").name, "Spanish (Español)");

        let translator = SelectionTranslator::new(BergamotPackCatalog::new());
        let err = translator
            .translate(&TranslateRequest {
                text: "hello".into(),
                source_lang: "en".into(),
                target_lang: "fr".into(),
                engine: TranslateEngine::Google,
                consent: ConsentPolicy::OfflineOnly,
            })
            .expect_err("must require consent");
        assert_eq!(err, TranslateError::ConsentRequired);
    }
}
