# Phase 0 Research

Status legend: **[verified]** = checked against the cited source on 2026-10-23. **[unverified]** = from prior knowledge or secondary sources; confirm before relying on it.

## 1. Prior art

| Project | Stack | License | What we learn | Reuse policy |
|---|---|---|---|---|
| [CleanYourTV / tv-vibehack](https://github.com/Gathin23/tv-vibehack) | Java launcher (~474 KB, API 26+), pure-Python ADB client, shell scripts | MIT **[verified]** | Closest prior art. Pre-change state backup + `restore.sh`; "never leave the TV without a HOME" guard; launcher rollback if HOME doesn't switch; keeps setup wizard and HDMI/TV-input packages; post-firmware-update re-check; per-model package lists; advises closing network ADB when done | Clean-room Rust reimplementation of the ideas. Data files (device specs, lists) may be imported with attribution |
| [FLauncher](https://gitlab.com/flauncher/flauncher) | Flutter | GPL-3.0 **[verified]** | Proof that a non-native UI toolkit can deliver a usable TV launcher; categories/wallpaper UX | Study only. No code copied |
| [UAD-ng](https://github.com/Universal-Debloater-Alliance/universal-android-debloater-next-generation) | Rust + Iced GUI, ADB | GPL-3.0 **[verified]** | Rust ADB tooling patterns; community-maintained package list (`uad_lists.json`) with removal recommendations | Study only. The package list is GPL data and is **not** imported |
| [UAD (original)](https://github.com/0x192/universal-android-debloater) | Rust | GPL-3.0 **[unverified]** | Predecessor of UAD-ng; archived | Study only |
| Projectivy Launcher | Kotlin (closed source) | Proprietary | Most popular 3rd-party TV launcher; has a "launcher override" mechanism. Recent Google TV updates reportedly break the override for some users ([Android Authority](https://www.androidauthority.com/google-tv-update-blocks-projectivy-override-3649445)) **[verified report, scope unknown]** | Behavioural reference only; no decompilation of its code is planned |
| [mpv-android](https://github.com/mpv-android/mpv-android) | Kotlin + libmpv (C) | MIT (app) / mpv LGPL/GPL **[unverified]** | Pattern for native engine + thin Kotlin shell — relevant to future RustPlay | Later |
| [Servo](https://github.com/servo/servo) | Rust | MPL-2.0 **[unverified]** | Android support exists but is experimental; no Widevine DRM | Later (RustBrowse) |

**Takeaway:** the safety ideas are proven by CleanYourTV. RustTV's differentiation must come from (1) a measurably fast launcher, (2) a typed, testable safety engine instead of shell scripts, (3) structured CLI/MCP access for agents, (4) a vendor-neutral, schema-validated compatibility database.

## 2. Rust + Slint on Android

- Slint supports Android via the `android-activity` crate; feature `backend-android-activity-06`. Minimum API 26. **[verified, Slint docs]**
- Recommended Rust build tool in the docs is `xbuild` (git master, not the old 0.2.0 release). Skia is the documented renderer and compiles from source on first build (needs clang; slow first build). **[verified]**
- The documented setup uses `NativeActivity` with `android:hasCode="false"` — no Java code. We need Java (Kotlin shim), so we will use a custom Activity / GameActivity instead. Whether Slint's backend works with GameActivity, and how to build that with Gradle + cargo, is **not documented** → spike required.
- The internal backend crate is "work in progress, breaking changes in patch releases" → pin Slint with `=x.y.z`. **[verified]**
- No documentation of D-pad key mapping (KEYCODE_DPAD_* / DPAD_CENTER) to Slint key events, and Slint has no built-in spatial focus navigation. We will own selection state in Rust and handle keys in a root `FocusScope`. Key delivery must be verified on device.
- Licensing: Slint is GPL-3.0 / Royalty-free 2.0 / Commercial. Owner decision: use the Royalty-free license; it requires visible Slint attribution (e.g. `AboutSlint` widget or badge) — we will add it to an About screen.

## 3. Rust/Slint vs Kotlin (Views / Compose for TV)

| Aspect | Rust + Slint | Kotlin Views | Compose for TV |
|---|---|---|---|
| Startup | Native code, no ART class loading for UI; but loads a native lib (incl. Skia) | ART + framework already warm in zygote | ART + Compose runtime; known to be heavier on cold start |
| Idle memory | Own GPU context + Skia; unknown | Framework shared with zygote | Larger runtime |
| APK size | Several MB of native libs (estimate) | Smallest (~0.5 MB, e.g. CleanYourTV) | Larger |
| Android API access | Via JNI / Kotlin shim | Direct | Direct |
| Focus/D-pad | Hand-built | Built-in | Built-in (`androidx.tv`) |
| Shared code with CLI/MCP | Yes | No | No |

The project hypothesis is that a Rust + Slint launcher can be faster than the stock launcher and competitive with lightweight Kotlin launchers. This is **a hypothesis to be measured** (BENCHMARK_PLAN.md), not a claim. Comparing against the stock launcher, CleanYourTV's Java launcher, and one Kotlin launcher keeps the result honest.

## 4. Reverse-engineering tools

| Tool | Assessment | Decision |
|---|---|---|
| [JADX](https://github.com/skylot/jadx) | Mature, Apache-2.0, decompiles DEX to Java. Enough for understanding manifests, intent filters and API usage of APKs we are authorized to inspect | Use when needed, on authorized APKs only |
| [Ghidra](https://github.com/NationalSecurityAgency/ghidra) | Powerful native binary analysis; heavy. Little expected value — our questions are about Java-level Android APIs | Not planned |
| [REA](https://github.com/morluto/rea) | MCP server wrapping JADX/Apktool etc. for agents. Its page reports ~368k stars / 92k forks — implausible for a niche tool; treat as a supply-chain red flag until verified | Do not install. Use JADX directly |

Source code (AOSP, open-source launchers) is always preferred over binary analysis. No DRM circumvention, credential extraction, or copying of proprietary assets.

## 5. Rootless ADB capabilities

What the `shell` user (uid 2000) can typically do. **All [unverified] per device** — vendors restrict some; the CLI must probe, not assume.

| Area | Commands | Notes |
|---|---|---|
| Device info | `getprop`, `wm size/density`, `cat /proc/meminfo`, `df`, `dumpsys battery/display` | Read-only, broadly available |
| Packages | `pm list packages -f -s -3 -d -e`, `dumpsys package <pkg>`, `cmd package resolve-activity` | Package visibility rules do not apply to shell |
| Performance | `dumpsys meminfo`, `dumpsys gfxinfo`, `dumpsys cpuinfo`, `top -n 1`, `am start -W`, `perfetto` (API 28+) | Some `/proc/<pid>` data of other apps is restricted |
| App control | `am start`, `am force-stop`, `input keyevent` | Reversible |
| Settings | `settings get/put global|secure|system` (e.g. animation scales) | Shell has WRITE_SECURE_SETTINGS; record originals |
| Package state | `pm disable-user --user 0`, `pm enable`, `pm uninstall -k --user 0`, `cmd package install-existing` | Some vendors block disabling certain packages; uninstall-for-user is **excluded** from MVP |
| Default HOME | `cmd package set-home-activity <component>` | Availability and behavior on Google TV must be tested |
| Wireless ADB | `adb connect` (legacy TCP 5555), `adb pair` (API 30+) | Network ADB is unencrypted on legacy TCP; advise disabling when done |

Not possible rootless: writing `/system`, reading other apps' private data, granting signature permissions, changing kernel parameters.

## 6. MCP SDK

Official Rust SDK: [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) (tokio-based, `rmcp-macros` for tools). Versions seen range from 0.x to 3.1.0 across sources → check crates.io and pin when Phase 5 starts. **[partially verified]**

## 7. Rust TV/Android ecosystem notes

- `jni` crate: the de-facto JNI binding; used for the bridge.
- `android-activity`: used under Slint; supports NativeActivity and GameActivity backends **[unverified for Slint]**.
- Pure-Rust ADB clients exist (e.g. `adb_client`); deferred per ARCHITECTURE.md decision.

## Assumptions

- The owner's Philips Google TV allows Developer options and network/Wi-Fi ADB.
- Android TV emulator images (API 30–34) are adequate for functional testing but **not** for performance numbers.
- Most target TVs run Android 9–14 on ARM (armv7 or arm64).
