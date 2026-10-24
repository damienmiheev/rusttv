//! Which bundled translation fits the device's language.
//!
//! Translations live in `translations/<lang>/LC_MESSAGES/rusttv-launcher.po`
//! and are compiled into the library (build.rs). English is the source
//! language and needs no file.

/// The bundled translations, by folder name. Keep in sync with `translations/`.
pub const LANGUAGES: &[&str] = &[
    "ar", "de", "es", "fr", "he", "hi", "id", "it", "ja", "ko", "nl", "pl", "pt_BR", "ru", "th",
    "tr", "uk", "vi", "zh_CN", "zh_TW",
];

/// The bundled translation for a BCP 47 language tag such as `uk-UA`,
/// `zh-Hant-TW` or `pt-PT` (case and `_` vs `-` don't matter), or `None` for
/// English and languages without a translation.
///
/// Chinese picks Traditional for the `Hant` script or Taiwan, Hong Kong and
/// Macau, Simplified otherwise; every Portuguese gets Brazilian Portuguese,
/// the only one bundled.
pub fn bundled_language(tag: &str) -> Option<&'static str> {
    let lower = tag.to_ascii_lowercase();
    let mut parts = lower.split(['-', '_']);
    let language = match parts.next()? {
        // Old ISO codes Java may still report.
        "iw" => "he",
        "in" => "id",
        other => other,
    };
    let rest: Vec<&str> = parts.collect();
    let wanted = match language {
        "zh" if rest
            .iter()
            .any(|part| matches!(*part, "hant" | "tw" | "hk" | "mo")) =>
        {
            "zh_TW"
        }
        "zh" => "zh_CN",
        "pt" => "pt_BR",
        other => other,
    };
    LANGUAGES.iter().copied().find(|bundled| *bundled == wanted)
}

/// Whether a language tag's script runs right to left (Arabic, Hebrew, Persian, Urdu).
pub fn is_right_to_left(tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    let language = lower.split(['-', '_']).next().unwrap_or_default();
    matches!(language, "ar" | "he" | "iw" | "fa" | "ur")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_bundled_translation_for_a_device_language() {
        assert_eq!(bundled_language("uk-UA"), Some("uk"));
        assert_eq!(bundled_language("de"), Some("de"));
        assert_eq!(bundled_language("PT-br"), Some("pt_BR"));
        assert_eq!(bundled_language("pt-PT"), Some("pt_BR"));
        assert_eq!(bundled_language("zh-CN"), Some("zh_CN"));
        assert_eq!(bundled_language("zh-Hans-CN"), Some("zh_CN"));
        assert_eq!(bundled_language("zh-Hant-TW"), Some("zh_TW"));
        assert_eq!(bundled_language("zh_HK"), Some("zh_TW"));
        assert_eq!(bundled_language("iw-IL"), Some("he"));
        assert_eq!(bundled_language("in-ID"), Some("id"));
    }

    #[test]
    fn english_and_missing_languages_get_none() {
        assert_eq!(bundled_language("en-US"), None);
        assert_eq!(bundled_language("sv-SE"), None);
        assert_eq!(bundled_language(""), None);
    }

    #[test]
    fn right_to_left_follows_the_script() {
        assert!(is_right_to_left("ar-EG"));
        assert!(is_right_to_left("he"));
        assert!(is_right_to_left("iw-IL"));
        assert!(!is_right_to_left("uk-UA"));
        assert!(!is_right_to_left("en"));
    }

    #[test]
    fn every_bundled_language_has_a_translation_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("translations");
        for language in LANGUAGES {
            let file = root
                .join(language)
                .join("LC_MESSAGES")
                .join("rusttv-launcher.po");
            assert!(file.is_file(), "missing {}", file.display());
        }
    }
}
