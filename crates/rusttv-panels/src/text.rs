//! Text from packages: cleaning, length limits, languages (PANEL_STUDIO.md § 5.1, § 5.6).

use crate::error::Code;
use serde::{Deserialize, Deserializer};
use std::fmt;

/// Most combining marks kept in a row ("Zalgo" text stacks dozens).
const MAX_COMBINING_RUN: usize = 4;

const LRI: char = '\u{2066}';
const RLI: char = '\u{2067}';
const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';
const ZWNJ: char = '\u{200C}';
const ZWJ: char = '\u{200D}';

/// Cleans text from a package before RustTV shows it:
///
/// - tabs and line breaks become spaces; other control characters go;
/// - bidi embeddings and overrides (LRE, RLE, PDF, LRO, RLO) go; bidi
///   isolates (LRI, RLI, FSI, PDI) stay only in matched pairs; the marks
///   (LRM, RLM, ALM) stay, since Arabic and Hebrew mixed with Latin names and
///   numbers need them;
/// - invisible characters go (zero-width space, word joiner, soft hyphen,
///   fillers, tags, most variation selectors, …); the zero-width non-joiner
///   and joiner stay only between two letters or emoji, where Persian, Indic
///   scripts and emoji sequences need them;
/// - runs of combining marks are capped at four;
/// - the ends are trimmed.
pub fn clean_text(text: &str) -> String {
    let mut out = clean_keeping_ends(text);
    let end = out.trim_end().len();
    out.truncate(end);
    let start = out.len() - out.trim_start().len();
    out.drain(..start);
    out
}

/// [`clean_text`] without trimming the ends, for pieces that are joined
/// with others (an expression's `" · "`).
pub(crate) fn clean_keeping_ends(text: &str) -> String {
    let normalised: Vec<char> = text
        .chars()
        .filter_map(|c| match c {
            '\t' | '\n' | '\r' => Some(' '),
            c if removed(c) => None,
            c => Some(c),
        })
        .collect();
    let paired = pair_isolates(&normalised);
    let mut capped = Vec::with_capacity(paired.len());
    let mut combining = 0;
    for c in paired {
        if is_combining(c) {
            combining += 1;
            if combining > MAX_COMBINING_RUN {
                continue;
            }
        } else if !is_format(c) {
            combining = 0;
        }
        capped.push(c);
    }
    // Joiners last, on what is actually kept, so none is left without a
    // letter on both sides (and cleaning twice changes nothing).
    let mut out = String::with_capacity(text.len());
    for (i, &c) in capped.iter().enumerate() {
        let between_letters =
            i > 0 && joins(capped[i - 1]) && capped.get(i + 1).is_some_and(|&next| joins(next));
        if !matches!(c, ZWNJ | ZWJ) || between_letters {
            out.push(c);
        }
    }
    out
}

/// Characters that never reach the screen.
pub(crate) fn removed(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00AD}'                  // soft hyphen
            | '\u{034F}'                // combining grapheme joiner
            | '\u{115F}' | '\u{1160}'   // Hangul fillers
            | '\u{17B4}' | '\u{17B5}'   // Khmer inherent vowels
            | '\u{180B}'..='\u{180F}'   // Mongolian variation selectors and vowel separator
            | '\u{200B}'                // zero-width space
            | '\u{202A}'..='\u{202E}'   // LRE RLE PDF LRO RLO
            | '\u{2028}' | '\u{2029}'   // line and paragraph separators
            | '\u{2060}'..='\u{2065}'   // word joiner, invisible operators
            | '\u{206A}'..='\u{206F}'   // deprecated format characters
            | '\u{3164}' | '\u{FFA0}'   // Hangul fillers
            | '\u{FE00}'..='\u{FE0D}'   // variation selectors (FE0E/FE0F text/emoji style stay)
            | '\u{FEFF}'                // byte order mark
            | '\u{FFF0}'..='\u{FFFB}'   // specials, interlinear annotation
            | '\u{1BCA0}'..='\u{1BCA3}' // shorthand format controls
            | '\u{1D173}'..='\u{1D17A}' // musical format controls
            | '\u{E0000}'..='\u{E0FFF}' // tags, variation selectors supplement
        )
}

/// Kept characters that draw nothing: they don't end a run of combining
/// marks and don't count as visible.
pub(crate) fn is_format(c: char) -> bool {
    matches!(
        c,
        ZWNJ | ZWJ
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}'
            | LRI
            | RLI
            | FSI
            | PDI
            | '\u{FE0E}'
            | '\u{FE0F}'
    )
}

