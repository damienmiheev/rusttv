//! Whether a person can be typing: the terminal rule (SECURITY.md § Change
//! lifecycle, step 4).

use std::io::{self, IsTerminal};

/// In debug builds with the `test-util` feature (only the CLI's
/// integration tests), `1` in this variable makes [`stdin_is_terminal`]
/// answer yes, so the tests can type confirmations through a pipe. Release
/// builds have no such code (as with `TEST_SIGNING_KEY_ENV`).
#[cfg(all(feature = "test-util", debug_assertions))]
pub const TEST_TERMINAL_ENV: &str = "RUSTTV_TEST_TERMINAL";

/// Whether stdin is an interactive terminal: the only place a person types
/// a plan's confirmation (there is no `--yes`).
pub fn stdin_is_terminal() -> bool {
    #[cfg(all(feature = "test-util", debug_assertions))]
    if std::env::var_os(TEST_TERMINAL_ENV).is_some_and(|value| value == "1") {
        return true;
    }
    io::stdin().is_terminal()
}
