//! Compiles the Slint UI into Rust code.

#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking; Slint prints the diagnostics"
)]
fn main() {
    slint_build::compile("ui/home.slint").expect("ui/home.slint should compile");
}
