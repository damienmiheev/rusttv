package ai.rusttv.launcher

import android.app.NativeActivity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Bundle
import android.view.WindowManager

/**
 * The home screen. All UI and logic live in the Rust library
 * `librusttv_launcher.so` (crates/rusttv-launcher), which calls [Bridge].
 */
class LauncherActivity : NativeActivity() {
    /** Flags app installs, updates and removals so Rust re-reads the list on resume. */
    private val packageReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            Bridge.markAppsChanged()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // NativeActivity.onCreate sets STATE_UNSPECIFIED (overriding the
        // manifest), and its content view reports itself as a text editor, so
        // Android auto-showed the keyboard on focus until Slint hid it again.
        // The home screen has no text input: keep it hidden.
        window.setSoftInputMode(
            WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN or
                WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING
        )
        // Registered at runtime: since Android 8 these broadcasts no longer reach
        // receivers declared in the manifest. Kept while the activity exists, so
        // changes made while another app is in front are seen on return.
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_PACKAGE_ADDED)
            addAction(Intent.ACTION_PACKAGE_CHANGED)
            addAction(Intent.ACTION_PACKAGE_REMOVED)
            addAction(Intent.ACTION_PACKAGE_REPLACED)
            addDataScheme("package")
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            // System broadcasts still arrive; other apps cannot send to it.
            registerReceiver(packageReceiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            registerReceiver(packageReceiver, filter)
        }
    }

    override fun onResume() {
        // Before super, which hands the resume to Rust: Rust then reads the
        // status as "checking" and follows the check to its end.
        Updater.checkIfDue(this)
        super.onResume()
    }

    override fun onDestroy() {
        unregisterReceiver(packageReceiver)
        super.onDestroy()
    }
}
