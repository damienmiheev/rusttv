# Scripts

Developer tools. They are not part of `rusttv` and are not installed with it.

| Script | What it does | Changes the device? |
|---|---|---|
| `licenses.sh [output]` | Third-party licence notices for everything shipped: Rust crates via `cargo-about` (`about.toml`), the Slint licence, Skia and the libraries compiled into it, the LLVM runtimes, the Rust standard library and cleanyourtv's package data (`licenses/`). Fails on a licence not accepted in `about.toml`; CI runs it. | No |
| `release-info.sh version\|signing-key [<rev>] \| apk-cert` | Release facts the release workflows read: the workspace version, `RELEASE_SIGNING_KEY` (fails if unset) and the APK certificate's SHA-256. Needs no Rust toolchain. | No |
| `brand/render.sh` | Renders the brand assets (app icon layers, TV banner, logo) from the HTML templates in `scripts/brand/` with headless Chrome (docs/brand/README.md). | No |
| `tv-capture.sh <serial>` | Read-only capture of a TV: the raw `adb` outputs the fixtures use, `rusttv` JSON reads, compat profile. Writes to `.rusttv/captures/` (git-ignored); unit identifiers are redacted. | No |

## TV session checklist

For the first session on a real TV (e.g. the Philips Google TV). Steps marked
**ask first** change something on the TV: the owner approves each one in that
session (CLAUDE.md, SECURITY.md).

1. **Connect** (owner): Developer options → network debugging on; then
   `rusttv device connect <tv-ip>` and accept the prompt on the TV. Note the
   serial from `rusttv device list`.
2. **Capture** (read-only): `scripts/tv-capture.sh <serial>`. Review the folder it prints.
   - Fixtures: copy the raw `.txt` files you need to
     `crates/rusttv-core/src/fixtures/<maker>-<model>-api<N>/` with a README
     listing each command (see the emulator fixture READMEs).
   - Profile: save `compat-profile.toml` as `compat/devices/<suggested-name>.toml`
     and fill in `tested.notes`.
   - Check `abi.txt`: its first entry is the ABI to install in step 3.
   - Keep `launcher-status.json` (and the raw `home-activities-all.txt`): it shows
     the stock launcher's Home priority, the evidence for decision D12.
3. **Install the launcher** (**ask first**): install the APK for the first ABI
   in `abi.txt` (`arm64-v8a`, `armeabi-v7a` or `x86_64`):
   `adb -s <serial> install android/app/build/outputs/apk/release/app-<abi>-release.apk`,
   then `adb -s <serial> shell am start -n ai.rusttv.launcher/.LauncherActivity`.
   Check with the remote: every tile reachable, OK launches, hold OK opens a menu that moves or hides a tile,
   Back stays on the home screen, coming back from an app works, labels readable
   from the sofa, backdrop look. Note anything odd.
4. **Benchmark** (**ask first**: it force-stops and starts the apps it measures, R1):

   ```bash
   rusttv performance benchmark --serial <serial> \
       --app ai.rusttv.launcher/.LauncherActivity \
       --app <stock home from home-activity.txt> --json \
       > benchmarks/raw/<device>/<date>/launchers.json
   ```

   Reboot and wait 5 minutes first (BENCHMARK_PLAN.md § Protocol). Numbers from
   one run are not a claim until they meet the claim rule there.
5. **Home button**: press Home and note which launcher opens; compare with
   `rusttv launcher status --serial <serial>`. Do not disable the
   stock launcher: Home takeover is a separate, opt-in flow (SECURITY.md § Home
   takeover) that is not implemented yet.
6. **Clean up** (**ask first**): `adb -s <serial> uninstall ai.rusttv.launcher` if
   the owner doesn't want it left on the TV.
