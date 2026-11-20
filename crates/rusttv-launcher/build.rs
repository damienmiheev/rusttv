//! Compiles the Slint UI into Rust code, with its translations bundled in,
//! generates Slint from the panel templates (PANELS.md § 4.1), and decodes the
//! built-in tiles' banners and the templates' badges into raw pixels.

#[path = "codegen/convert.rs"]
mod convert;

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use slint_build::{CompilerConfiguration, DefaultTranslationContext};

/// The banners of the launcher's own tiles, rendered by scripts/brand/render.sh.
const TILES: [&str; 5] = ["about", "settings", "hidden", "remote", "history"];

/// Their size: an Android TV banner (xhdpi), like the apps' banners.
const TILE_SIZE: (u32, u32) = (320, 180);

/// The size of the badges (`convert::BADGES`).
const BADGE_SIZE: (u32, u32) = (128, 128);

#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking; Slint prints the diagnostics"
)]
fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"),
    );

    println!("cargo::rerun-if-changed=codegen/convert.rs");
    // Every panel template (templates/*.json), each turned into
    // `<name>.slint` in OUT_DIR: the same set scripts/launcher-pot.sh and
    // the translation test read.
    println!("cargo::rerun-if-changed=templates");
    convert_templates("templates", &out);
    write_badges(&out);
    write_pictures(&out);

    // `translations/<lang>/LC_MESSAGES/rusttv-launcher.po`, compiled into the
    // library, so the APK ships no separate files. No default context: each
    // text is translated once, whichever component (or template) shows it.
    // Generated files import "theme.slint" (found in ui/, the same file
    // home.slint imports, so there is one `Dir.rtl`) and "badges.slint" (in
    // OUT_DIR) by name.
    let config = CompilerConfiguration::new()
        .with_include_paths(vec![out.clone(), manifest.join("ui")])
        .with_bundled_translations("translations")
        .with_default_translation_context(DefaultTranslationContext::None)
        // Element names for the tests' search (i-slint-backend-testing), so
        // they read what a page draws: with debug assertions (the tests'
        // `cfg_attr`), never in a release build.
        .with_debug_info(std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some());
    write_test_pages(&out, &manifest);

    slint_build::compile_with_config("ui/home.slint", config)
        .expect("ui/home.slint should compile");

    for name in TILES {
        let source = format!("assets/tiles/{name}.png");
        println!("cargo::rerun-if-changed={source}");
        let rgba = decode(Path::new(&source), TILE_SIZE);
        std::fs::write(out.join(format!("{name}.rgba")), rgba).expect("OUT_DIR should be writable");
    }
    for name in convert::BADGES {
        let source = format!("assets/badges/{name}.png");
        println!("cargo::rerun-if-changed={source}");
        let rgba = decode(Path::new(&source), BADGE_SIZE);
        std::fs::write(out.join(format!("badge-{name}.rgba")), rgba)
            .expect("OUT_DIR should be writable");
    }
}

