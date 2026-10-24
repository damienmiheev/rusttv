//! Compiles the Slint UI into Rust code, with its translations bundled in.

use slint_build::{CompilerConfiguration, DefaultTranslationContext};

#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking; Slint prints the diagnostics"
)]
fn main() {
    // `translations/<lang>/LC_MESSAGES/rusttv-launcher.po`, compiled into the
    // library, so the APK ships no separate files. No default context: each
    // text is translated once, whichever component shows it.
    let config = CompilerConfiguration::new()
        .with_bundled_translations("translations")
        .with_default_translation_context(DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/home.slint", config)
        .expect("ui/home.slint should compile");
}
