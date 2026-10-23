# Spike: Slint launcher on Android TV (Phase 1b)

Throwaway prototype answering RISKS.md uncertainties 1–4. **Not product
code**: no tests, minimal error handling, kept outside the Cargo workspace.
Delete it once the real launcher exists. Results are summarized in
RESEARCH.md §8.

## What it is

A Rust + Slint home screen (`src/lib.rs`) in a Gradle app whose activity is a
Kotlin subclass of `NativeActivity` (`android/`). A Kotlin `Bridge` object
lists TV apps, renders their banners to ARGB pixels and launches them; Rust
calls it over JNI (`jni` 0.21). Renderer: FemtoVG (OpenGL ES).

## Build and run

Requires the Android SDK (platform 34), NDK 30.0.16248370, JDK 17,
`cargo-ndk`, and the Rust target `aarch64-linux-android`.

```bash
JAVA_HOME=/path/to/jdk17 ./build.sh
adb install -r android/app/build/outputs/apk/release/app-release.apk
adb shell am start -n ai.rusttv.spike/.LauncherActivity
adb logcat -s RustTVSpike
```

## Results (2026-10-23, Google TV emulator, API 34, arm64, 1080p)

| Question | Result |
|---|---|
| 1. Does Slint render on Android TV? | **Yes.** FemtoVG renderer, 1080p at 2x density (960x540 logical). Window ready ~23 ms into `android_main`. |
| 2. Do D-pad keys reach Slint? | **Yes.** Arrows arrive as Slint `Key.*Arrow` (U+F700–F703); OK/DPAD_CENTER as `Key.Return` (`"\n"`); BACK as U+F748 and must be consumed, or Android closes the launcher. Slint has no spatial focus navigation; a selected-index model in Slint was enough. |
| 3. Kotlin in the same APK? | **Yes,** without GameActivity: a Kotlin `NativeActivity` subclass with `hasCode=true`. JNI `FindClass` can't see app classes from the native thread; load them through the activity's class loader. |
| 4. List, show and launch apps? | **Yes.** `queryIntentActivities(LEANBACK_LAUNCHER)` with `<queries>` (no `QUERY_ALL_PACKAGES`), banners drawn to 320x180 ARGB in Kotlin and copied to a Slint image; 4 apps with banners in ~23 ms. Launch via `getLeanbackLaunchIntentForPackage` works. |
| 5. Size and resources (rough) | APK 12.4 MB (native lib 11.8 MB, stored uncompressed). Idle PSS 43.9 MB vs 60.9 MB for the stock launcher; idle CPU 0% for both. Cold start median 189 ms vs 318 ms for the stock launcher (5 runs each, `am start -W`). Emulator numbers: not comparable or publishable, and the stock launcher does far more work. |
| 6. Can it become Home on Google TV? | **No, not by preference.** See below. |

## Finding: Google TV keeps Home

`cmd package set-home-activity` and `cmd role add-role-holder
android.app.role.HOME` both succeed and the spike becomes the HOME role
holder and preferred activity, but the Home button and the HOME intent still
open `com.google.android.apps.tv.launcherx`.

Cause: launcherx declares its HOME intent filter with `priority=2`. Android
picks the highest-priority match before it consults preferred activities, and
non-system apps are capped at priority 0. So no third-party launcher can win
while launcherx is enabled. This is why debloat tools disable the stock
launcher and Projectivy uses an accessibility-service Home override.

The emulator's Home settings were restored afterwards.

## Build notes

- `cargo-ndk --platform 26` sets `ANDROID_PLATFORM=26`; Slint's build script
  then looks for the API 26 SDK platform. `build.sh` sets `ANDROID_JAR` to the
  installed platform 34 instead.
- The Android SDK command-line tools now ship an `android` CLI; `sdkmanager`
  is deprecated.
- Release builds are signed with the debug key (spike only).