/// Test-only templates (codegen/test_templates/*.json) use the converter's
/// paths the real ones don't (one column, a header without a hint,
/// conditional blocks, rows in a column, a nested tile column, a row's
/// attention). They are compiled into `test_pages.rs`, which only the
/// tests include; on purpose in every build (a second or so), so no build
/// can carry a converter that writes Slint its compiler refuses.
/// `TestPages` shows the page or tile its `page` names (the file's name),
/// with `value` in every prop; a row's functions are its `<file>-title`,
/// `-detail` and `-attention` (with empty props).
#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking; Slint prints the diagnostics"
)]
fn write_test_pages(out: &Path, manifest: &Path) {
    println!("cargo::rerun-if-changed=codegen/test_templates");
    let test_out = out.join("test_templates");
    std::fs::create_dir_all(&test_out).expect("OUT_DIR should be writable");
    let mut imports = String::new();
    let mut body = String::new();
    for (file, template) in convert_templates("codegen/test_templates", &test_out) {
        // convert() accepted the file, so these fields are there.
        let component = template["component"].as_str().unwrap_or_default();
        let name = file.replace('_', "-");
        imports.push_str(&format!(
            "import {{ {component} }} from \"{file}.slint\";\n"
        ));
        if template["face"] == "row" {
            // A row is a global of functions: call them, so they compile.
            let props = template["props"].as_array().map_or(0, Vec::len);
            let args = vec!["\"\""; props].join(", ");
            body.push_str(&format!(
                "    out property <string> {name}-title: {component}.title({args});\n    \
                 out property <string> {name}-detail: {component}.detail({args});\n    \
                 out property <bool> {name}-attention: {component}.attention({args});\n"
            ));
        } else {
            // Every prop gets `value`, so a test can show conditional blocks.
            let props: String = template["props"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|prop| prop.as_str())
                .map(|prop| format!(" {prop}: root.value;"))
                .collect();
            body.push_str(&format!(
                "    if page == \"{name}\": {component} {{ u: 1px;{props} }}\n"
            ));
        }
    }
    let root_file = test_out.join("test_pages.slint");
    std::fs::write(
        &root_file,
        format!(
            "{imports}\nexport component TestPages inherits Window {{\n    \
             in property <string> page;\n    in property <string> value;\n{body}}}\n"
        ),
    )
    .expect("OUT_DIR should be writable");
    // No bundled translations: the tests read the English texts.
    let config = CompilerConfiguration::new()
        .with_include_paths(vec![test_out, out.to_path_buf(), manifest.join("ui")])
        .with_default_translation_context(DefaultTranslationContext::None)
        .with_debug_info(std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some());
    slint_build::compile_with_output_path(&root_file, out.join("test_pages.rs"), config)
        .expect("the test templates should compile");
}

/// Files build.rs generates in OUT_DIR, which generated Slint imports by
/// name: no template may take their names (nor a `ui/*.slint` file's).
const GENERATED_FILES: [&str; 3] = ["badges", "pictures", "test_pages"];

/// The names a template in any folder can't take: [`GENERATED_FILES`] and
/// every `ui/*.slint`, since the include paths find files by name.
#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking"
)]
fn taken_files() -> Vec<String> {
    let ui = std::fs::read_dir("ui")
        .expect("ui/ should exist")
        .map(|entry| entry.expect("ui/ should be readable").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "slint"))
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_owned()));
    GENERATED_FILES
        .map(str::to_owned)
        .into_iter()
        .chain(ui)
        .collect()
}

