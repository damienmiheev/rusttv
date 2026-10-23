//! Records the target triple, which picks the release archive
//! `rusttv update cli` downloads (`rusttv-<version>-<target>.tar.gz`).

#[expect(
    clippy::expect_used,
    reason = "Cargo always sets TARGET for build scripts; a build script reports errors by panicking"
)]
fn main() {
    let target = std::env::var("TARGET").expect("Cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=RUSTTV_TARGET={target}");
}
