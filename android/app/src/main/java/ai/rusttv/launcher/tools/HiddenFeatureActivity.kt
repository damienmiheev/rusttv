package ai.rusttv.launcher.tools

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.os.Bundle
import android.util.Log

/**
 * Starts or stops a vendor service the adb shell cannot reach itself, for the
 * experimental `rusttv tools` commands (TOOLS.md). No UI; it finishes at once.
 *
 * Only callers holding android.permission.DUMP (the adb shell, the system) can
 * start it: the manifest sets that permission. It acts only on the exact
 * extras below and ignores everything else, so it is inert without them.
 *
 *   am start -n ai.rusttv.launcher/.tools.HiddenFeatureActivity \
 *       --es feature airplay --es action start|stop
 */
class HiddenFeatureActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val feature = intent.getStringExtra("feature")
        val action = intent.getStringExtra("action")
        if (feature == "airplay" && (action == "start" || action == "stop")) {
            airplay(action)
        } else {
            Log.i(TAG, "ignored: feature=$feature action=$action")
        }
        finish()
    }

    /**
     * MediaTek's AirPlay `BootupService` connects the AirPlay HAL and tells the
     * daemon setup is done. It is guarded by a normal-level permission this app
     * declares, which the adb shell does not hold.
     */
    private fun airplay(action: String) {
        val service = Intent().setComponent(AIRPLAY_BOOTUP_SERVICE)
        try {
            if (action == "start") {
                val started = startService(service)
                Log.i(TAG, "airplay start: ${started?.flattenToShortString() ?: "not found"}")
            } else {
                val stopped = stopService(service)
                Log.i(TAG, "airplay stop: ${if (stopped) "stopped" else "was not running"}")
            }
        } catch (e: SecurityException) {
            Log.w(TAG, "airplay $action: not allowed: ${e.message}")
        } catch (e: IllegalStateException) {
            // Background start restriction.
            Log.w(TAG, "airplay $action: not allowed now: ${e.message}")
        }
    }

    private companion object {
        const val TAG = "RustTV.HiddenFeature"
        val AIRPLAY_BOOTUP_SERVICE = ComponentName(
            "com.mediatek.partner.airplay_sys",
            "com.mediatek.partner.airplay_sys.BootupService",
        )
    }
}
