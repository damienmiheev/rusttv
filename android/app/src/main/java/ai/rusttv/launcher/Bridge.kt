package ai.rusttv.launcher

import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Android APIs the Rust side calls through JNI (crates/rusttv-launcher/src/android.rs).
 * Coarse calls only: one call per operation, plain types across the boundary.
 * Apps are identified by their flattened component name, "package/activity".
 */
object Bridge {
    /** Set when an app was installed, updated or removed; cleared by [takeAppsChanged]. */
    private val appsChanged = AtomicBoolean(false)

    /** Called by [LauncherActivity]'s package receiver. */
    fun markAppsChanged() {
        appsChanged.set(true)
    }

    /** Whether the app list changed since the last call; clears the flag. */
    @JvmStatic
    fun takeAppsChanged(): Boolean = appsChanged.getAndSet(false)

    /** Launchable TV apps as "component\nlabel", without this launcher. */
    @JvmStatic
    fun listApps(context: Context): Array<String> {
        val pm = context.packageManager
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LEANBACK_LAUNCHER)
        return pm.queryIntentActivities(intent, 0)
            .map { it.activityInfo }
            .filter { it.packageName != context.packageName }
            .map { info ->
                val component = ComponentName(info.packageName, info.name).flattenToShortString()
                val label = info.loadLabel(pm).toString().replace('\n', ' ')
                "$component\n$label"
            }
            .toTypedArray()
    }

    /** The activity's (or else the app's) TV banner as ARGB pixels, or null if it has none. */
    @JvmStatic
    fun banner(context: Context, component: String, width: Int, height: Int): IntArray? {
        val pm = context.packageManager
        val name = ComponentName.unflattenFromString(component) ?: return null
        val drawable = try {
            pm.getActivityBanner(name) ?: pm.getApplicationBanner(name.packageName)
        } catch (e: PackageManager.NameNotFoundException) {
            null
        }
        return drawable?.let { pixels(it, width, height) }
    }

    /** The activity's icon (Android falls back to the app's) as ARGB pixels, or null. */
    @JvmStatic
    fun icon(context: Context, component: String, width: Int, height: Int): IntArray? {
        val name = ComponentName.unflattenFromString(component) ?: return null
        val drawable = try {
            context.packageManager.getActivityIcon(name)
        } catch (e: PackageManager.NameNotFoundException) {
            null
        }
        return drawable?.let { pixels(it, width, height) }
    }

    /** Starts the TV activity; false if it no longer exists. */
    @JvmStatic
    fun launch(context: Context, component: String): Boolean {
        val name = ComponentName.unflattenFromString(component) ?: return false
        val intent = Intent(Intent.ACTION_MAIN)
            .addCategory(Intent.CATEGORY_LEANBACK_LAUNCHER)
            .setComponent(name)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_RESET_TASK_IF_NEEDED)
        return try {
            context.startActivity(intent)
            true
        } catch (e: ActivityNotFoundException) {
            false
        }
    }

    /**
     * Every banner and icon has been copied to the launcher. The pictures the
     * system decoded for them (often larger than shown) are garbage now, but
     * their pixels stay in native memory until the next collection, which an
     * idle launcher may not see for a long time: ask for it once.
     */
    @JvmStatic
    fun artworkLoaded() {
        Runtime.getRuntime().gc()
    }

    /** The third-party licence notices packed into the APK by Gradle (app/build.gradle.kts). */
    @JvmStatic
    fun notices(context: Context): String =
        context.assets.open("licenses/THIRD_PARTY_NOTICES.txt").use {
            it.readBytes().toString(Charsets.UTF_8)
        }

    /** The updater's status, "kind" or "kind\tdetail" ([Updater]); a field read. */
    @JvmStatic
    fun updateStatus(context: Context): String = Updater.status(context)

    /** Starts a check for a newer launcher in the background. */
    @JvmStatic
    fun checkForUpdate(context: Context) = Updater.check(context)

    /** Starts installing the newer launcher in the background; Android asks the user to confirm. */
    @JvmStatic
    fun installUpdate(context: Context) = Updater.install(context)

    private fun pixels(drawable: Drawable, width: Int, height: Int): IntArray {
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, width, height)
        drawable.draw(Canvas(bitmap))
        val pixels = IntArray(width * height)
        bitmap.getPixels(pixels, 0, width, 0, 0, width, height)
        bitmap.recycle()
        return pixels
    }
}