/// Turns every `<dir>/*.json` panel template into `<out>/<name>.slint`;
/// returns each file's name and template, in name order. Two templates in
/// one folder can't share a component, and none can take a name in
/// [`taken_files`]. (The test templates are compiled apart from the real
/// ones, so a name there only shadows within that compile.)
#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking"
)]
fn convert_templates(dir: &str, out: &Path) -> Vec<(String, serde_json::Value)> {
    let mut templates: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("the templates' folder should exist")
        .map(|entry| {
            entry
                .expect("the templates' folder should be readable")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    templates.sort();
    let taken = taken_files();
    let mut converted = Vec::new();
    for path in templates {
        let source = path.display().to_string();
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("a template's name should be UTF-8");
        println!("cargo::rerun-if-changed={source}");
        assert!(
            !taken.iter().any(|taken| taken == name),
            "{source}: a template can't be named {name}"
        );
        let json = std::fs::read_to_string(&path).expect("panel template should exist");
        let slint = convert::convert(&json).unwrap_or_else(|error| panic!("{source}: {error}"));
        std::fs::write(out.join(format!("{name}.slint")), slint)
            .expect("OUT_DIR should be writable");
        // Read again (cheaply) for its component and face; convert()
        // accepted it, so it is JSON.
        let template: serde_json::Value =
            serde_json::from_str(&json).expect("a converted template is JSON");
        let component = &template["component"];
        assert!(
            converted
                .iter()
                .all(|(_, other): &(String, serde_json::Value)| other["component"] != *component),
            "{source}: another template is already {component}"
        );
        converted.push((name.to_owned(), template));
    }
    converted
}

/// Writes the `Pictures` global (`pictures.slint`) and its Rust setter
/// (`pictures.rs`, included by ui.rs) from [`convert::PICTURES`]: pictures
/// Home provides for pages' `picture` blocks, so the list lives in one
/// place.
#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking"
)]
fn write_pictures(out: &Path) {
    let mut slint = String::from(
        "// Generated by build.rs from convert.rs PICTURES: pictures Home provides\n\
         // for panel pages, set from Rust (ui.rs).\nexport global Pictures {\n",
    );
    let mut rust = String::from(
        "/// Generated by build.rs from `PICTURES`: sets every picture panel pages may\n\
         /// show, each from `picture(name)`.\n\
         fn set_pictures(window: &HomeWindow, picture: impl Fn(&str) -> slint::Image) {\n    \
         let pictures = window.global::<Pictures>();\n",
    );
    for name in convert::PICTURES {
        slint.push_str(&format!("    in property <image> {name};\n"));
        rust.push_str(&format!(
            "    pictures.set_{}(picture(\"{name}\"));\n",
            name.replace('-', "_")
        ));
    }
    slint.push_str("}\n");
    rust.push_str("}\n");
    std::fs::write(out.join("pictures.slint"), slint).expect("OUT_DIR should be writable");
    std::fs::write(out.join("pictures.rs"), rust).expect("OUT_DIR should be writable");
}

/// Writes the `Badges` global (`badges.slint`) and its Rust setter
/// (`badges.rs`, included by ui.rs) from [`convert::BADGES`], so the list lives in
/// one place.
#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking"
)]
fn write_badges(out: &Path) {
    let mut slint = String::from(
        "// Generated by build.rs from BADGES: pictures panel templates may show,\n\
         // set from Rust (ui.rs), like the tiles' banners.\nexport global Badges {\n",
    );
    let mut rust = String::from(
        "/// Generated by build.rs from `BADGES`: sets every picture panel templates may show.\n\
         fn set_badges(window: &HomeWindow) {\n    let badges = window.global::<Badges>();\n",
    );
    for name in convert::BADGES {
        slint.push_str(&format!("    in property <image> {name};\n"));
        rust.push_str(&format!(
            "    badges.set_{setter}(slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice(\n        \
             include_bytes!(concat!(env!(\"OUT_DIR\"), \"/badge-{name}.rgba\")),\n        {w},\n        {h},\n    )));\n",
            setter = name.replace('-', "_"),
            w = BADGE_SIZE.0,
            h = BADGE_SIZE.1,
        ));
    }
    slint.push_str("}\n");
    rust.push_str("}\n");
    std::fs::write(out.join("badges.slint"), slint).expect("OUT_DIR should be writable");
    std::fs::write(out.join("badges.rs"), rust).expect("OUT_DIR should be writable");
}

/// Decodes a PNG of `expected` size into RGBA bytes.
#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking"
)]
fn decode(path: &Path, expected: (u32, u32)) -> Vec<u8> {
    let file = File::open(path).expect("picture should exist (scripts/brand/render.sh)");
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().expect("picture should be a PNG");
    let size = reader
        .output_buffer_size()
        .expect("picture should fit in memory");
    let mut pixels = vec![0; size];
    let info = reader
        .next_frame(&mut pixels)
        .expect("picture should decode");
    assert_eq!(
        (info.width, info.height),
        expected,
        "{} should be {}×{}",
        path.display(),
        expected.0,
        expected.1
    );
    pixels.truncate(info.buffer_size());
    match info.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[red, green, blue]| [red, green, blue, 255])
            .collect(),
        other => panic!("{}: unexpected colour type {other:?}", path.display()),
    }
}
