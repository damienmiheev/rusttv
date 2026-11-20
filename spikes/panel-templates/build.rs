//! Generates `about_tile.slint` from templates/about_tile.json, then compiles
//! the spike's UI with it and the spike's translations.

#[path = "convert.rs"]
mod convert;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it"));
    let logo = manifest.join("assets/rusttv-logo.png");
    let logo = logo.to_str().expect("UTF-8 path");
    let source = manifest.join("templates/about_tile.json");
    println!("cargo::rerun-if-changed={}", source.display());
    println!("cargo::rerun-if-changed=convert.rs");

    let json = std::fs::read_to_string(&source).expect("template exists");
    let theme = manifest.join("ui/theme.slint");
    let theme = theme.to_str().expect("UTF-8 path");
    let slint = match convert::convert(&json, theme, &[("rusttv-logo", logo)]) {
        Ok(slint) => slint,
        Err(error) => panic!("{}: {error}", source.display()),
    };
    std::fs::write(out.join("about_tile.slint"), slint).expect("OUT_DIR is writable");

    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![out.clone()])
        .with_bundled_translations(manifest.join("translations"))
        // As the launcher: no context, so one msgid is one translation
        // whichever component (hand-written or generated) shows it.
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/spike.slint", config).expect("ui compiles");
}
