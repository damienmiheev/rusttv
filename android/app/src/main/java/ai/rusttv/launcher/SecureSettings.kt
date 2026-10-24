package ai.rusttv.launcher

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.provider.Settings

/**
 * The only code in the launcher that writes a system setting (HISTORY.md § 7).
 *
 * `WRITE_SECURE_SETTINGS` would let the app write any global or secure
 * setting; this object restricts it to the three animation scales, by enum,
 * with the CLI's value rule. A build check fails if anything else in the APK
 * calls a settings writer (android/check-dex.sh).
 */
object SecureSettings {
    /** The allowlisted keys, in rusttv-history's `SETTING_KEYS` order. */
    private val KEYS = arrayOf(
        Settings.Global.ANIMATOR_DURATION_SCALE,
        Settings.Global.TRANSITION_ANIMATION_SCALE,
        Settings.Global.WINDOW_ANIMATION_SCALE,
    )

    /** The CLI's rule for a scale: 1–2 digits, then up to 3 decimals. */
    private val VALUE = Regex("""^[0-9]{1,2}(\.[0-9]{1,3})?$""")

    /** Whether the launcher was granted `WRITE_SECURE_SETTINGS` (from the Mac). */
    fun canWrite(context: Context): Boolean =
        context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) ==
            PackageManager.PERMISSION_GRANTED

    /** The key at [index], or null for any other index. */
    fun key(index: Int): String? = KEYS.getOrNull(index)

    /** An allowlisted setting's value; null when unset or not allowlisted. */
    fun get(context: Context, index: Int): String? =
        key(index)?.let { Settings.Global.getString(context.contentResolver, it) }

    /** Writes an allowlisted setting; false when refused or not allowed. */
    fun put(context: Context, index: Int, value: String): Boolean {
        val key = key(index) ?: return false
        if (!VALUE.matches(value) || !canWrite(context)) return false
        return try {
            Settings.Global.putString(context.contentResolver, key, value)
        } catch (e: SecurityException) {
            false
        }
    }
}
