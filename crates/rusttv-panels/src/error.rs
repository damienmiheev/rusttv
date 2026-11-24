//! Errors: a code and a position, never the package's own text.
//!
//! Errors reach the CLI, the MCP and History (PANEL_STUDIO.md § 5.1), so a
//! package must not be able to put words in them: no field names, keys or
//! values from the file, only our codes and where they happened.

use std::fmt;

/// Declares [`Code`], its spellings and the list of all codes from one table,
/// so a new code can't be missing from any of them.
macro_rules! codes {
    ($($(#[$doc:meta])* $variant:ident => $spelling:literal,)*) => {
        /// What is wrong. [`Code::as_str`] is the stable spelling used in output.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum Code {
            $($(#[$doc])* $variant,)*
        }

        impl Code {
            /// Every code, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The stable spelling, e.g. `PACKAGE_TOO_NEW`.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)*
                }
            }
        }
    };
}

codes! {
    /// `package.json` is larger than [`crate::MAX_PACKAGE_JSON`].
    TooLarge => "TOO_LARGE",
    /// Not valid JSON (or not UTF-8), including numbers out of range.
    NotJson => "NOT_JSON",
    /// An object has the same key twice.
    DuplicateKey => "DUPLICATE_KEY",
    /// Objects and arrays nest deeper than [`crate::MAX_JSON_DEPTH`].
    TooDeep => "TOO_DEEP",
    /// A format older than this launcher reads (format 1 is build-time only).
    FormatUnsupported => "FORMAT_UNSUPPORTED",
    /// The package needs a newer launcher (a newer format or `min_launcher`).
    PackageTooNew => "PACKAGE_TOO_NEW",
    /// The package needs a renderer feature this launcher doesn't have.
    RequiresFeature => "REQUIRES_FEATURE",
    /// A field this format doesn't know.
    UnknownField => "UNKNOWN_FIELD",
    /// A required field is missing.
    MissingField => "MISSING_FIELD",
    /// A value has the wrong JSON type.
    WrongType => "WRONG_TYPE",
    /// A value that isn't allowed for its field.
    BadValue => "BAD_VALUE",
    /// A package id that isn't `user.` + a name (see [`crate::Name`]).
    BadId => "BAD_ID",
    /// A package id that is reserved for RustTV.
    ReservedId => "RESERVED_ID",
    /// A version that isn't `<major>.<minor>.<patch>`.
    BadVersion => "BAD_VERSION",
    /// A language code RustTV doesn't know.
    BadLanguage => "BAD_LANGUAGE",
    /// A text with nothing visible after cleaning.
    EmptyText => "EMPTY_TEXT",
    /// A text longer than its field allows after cleaning.
    TextTooLong => "TEXT_TOO_LONG",
    /// A link that isn't a plain `https://` URL.
    BadUrl => "BAD_URL",
    /// A feature, source or action name that isn't `[a-z0-9-]{1,32}`.
    BadName => "BAD_NAME",
    /// The same entry listed twice.
    Duplicate => "DUPLICATE",
    /// A list longer than its field allows.
    TooMany => "TOO_MANY",
    /// The name has no text in the default language.
    NoDefaultName => "NO_DEFAULT_NAME",
}

impl Code {
    /// The code spelled by [`Code::as_str`], if it is one. Field validators
    /// report their code through serde's custom error text; this reads it back.
    pub(crate) fn from_spelling(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|code| code.as_str() == text)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for Code {}

/// Where in `package.json` the problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Position {
    /// The whole file.
    File,
    /// A line and column (both from 1), as the JSON reader reports them.
    Line {
        /// Line, from 1.
        line: usize,
        /// Column, from 1.
        column: usize,
    },
    /// A field path built only from this format's own field names, e.g.
    /// `meta.requires`, and an index into a list; never a key from the package.
    Field {
        /// The path, e.g. `meta.requires`.
        path: &'static str,
        /// The index in that list, if the field is a list.
        index: Option<usize>,
    },
}

impl Position {
    pub(crate) const fn field(path: &'static str) -> Self {
        Self::Field { path, index: None }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File => f.write_str("file"),
            Self::Line { line, column } => write!(f, "line {line} column {column}"),
            Self::Field { path, index: None } => f.write_str(path),
            Self::Field {
                path,
                index: Some(index),
            } => write!(f, "{path}[{index}]"),
        }
    }
}

/// A refused package: what and where.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageError {
    /// What is wrong.
    pub code: Code,
    /// Where.
    pub at: Position,
    /// For [`Code::PackageTooNew`], the launcher version the package asks
    /// for, when it says so; a version is ours to print, not free text.
    pub needs: Option<crate::Version>,
}

impl PackageError {
    pub(crate) const fn new(code: Code, at: Position) -> Self {
        Self {
            code,
            at,
            needs: None,
        }
    }

    /// Turns a serde_json error from the raw pass into a code and a line,
    /// dropping its message (which can quote the package's keys and values).
    pub(crate) fn from_json(error: &serde_json::Error) -> Self {
        Self::from_serde(error, Code::NotJson)
    }

    /// The same for typed parsing, which runs only after the raw pass proved
    /// the file is JSON: serde_json reports some type mismatches (a number
    /// where an enum belongs) as syntax errors, so those are wrong types.
    pub(crate) fn from_typed_json(error: &serde_json::Error) -> Self {
        Self::from_serde(error, Code::WrongType)
    }

    fn from_serde(error: &serde_json::Error, syntax: Code) -> Self {
        use serde_json::error::Category;
        let at = Position::Line {
            line: error.line(),
            column: error.column(),
        };
        let code = match error.classify() {
            Category::Io | Category::Syntax | Category::Eof => syntax,
            Category::Data => data_code(&error.to_string()),
        };
        Self::new(code, at)
    }
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.code, self.at)?;
        if let Some(version) = self.needs {
            write!(f, " (needs RustTV {version})")?;
        }
        Ok(())
    }
}

impl std::error::Error for PackageError {}

/// Picks a code for a serde data error: our validators' own codes first, then
/// serde's fixed wordings, matched on their start only and never echoed. The
/// `field_checks_report_their_own_codes` test in `lib.rs` pins these wordings.
fn data_code(message: &str) -> Code {
    let head = message.split(" at line ").next().unwrap_or(message);
    if let Some(code) = Code::from_spelling(head) {
        code
    } else if head.starts_with("unknown field") {
        Code::UnknownField
    } else if head.starts_with("missing field") {
        Code::MissingField
    } else if head.starts_with("invalid type") || head.starts_with("invalid length") {
        Code::WrongType
    } else {
        Code::BadValue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_reads_back_from_its_spelling() {
        for &code in Code::ALL {
            assert_eq!(Code::from_spelling(code.as_str()), Some(code));
        }
        assert_eq!(Code::from_spelling("NOT_A_CODE"), None);
    }

    #[test]
    fn a_too_new_error_says_which_launcher_it_needs() {
        let mut error =
            PackageError::new(Code::PackageTooNew, Position::field("meta.min_launcher"));
        error.needs = Some(crate::Version::new(0, 9, 0));
        assert_eq!(
            error.to_string(),
            "PACKAGE_TOO_NEW at meta.min_launcher (needs RustTV 0.9.0)"
        );
    }
}
