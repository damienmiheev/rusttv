//! Plural categories (CLDR cardinal rules) for RustTV's languages, for whole
//! numbers. Counts in panels are whole (items, unread messages); fractions
//! get the language's `other` form until expressions bring decimals.

use crate::Language;
use serde::Deserialize;

/// A CLDR plural category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluralCategory {
    /// Zero (Arabic).
    Zero,
    /// One.
    One,
    /// Two (Arabic, Hebrew).
    Two,
    /// A few.
    Few,
    /// Many.
    Many,
    /// Everything else; every plural message has it.
    Other,
}

use PluralCategory::{Few, Many, One, Other, Two, Zero};

/// The categories `language` uses.
pub fn plural_categories(language: Language) -> &'static [PluralCategory] {
    match language.code() {
        "ar" => &[Zero, One, Two, Few, Many, Other],
        "he" => &[One, Two, Other],
        "pl" | "ru" | "uk" => &[One, Few, Many, Other],
        "es" | "fr" | "it" | "pt_BR" => &[One, Many, Other],
        "id" | "ja" | "ko" | "th" | "vi" | "zh_CN" | "zh_TW" => &[Other],
        // de, en, hi, nl, tr
        _ => &[One, Other],
    }
}

/// The category of the whole number `n` in `language`.
pub fn plural_category(language: Language, n: u64) -> PluralCategory {
    let (n10, n100) = (n % 10, n % 100);
    let millions = n != 0 && n.is_multiple_of(1_000_000);
    match language.code() {
        "ar" => match (n, n100) {
            (0, _) => Zero,
            (1, _) => One,
            (2, _) => Two,
            (_, 3..=10) => Few,
            (_, 11..=99) => Many,
            _ => Other,
        },
        "he" => match n {
            1 => One,
            2 => Two,
            _ => Other,
        },
        "pl" => match (n, n10, n100) {
            (1, _, _) => One,
            (_, 2..=4, _) if !(12..=14).contains(&n100) => Few,
            _ => Many,
        },
        "ru" | "uk" => match (n10, n100) {
            (1, _) if n100 != 11 => One,
            (2..=4, _) if !(12..=14).contains(&n100) => Few,
            _ => Many,
        },
        "fr" | "pt_BR" => match n {
            0 | 1 => One,
            _ if millions => Many,
            _ => Other,
        },
        "es" | "it" => match n {
            1 => One,
            _ if millions => Many,
            _ => Other,
        },
        "hi" => match n {
            0 | 1 => One,
            _ => Other,
        },
        "id" | "ja" | "ko" | "th" | "vi" | "zh_CN" | "zh_TW" => Other,
        // de, en, nl, tr
        _ => match n {
            1 => One,
            _ => Other,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LANGUAGES;

    fn language(code: &str) -> Language {
        Language::parse(code).unwrap()
    }

    #[test]
    fn every_category_a_number_gets_is_one_its_language_has() {
        let numbers = (0..2000).chain([1_000_000, 2_000_000, 1_000_001, u64::MAX]);
        for code in LANGUAGES {
            let language = language(code);
            let has = plural_categories(language);
            assert!(has.contains(&Other), "{code}");
            for n in numbers.clone() {
                assert!(has.contains(&plural_category(language, n)), "{code} {n}");
            }
        }
    }

    #[test]
    fn known_numbers_get_their_cldr_category() {
        let cases = [
            ("en", 1, One),
            ("en", 0, Other),
            ("en", 2, Other),
            ("fr", 0, One),
            ("fr", 2, Other),
            ("fr", 1_000_000, Many),
            ("es", 0, Other),
            ("es", 2_000_000, Many),
            ("uk", 1, One),
            ("uk", 21, One),
            ("uk", 11, Many),
            ("uk", 3, Few),
            ("uk", 13, Many),
            ("uk", 24, Few),
            ("uk", 5, Many),
            ("ru", 111, Many),
            ("pl", 1, One),
            ("pl", 21, Many),
            ("pl", 22, Few),
            ("pl", 12, Many),
            ("ar", 0, Zero),
            ("ar", 2, Two),
            ("ar", 103, Few),
            ("ar", 111, Many),
            ("ar", 100, Other),
            ("he", 2, Two),
            ("he", 20, Other),
            ("hi", 0, One),
            ("ja", 1, Other),
            ("fr", 1, One),
            ("pt_BR", 0, One),
            ("pt_BR", 3_000_000, Many),
            ("it", 0, Other),
            ("it", 1_000_000, Many),
            ("ru", 0, Many),
            ("ru", 12, Many),
            ("ru", 14, Many),
            ("ru", 21, One),
            ("pl", 0, Many),
            ("pl", 5, Many),
            ("pl", 112, Many),
            ("ar", 3, Few),
            ("ar", 10, Few),
            ("ar", 11, Many),
            ("ar", 99, Many),
            ("ar", 101, Other),
            ("ar", 102, Other),
            ("he", 1, One),
        ];
        for (code, n, category) in cases {
            assert_eq!(plural_category(language(code), n), category, "{code} {n}");
        }
    }
}
