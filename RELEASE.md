# Releasing RustTV

How a release is built, signed and checked. Nothing here is automated past
building: tags, GitHub releases and uploads are done by hand, by the owner.

## What a release contains

| File | Built by | Signed |
|---|---|---|
| `rusttv-<version>-<target>.tar.gz` for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`: `rusttv`, `rusttv-mcp`, README.md, CLI.md, MCP.md, SECURITY.md, LICENSE-MIT, LICENSE-APACHE, THIRD_PARTY_NOTICES.txt | `.github/workflows/release-build.yml` | No (SHA-256 in `SHA256SUMS`) |
| `rusttv-launcher-<version>-<abi>.apk` for `arm64-v8a`, `armeabi-v7a`, `x86_64` | `android/build.sh`, locally with the release key | Release key (below) |
| `SHA256SUMS` covering all of the above | the workflow (CLI), `shasum -a 256` (APKs) | Update signing key: `SHA256SUMS.minisig` (below) |
| `SHA256SUMS.minisig` | `minisign -Sm SHA256SUMS`, locally | — |

`rusttv update` (CLI.md § Updates) installs the CLI archive and the APK for
the device's ABI only if `SHA256SUMS.minisig` is a valid signature of
`SHA256SUMS` by the update signing key and the file matches `SHA256SUMS`, so
the file names above are a contract: keep them exactly.

The macOS binaries are not notarized; Gatekeeper blocks them until the user
runs `xattr -d com.apple.quarantine rusttv rusttv-mcp` (say so in the release
notes).

## Version

One version for everything: `[workspace.package] version` in `Cargo.toml`.

- The crates, `rusttv --version`, the MCP server's `serverInfo`, the
  launcher's About page, `rusttv_version` in benchmark results and compat
  profiles all read it (`rusttv_core::VERSION` / `CARGO_PKG_VERSION`).
- The APK's `versionName` is the same string and its `versionCode` is
  `major * 10000 + minor * 100 + patch` (0.1.0 → 100), both computed by
  `android/app/build.gradle.kts`. Minor and patch must stay below 100.
- All three ABI APKs share one `versionCode`; fine for sideloading, not for a
  store that hosts several APKs of one app.

Bump it in `Cargo.toml` only, then `cargo check` so `Cargo.lock` follows.

## Signing

The APK is signed with a project release key that never enters the repository
or CI. Without it, `android/build.sh` signs release APKs with the local debug
key, so builds keep working; those APKs are for testing, never for a release.
CI has no key, so the APKs the workflow uploads are debug-signed too.

### Create the key (once)

On the owner's machine, outside the repository:

```bash
mkdir -p ~/.rusttv-release && chmod 700 ~/.rusttv-release
keytool -genkeypair -v -keystore ~/.rusttv-release/rusttv-release.jks -storetype PKCS12 -alias rusttv -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=RustTV"
```

`keytool` comes with the JDK. It asks for the keystore password (PKCS12 uses
the same password for the key). Android requires a key valid past 2033;
10000 days is the usual choice.

Back it up now: **if the key is lost, installed launchers cannot be updated**
(Android refuses an update signed with a different key; users would have to
uninstall first). Keep at least two copies of the `.jks` file in separate
places (for example an encrypted USB drive and a password manager's file
attachment), and the password in the password manager. Never put either in
the repository, CI secrets, an issue or a chat.

Record the certificate fingerprint for the release notes, so users can check
an APK (`apksigner verify --print-certs`):

```bash
keytool -list -v -keystore ~/.rusttv-release/rusttv-release.jks -alias rusttv
```

### Use the key

Either set four environment variables for the build:

```bash
export RUSTTV_KEYSTORE_FILE=~/.rusttv-release/rusttv-release.jks RUSTTV_KEY_ALIAS=rusttv
read -rs RUSTTV_KEYSTORE_PASSWORD && export RUSTTV_KEYSTORE_PASSWORD RUSTTV_KEY_PASSWORD="$RUSTTV_KEYSTORE_PASSWORD"
```

or write `android/keystore.properties` (git-ignored, as are `*.jks` and
`*.keystore`):

```properties
storeFile=/Users/you/.rusttv-release/rusttv-release.jks
storePassword=...
keyAlias=rusttv
keyPassword=...
```

Environment variables win over the file, field by field. A relative
`storeFile` is resolved against `android/`. If only some of the four values
are set, the build fails instead of falling back to the debug key.

Check the result:

```bash
"$ANDROID_HOME"/build-tools/34.0.0/apksigner verify --print-certs android/app/build/outputs/apk/release/app-arm64-v8a-release.apk
```

The certificate must be `CN=RustTV` with the recorded fingerprint, not
`CN=Android Debug`.

## Update signing key

A second owner key, separate from the APK key: a
[minisign](https://jedisct1.github.io/minisign/) key that signs `SHA256SUMS`.
Its public half is compiled into `rusttv`, which then trusts nothing else for
`rusttv update` (SECURITY.md § Updates). It covers the CLI archives, which
have no other signature, and the APK downloads; Android's own APK signature
check stays on top for the launcher.

### Create the key (once)

`minisign` is in Homebrew (`brew install minisign`) and most Linux
distributions. On the owner's machine:

```bash
minisign -G -p ~/.rusttv-release/rusttv-release.pub -s ~/.rusttv-release/rusttv-release.key
```

It asks for a password that encrypts the secret key; use a new one and keep
it in the password manager. Back up `rusttv-release.key` like the `.jks`
(two copies in separate places, never in the repository, CI, an issue or a
chat): **if it is lost, `rusttv update` can't install a release until users
install a build with a new public key by hand**; if it leaks, anyone can sign
a release that `rusttv update` installs, so replace it at once (new key, new
build, say so in the release notes).

### Put the public key into RustTV (once)

`~/.rusttv-release/rusttv-release.pub` has two lines; the second is the key
(base64, starting `RW`). In `crates/rusttv-core/src/update.rs`, replace

```rust
pub const RELEASE_SIGNING_KEY: Option<&str> = None;
```

with

```rust
pub const RELEASE_SIGNING_KEY: Option<&str> = Some("RW...");
```

and commit it. Until then every build refuses `rusttv update cli` and
`rusttv update launcher` with `UPDATE_VERIFY_FAILED`. Builds made before the
key was set can't update themselves: users install the first signed release
by hand. Publish the public key in the release notes and README too.

### Sign a release

After `SHA256SUMS` is complete (CLI archives and APKs):

```bash
minisign -Sm SHA256SUMS -s ~/.rusttv-release/rusttv-release.key -t "RustTV <version>"
minisign -Vm SHA256SUMS -p ~/.rusttv-release/rusttv-release.pub
```

The first writes `SHA256SUMS.minisig` (pre-hashed, minisign's default;
RustTV refuses legacy signatures made with `-l`); the second checks it.

## Checklist

Work on a release branch; tick each item in the release pull request.

### Before the tag

- [ ] `main` is green in CI (fmt, clippy, tests, cargo deny, licence notices, Android APK).
- [ ] Version bumped in `Cargo.toml` and `Cargo.lock` (above).
- [ ] `RELEASE_SIGNING_KEY` in `crates/rusttv-core/src/update.rs` is `Some(...)` with the update signing key's public half, committed before the Release build runs. A release built while it is `None` can never update itself (`can_verify` is false in `rusttv update check --json`).
- [ ] `CHANGELOG.md`: the version's section is complete and dated; "unreleased" removed.
- [ ] CLI.md, MCP.md and README.md match the commands and JSON schema shipped. A changed JSON field or error code has a schema version bump (CLAUDE.md).
- [ ] Benchmarks current: the numbers quoted anywhere (README, release notes) come from a run of this version, made by BENCHMARK_PLAN.md, with raw data in `benchmarks/raw/`. No performance claim without it.
- [ ] Compat data current: every device named as compatible has a profile in `compat/devices/` tested with this version (`tested.rusttv_version`). No compatibility claim without it.
- [ ] Licence notices: `scripts/licenses.sh` passes; the APK has `assets/licenses/THIRD_PARTY_NOTICES.txt` and the About page shows it; every CLI archive has THIRD_PARTY_NOTICES.txt, LICENSE-MIT and LICENSE-APACHE. If Slint, Skia, the NDK or the Rust toolchain changed, the files under `licenses/` were updated.
- [ ] `third_party/` patches still needed, and their upstream issues linked (third_party/README.md).
- [ ] Trademark/name check done (ROADMAP Phase 6).

### Build

- [ ] Run the **Release build** workflow by hand (`workflow_dispatch`) on the release commit; all jobs pass. Download the `cli-*` and `SHA256SUMS` artifacts.
- [ ] Build the APKs locally with the release key: `android/build.sh`. Check the signature (above).
- [ ] Rename the APKs to `rusttv-launcher-<version>-<abi>.apk` and add their sums, in the directory holding the downloaded CLI archives and `SHA256SUMS`:

  ```bash
  version=0.1.0  # the Cargo version
  for abi in arm64-v8a armeabi-v7a x86_64; do
    cp "<repo>/android/app/build/outputs/apk/release/app-$abi-release.apk" "rusttv-launcher-$version-$abi.apk"
  done
  shasum -a 256 rusttv-launcher-*.apk >> SHA256SUMS
  ```
- [ ] Sign `SHA256SUMS` with the update signing key and check the signature (above); `RELEASE_SIGNING_KEY` in `rusttv-core` is the key's public half.
- [ ] Smoke test: `rusttv --version` and `rusttv device list` on macOS and Linux; the signed APK installs (`rusttv launcher install`) and starts on the emulator and on at least one TV with a compat profile; About shows the version and the notices.

### Tag and publish (owner only)

- [ ] Signed tag on the release commit: `git tag -s v<version> -m "RustTV <version>"`, then push it. The tag must equal `v` + the Cargo version; the workflow fails otherwise. The tag push re-runs the Release build, which must pass; its archives are not byte-identical to the tested ones (tar records file times), so publish the files from the run whose `SHA256SUMS` you signed, never the tag run's.
- [ ] GitHub release from the tag with the CLI archives, the signed APKs, `SHA256SUMS` and `SHA256SUMS.minisig`; published (not a draft or pre-release, which `rusttv update` ignores).
- [ ] From the previous release's binary (none for the first release): `rusttv update check` sees the new version; `rusttv update cli --dry-run` and `rusttv update launcher --dry-run` verify and plan it.
- [ ] Release notes: the CHANGELOG section; supported devices (only those with a compat profile); how to install (CLI archive, `rusttv launcher install`); the APK signing certificate's SHA-256 fingerprint; the macOS quarantine note; known issues; the safety model in one paragraph with a link to SECURITY.md.

## Never in a release

- The keystore, its passwords, `android/keystore.properties`, the update signing key (`rusttv-release.key`) or its password, or any `.env` file.
- Debug-signed APKs (the CI artifacts are debug-signed).
- Anything from `internal/`, `.rusttv/` (plans, journals, captures), or raw
  device captures that identify a person, a home network or a serial number
  beyond what `compat/` and `benchmarks/raw/` already publish.
- Binaries built from a dirty tree or without `--locked`.
- Performance or compatibility claims without the data above.
- Spikes (`spikes/`) and the desktop dev launcher (`rusttv-launcher-desktop`).
