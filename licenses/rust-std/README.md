# The Rust standard library

Every Rust binary statically links the standard library (`core`, `alloc`,
`std`, `compiler_builtins`, `std_detect`, `panic_abort`/`panic_unwind`,
`unwind`) and the crates std itself depends on. These are not cargo
dependencies of RustTV, so `cargo about` does not list them; their notices are
kept here and appended by `scripts/licenses.sh`.

| File | Covers | Licence |
|---|---|---|
| `COPYRIGHT`, `LICENSE-APACHE`, `LICENSE-MIT` | The Rust standard library (rust-lang/rust) | Apache-2.0 OR MIT |
| `Unicode-3.0.txt` | Unicode data tables in `core::unicode` | Unicode-3.0 |
| `compiler-builtins-LICENSE.txt` | `compiler_builtins` (derived from LLVM compiler-rt) | MIT AND Apache-2.0 WITH LLVM-exception |
| `libm-LICENSE.txt` | `libm`, bundled in `compiler_builtins` (derived from musl) | MIT |
| `stdarch-LICENSE-MIT.txt` | `std_detect` (stdarch) | Apache-2.0 OR MIT |
| `addr2line-LICENSE-MIT.txt` | addr2line 0.27.1 | Apache-2.0 OR MIT |
| `adler2-LICENSE-0BSD.txt` | adler2 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| `cfg-if-LICENSE-MIT.txt` | cfg-if 1.0.4 | MIT OR Apache-2.0 |
| `gimli-LICENSE-MIT.txt` | gimli 0.34.0 | MIT OR Apache-2.0 |
| `hashbrown-LICENSE-MIT.txt` | hashbrown 0.17.1 | MIT OR Apache-2.0 |
| `libc-LICENSE-MIT.txt` | libc 0.2.189 | MIT OR Apache-2.0 |
| `memchr-LICENSE-MIT.txt` | memchr 2.8.3 | Unlicense OR MIT |
| `miniz_oxide-LICENSE-MIT.md` | miniz_oxide 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| `object-LICENSE-MIT.txt` | object 0.39.1 | Apache-2.0 OR MIT |
| `rustc-demangle-LICENSE-MIT.txt` | rustc-demangle 0.1.28 | MIT OR Apache-2.0 |

Where several licences are offered, the MIT text (0BSD for adler2, whose MIT
file has no copyright line) is shipped; the Apache-2.0 text is in
`LICENSE-APACHE` anyway.

Source, for toolchain 1.99.1 (`rust-toolchain.toml`), all copied unmodified:

- `COPYRIGHT`, `LICENSE-*`, `library/compiler-builtins/LICENSE.txt`,
  `library/compiler-builtins/libm/LICENSE.txt` and
  `library/stdarch/LICENSE-MIT` from rust-lang/rust at tag `1.99.1`.
- `Unicode-3.0.txt` from the toolchain:
  `$(rustc --print sysroot)/share/doc/rust/licenses/Unicode-3.0.txt`.
- The crate files from the crates.io packages at the versions in
  rust-lang/rust `1.99.1` `library/Cargo.lock`; each package's SHA-256 matched
  the checksum in that lockfile.

The crate list is the std runtime crates in the Android sysroot
(`lib/rustlib/aarch64-linux-android/lib/*.rlib`), minus those not linked into a
cdylib (`test`, `getopts`, `proc_macro`, `profiler_builtins`). The toolchain's
`share/doc/rust/COPYRIGHT-library.html` was not used as is: it also lists
build-time tools (clap, etc.) and, in 1.99.1, misses addr2line, adler2,
miniz_oxide, object and memchr 2.8.3.

To update when `rust-toolchain.toml` changes: diff `library/Cargo.lock`
between the old and new tags, refresh the files above for every crate whose
version changed (and any crate added to the sysroot), and re-copy the
rust-lang/rust files at the new tag.