/// Letters and emoji: what a joiner may sit between. The Brahmic and
/// Southeast Asian blocks count whole, since their viramas and vowel signs
/// (which joiners follow) aren't alphabetic to Rust.
fn joins(c: char) -> bool {
    c.is_alphabetic()
        || is_combining(c)
        || matches!(c,
            '\u{0900}'..='\u{0DFF}'     // Devanagari to Sinhala (viramas, vowel signs)
            | '\u{0E00}'..='\u{0EFF}'   // Thai, Lao
            | '\u{1000}'..='\u{109F}'   // Myanmar
            | '\u{1780}'..='\u{17FF}'   // Khmer
            | '\u{2600}'..='\u{27BF}'   // symbols and dingbats used as emoji
            | '\u{1F000}'..='\u{1FAFF}' // emoji
            | '\u{FE0F}')
}

/// Combining marks that can be stacked: the generic blocks, Hebrew points,
/// Arabic harakat and Thai vowel and tone marks.
fn is_combining(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{0591}'..='\u{05C7}'
        | '\u{0610}'..='\u{061A}'
        | '\u{064B}'..='\u{065F}'
        | '\u{0670}'
        | '\u{06D6}'..='\u{06ED}'
        | '\u{0E31}'
        | '\u{0E34}'..='\u{0E3A}'
        | '\u{0E47}'..='\u{0E4E}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}'
    )
}

/// Drops PDIs that close nothing and isolate initiators left open, so the
/// text can't change the direction of whatever RustTV draws after it.
fn pair_isolates(chars: &[char]) -> Vec<char> {
    let mut keep = vec![true; chars.len()];
    let mut open = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            LRI | RLI | FSI => open.push(i),
            // The guard pops either way; only an unmatched PDI is dropped.
            PDI if open.pop().is_none() => keep[i] = false,
            _ => {}
        }
    }
    for i in open {
        keep[i] = false;
    }
    chars
        .iter()
        .zip(keep)
        .filter_map(|(&c, k)| k.then_some(c))
        .collect()
}

/// Cleaned text of 1 to `MAX` characters with at least one visible one. The
/// limit is part of the type, so each field's limit is checked as it is read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Text<const MAX: usize>(String);

impl<const MAX: usize> Text<MAX> {
    /// Cleans and checks `raw` (see [`clean_text`]).
    ///
    /// # Errors
    ///
    /// [`Code::EmptyText`] if nothing visible is left, [`Code::TextTooLong`]
    /// if more than `MAX` characters are.
    pub fn new(raw: &str) -> Result<Self, Code> {
        // Cleaning only removes characters, and none is wider than 4 bytes;
        // 16 bytes a character leaves room for the removed ones without
        // cleaning megabytes for a 500-character field.
        if raw.len() > MAX.saturating_mul(16) {
            return Err(Code::TextTooLong);
        }
        let text = clean_text(raw);
        if !text
            .chars()
            .any(|c| !c.is_whitespace() && !is_format(c) && c != '\u{2800}')
        {
            Err(Code::EmptyText)
        } else if text.chars().count() > MAX {
            Err(Code::TextTooLong)
        } else {
            Ok(Self(text))
        }
    }

    /// The cleaned text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const MAX: usize> fmt::Display for Text<MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de, const MAX: usize> Deserialize<'de> for Text<MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).map_err(|code| serde::de::Error::custom(code.as_str()))
    }
}

/// The languages RustTV is translated into: its `.po` folders
/// (`crates/rusttv-launcher/translations/<code>`, so `pt_BR`, not `pt-BR`)
/// plus English.
pub const LANGUAGES: [&str; 21] = [
    "ar", "de", "en", "es", "fr", "he", "hi", "id", "it", "ja", "ko", "nl", "pl", "pt_BR", "ru",
    "th", "tr", "uk", "vi", "zh_CN", "zh_TW",
];

/// One of [`LANGUAGES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Language(&'static str);

impl Language {
    /// The language for `code`.
    ///
    /// # Errors
    ///
    /// [`Code::BadLanguage`] if RustTV doesn't have it.
    pub fn parse(code: &str) -> Result<Self, Code> {
        LANGUAGES
            .iter()
            .find(|known| **known == code)
            .map(|known| Self(known))
            .ok_or(Code::BadLanguage)
    }

    /// English, the last fallback.
    pub const fn english() -> Self {
        Self("en")
    }

