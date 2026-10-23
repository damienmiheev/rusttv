package ai.rusttv.launcher

import android.app.NativeActivity

/**
 * The home screen. All UI and logic live in the Rust library
 * `librusttv_launcher.so` (crates/rusttv-launcher), which calls [Bridge].
 */
class LauncherActivity : NativeActivity()
