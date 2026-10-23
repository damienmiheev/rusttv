package ai.rusttv.spike

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas

/**
 * Android APIs the Rust side calls through JNI. Coarse calls only:
 * one call per operation, plain types across the boundary.
 */
object Bridge {
    /** Launchable TV apps as "label\npackage", sorted by label. */
    @JvmStatic
    fun listApps(context: Context): Array<String> {
        val pm = context.packageManager
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LEANBACK_LAUNCHER)
        return pm.queryIntentActivities(intent, 0)
            .filter { it.activityInfo.packageName != context.packageName }
            .map { "${it.loadLabel(pm)}\n${it.activityInfo.packageName}" }
            .distinct()
            .sortedBy { it.lowercase() }
            .toTypedArray()
    }

    /** The app's TV banner (or icon), drawn at width x height, as ARGB pixels. */
    @JvmStatic
    fun banner(context: Context, packageName: String, width: Int, height: Int): IntArray {
        val pixels = IntArray(width * height)
        val pm = context.packageManager
        val drawable = try {
            pm.getApplicationBanner(packageName) ?: pm.getApplicationIcon(packageName)
        } catch (e: PackageManager.NameNotFoundException) {
            return pixels
        }
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, width, height)
        drawable.draw(Canvas(bitmap))
        bitmap.getPixels(pixels, 0, width, 0, 0, width, height)
        bitmap.recycle()
        return pixels
    }

    /** Starts the app's TV launch activity. */
    @JvmStatic
    fun launch(context: Context, packageName: String): Boolean {
        val pm = context.packageManager
        val intent = pm.getLeanbackLaunchIntentForPackage(packageName)
            ?: pm.getLaunchIntentForPackage(packageName)
            ?: return false
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
        return true
    }
}
