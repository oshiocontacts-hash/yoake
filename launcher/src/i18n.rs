//! The launcher's languages: Japanese (first) and English (the strings in the code). The Japanese
//! table (`launcher/lang/ja.json`) maps each English string to its translation; a string it lacks
//! shows in English. The other `lang/*.json` files are kept for merging with upstream but are not
//! loaded or offered.
//!
//! The first run starts in English on an English system, else in Japanese; the player can pick
//! one in Settings (`Settings::language`). Translations are Horizon's own, written for this launcher.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Language {
    #[default]
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "en")]
    English,
}

/// A saved language other than `en` is shown in Japanese; the settings file is left as it is.
impl<'de> Deserialize<'de> for Language {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        Ok(Self::parse(&code).unwrap_or(Self::Japanese))
    }
}

impl Language {
    pub const ALL: [Language; 2] = [Self::Japanese, Self::English];

    /// Its code in `settings.json` and on the command line.
    pub fn code(self) -> &'static str {
        match self {
            Self::Japanese => "ja",
            Self::English => "en",
        }
    }

    /// Its name in itself: what a player looks for in a list of languages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Japanese => "日本語",
            Self::English => "English",
        }
    }

    /// Parses `ja` or `en` (also `ja-JP`, `en_US.UTF-8`); any other language is `None`.
    pub fn parse(code: &str) -> Option<Self> {
        let lower = code.trim().to_ascii_lowercase();
        let clean = lower.split(['.', '@']).next().unwrap_or("");
        let normalized = clean.replace('_', "-");
        let base = normalized.split('-').next().unwrap_or("");
        Self::ALL
            .into_iter()
            .find(|language| language.code() == base)
    }

    /// The first of the system's preferred languages decides: English if it is English, else
    /// Japanese (also when there is none).
    pub fn pick<'a>(preferred: impl IntoIterator<Item = &'a str>) -> Self {
        match preferred.into_iter().next().and_then(Self::parse) {
            Some(Self::English) => Self::English,
            _ => Self::Japanese,
        }
    }

    /// The default language: English on an English system, Japanese otherwise.
    pub fn system() -> Self {
        let locales: Vec<String> = sys_locale::get_locales().collect();
        Self::pick(locales.iter().map(String::as_str))
    }

    /// The table, as shipped: English text to this language's.
    fn source(self) -> &'static str {
        match self {
            Self::English => "{}",
            Self::Japanese => include_str!("../lang/ja.json"),
        }
    }
}

/// English (the strings in the code) until the window sets its language.
static CURRENT: AtomicU8 = AtomicU8::new(1);

/// Shows the launcher in `language` from now on.
pub fn set(language: Language) {
    let i = Language::ALL
        .iter()
        .position(|l| *l == language)
        .unwrap_or(0);
    CURRENT.store(i as u8, Ordering::Relaxed);
}

pub fn current() -> Language {
    Language::ALL[CURRENT.load(Ordering::Relaxed) as usize % Language::ALL.len()]
}

type Table = HashMap<String, &'static str>;

fn table(language: Language) -> &'static Table {
    static TABLES: OnceLock<Vec<Table>> = OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        Language::ALL
            .iter()
            .map(|l| {
                serde_json::from_str::<HashMap<String, String>>(l.source())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(en, said)| (en, &*Box::leak(said.into_boxed_str())))
                    .collect()
            })
            .collect()
    });
    let i = Language::ALL
        .iter()
        .position(|l| *l == language)
        .unwrap_or(0);
    &tables[i]
}

/// `en` in `language`, or `en` itself where the table lacks it.
pub fn tr_in(language: Language, en: &str) -> &str {
    tr_in_brand(language, en, crate::brand::yoake())
}

fn tr_in_brand(language: Language, en: &str, yoake: bool) -> &str {
    let translated = table(language).get(en).copied().unwrap_or(en);
    let translated = if yoake
        && language != Language::Japanese
        && matches!(crate::brand::translate(en, en, true), std::borrow::Cow::Owned(_))
    {
        en
    } else {
        translated
    };
    match crate::brand::translate(en, translated, yoake) {
        std::borrow::Cow::Borrowed(value) => value,
        std::borrow::Cow::Owned(value) => {
            static BRANDED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
            let mut strings = BRANDED
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *strings
                .entry(value.clone())
                .or_insert_with(|| Box::leak(value.into_boxed_str()))
        }
    }
}

/// `en` in the launcher's language.
pub fn tr(en: &str) -> &str {
    tr_in(current(), en)
}

/// `en` in the launcher's language with its `{name}` holes filled.
pub fn trf(en: &str, args: &[(&str, &str)]) -> String {
    let mut out = tr(en).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
#[path = "i18n/tests.rs"]
mod tests;