    /// The code, e.g. `pt_BR`.
    pub const fn code(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(|code| serde::de::Error::custom(code.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Every character `clean_text` must never return, listed by hand (not
    /// through `removed`, so the test checks the rule, not itself).
    const FORBIDDEN: &[char] = &[
        '\u{0}',
        '\u{7}',
        '\u{1B}',
        '\u{7F}',
        '\u{85}',
        '\u{AD}',
        '\u{34F}',
        '\u{115F}',
        '\u{17B4}',
        '\u{180E}',
        '\u{200B}',
        '\u{202A}',
        '\u{202B}',
        '\u{202C}',
        '\u{202D}',
        '\u{202E}',
        '\u{2028}',
        '\u{2029}',
        '\u{2060}',
        '\u{2064}',
        '\u{206F}',
        '\u{3164}',
        '\u{FE00}',
        '\u{FEFF}',
        '\u{FFA0}',
        '\u{FFFB}',
        '\u{1D173}',
        '\u{E0041}',
        '\u{E0100}',
    ];

    #[test]
    fn controls_overrides_and_invisibles_are_removed() {
        let dirty: String = FORBIDDEN.iter().flat_map(|&c| [c, 'x']).collect();
        assert_eq!(clean_text(&dirty), "x".repeat(FORBIDDEN.len()));
    }

    #[test]
    fn line_breaks_and_tabs_become_spaces() {
        assert_eq!(clean_text("a\nb\tc\r\nd"), "a b c  d");
    }

    #[test]
    fn bidi_marks_paired_isolates_and_joiners_between_letters_are_kept() {
        let text =
            "\u{2067}مرحبا\u{2069} \u{200F}RustTV\u{200E} می\u{200C}خواهم क्\u{200D}ष 👨\u{200D}👩";
        assert_eq!(clean_text(text), text);
    }

    #[test]
    fn unmatched_isolates_and_stray_joiners_are_dropped() {
        assert_eq!(clean_text("\u{2067}abc"), "abc");
        assert_eq!(clean_text("abc\u{2069}def"), "abcdef");
        assert_eq!(clean_text("\u{2066}a\u{2069}\u{2069}"), "\u{2066}a\u{2069}");
        assert_eq!(clean_text("\u{200D}a b\u{200C} \u{200D}"), "a b");
    }

    #[test]
    fn stacked_marks_are_capped_even_with_joiners_or_marks_between_them() {
        let zalgo = format!("e{}", "\u{0301}".repeat(30));
        assert_eq!(clean_text(&zalgo).chars().count(), 1 + MAX_COMBINING_RUN);
        let sneaky = format!("e{}", "\u{0301}\u{200D}\u{200E}".repeat(10));
        assert_eq!(
            clean_text(&sneaky)
                .chars()
                .filter(|&c| c == '\u{0301}')
                .count(),
            MAX_COMBINING_RUN
        );
        let hebrew = format!("ש{}", "\u{05B8}".repeat(10));
        assert_eq!(clean_text(&hebrew).chars().count(), 1 + MAX_COMBINING_RUN);
    }

    #[test]
    fn a_joiner_before_a_capped_mark_goes_too() {
        let text = format!("a{}\u{200D}\u{0301}", "\u{0301}".repeat(4));
        let once = clean_text(&text);
        assert_eq!(once, format!("a{}", "\u{0301}".repeat(4)));
        assert_eq!(clean_text(&once), once);
    }

    #[test]
    fn devanagari_and_arabic_text_pass_unchanged() {
        assert_eq!(clean_text("नमस्ते दुनिया"), "नमस्ते दुनिया");
        assert_eq!(clean_text("مَرْحَبًا"), "مَرْحَبًا");
    }

    #[test]
    fn text_needs_something_visible_and_counts_characters_after_cleaning() {
        assert_eq!(
            Text::<3>::new("  a\u{200B}bc ").map(|t| t.0),
            Ok("abc".into())
        );
        assert_eq!(Text::<3>::new("abcd"), Err(Code::TextTooLong));
        // A huge field is refused before cleaning; removed characters still fit.
        assert_eq!(Text::<3>::new(&"a".repeat(49)), Err(Code::TextTooLong));
        assert_eq!(
            Text::<3>::new(&format!("ab{}c", "\u{200b}".repeat(10))).map(|t| t.as_str().len()),
            Ok(3)
        );
        assert!(Text::<4>::new("київ").is_ok());
        for blank in [
            " \u{202E} ",
            "\u{200D}",
            "\u{200E}\u{200E}",
            "\u{3164}",
            "\u{2800}",
            "\u{FE0F}",
            "\u{2066}\u{2069}",
        ] {
            assert_eq!(Text::<10>::new(blank), Err(Code::EmptyText), "{blank:?}");
        }
    }

    #[test]
    fn languages_match_the_launchers_translation_folders() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rusttv-launcher/translations");
        let mut folders: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_type().unwrap().is_dir())
            .map(|entry| entry.file_name().into_string().unwrap())
            .collect();
        folders.push("en".into());
        folders.sort();
        assert_eq!(folders, LANGUAGES);
        assert_eq!(Language::parse("pt_BR").map(Language::code), Ok("pt_BR"));
        assert_eq!(Language::parse("pt-BR"), Err(Code::BadLanguage));
    }

    proptest! {
        #[test]
        fn cleaning_is_idempotent_and_never_keeps_a_forbidden_char(s in "\\PC*|[\\x00-\\x1f\u{200B}-\u{200F}\u{202A}-\u{202E}\u{2066}-\u{2069}\u{E0000}-\u{E007F}\u{0301} a]*") {
            let once = clean_text(&s);
            prop_assert_eq!(clean_text(&once), once.as_str());
            prop_assert!(!once.chars().any(|c| FORBIDDEN.contains(&c)));
        }
    }
}
