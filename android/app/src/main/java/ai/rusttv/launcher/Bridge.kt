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
    /**
     * Marks the window secure while the review screen is open (HISTORY.md
     * § 6.2): `FLAG_SECURE` makes adb screen captures show black, and other
     * apps' overlays are hidden (Android 12+).
     */
    @JvmStatic
    fun setSecure(activity: android.app.Activity, on: Boolean) {
        activity.runOnUiThread {
            val window = activity.window
            if (on) {
                window.addFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE)
            } else {
                window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE)
            }
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) {
                window.setHideOverlayWindows(on)
            }
        }
    }

    /** Where the pointer's accessibility service stands: "ready" or "off" (REMOTE.md § 5). */
    @JvmStatic
    fun pointerState(): String = PointerService.state()

    /** Moves (0), clicks (1) or scrolls (2) the pointer; never blocks. */
    @JvmStatic
    fun pointer(dx: Int, dy: Int, action: Int) = PointerService.post(dx, dy, action)

    /** The package of the app in front, from the pointer's service, or "". */
    @JvmStatic
    fun pointerApp(): String = PointerService.app()

    /** The focused text field in the app in front as "label\nvalue", or "". */
    @JvmStatic
    fun pointerField(): String = PointerService.field()

    /** Types into that field: 0 insert, 1 delete `count` characters, 2 Enter. */
    @JvmStatic
    fun pointerType(op: Int, text: String, count: Int) = PointerService.type(op, text, count)

    /** YouTube's TV apps take a search as its web address, not as a search intent. */
    private val YOUTUBE = setOf("com.google.android.youtube.tv", "com.google.android.youtube.tvmusic")

    /** Whether [app] takes a search from a phone (REMOTE.md § 5): YouTube, or a search intent. */
    @JvmStatic
    fun canSearch(context: Context, app: String): Boolean =
        app in YOUTUBE ||
            context.packageManager.queryIntentActivities(
                Intent(Intent.ACTION_SEARCH).setPackage(app),
                0,
            ).isNotEmpty()

    /** Opens [app]'s results for a phone's [query]. */
    @JvmStatic
    fun search(context: Context, app: String, query: String) {
        val intent = if (app in YOUTUBE) {
            Intent(
                Intent.ACTION_VIEW,
                android.net.Uri.parse("https://www.youtube.com/results")
                    .buildUpon()
                    .appendQueryParameter("search_query", query)
                    .build(),
            )
        } else {
            Intent(Intent.ACTION_SEARCH).putExtra(android.app.SearchManager.QUERY, query)
        }
        intent.setPackage(app).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (_: ActivityNotFoundException) {
            // The app stopped taking searches: nothing happens.
        }
    }

    /** Hides the TV's on-screen keyboard while a phone types, or lets it show. */
    @JvmStatic
    fun hideKeyboard(hide: Boolean) = PointerService.hideKeyboard(hide)

    /** Whether the app in front draws for the D-pad only (nothing to click). */
    @JvmStatic
    fun pointerDpad(): Boolean = PointerService.dpad()

    /** Presses a key in the app in front (PointerService.key's codes). */
    @JvmStatic
    fun pointerKey(code: Int) = PointerService.key(code)

    /** Hides the pointer. */
    @JvmStatic
    fun pointerStop() = PointerService.hide()

    /** Android's accessibility settings, where the person turns the pointer on or off. */
    @JvmStatic
    fun openAccessibilitySettings(context: Context) {
        val intent = Intent(android.provider.Settings.ACTION_ACCESSIBILITY_SETTINGS)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            context.startActivity(intent)
        } catch (_: ActivityNotFoundException) {
            // No such screen on this TV: the row's text says where to look.
        }
    }

    /** Four random numbers from the OS source, for the review code. */
    @JvmStatic
    fun randomCode(): IntArray = IntArray(4) { java.security.SecureRandom().nextInt(4) }

    /** The local time zone's offset from UTC now, in seconds. */
    @JvmStatic
    fun utcOffsetSecs(): Int =
        java.util.TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 1000

    /** Set when an app was installed, updated or removed; cleared by [takeAppsChanged]. */
    private val appsChanged = AtomicBoolean(false)

    /** Called by [LauncherActivity]'s package receiver. */
    fun markAppsChanged() {
        appsChanged.set(true)
    }

    /** Whether the app list changed since the last call; clears the flag. */
    @JvmStatic
    fun takeAppsChanged(): Boolean = appsChanged.getAndSet(false)

    /** Launchable TV apps as "component\nlabel\ncategory", without this launcher. */
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
                // ApplicationInfo.CATEGORY_*: names a new folder (API 26+).
                "$component\n$label\n${info.applicationInfo.category}"
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

    /**
     * The app's tile picture for RustTV Remote's phones (REMOTE.md § 4), as
     * WebP: a kind byte (0 banner, 1 icon) then the file, or null with
     * neither. Drawn again for each phone that asks, on the launcher's
     * picture thread, so nothing is kept in memory for it.
     */
    @JvmStatic
    fun tilePicture(context: Context, component: String, width: Int, height: Int, iconSize: Int): ByteArray? {
        val pm = context.packageManager
        val name = ComponentName.unflattenFromString(component) ?: return null
        val banner = try {
            pm.getActivityBanner(name) ?: pm.getApplicationBanner(name.packageName)
        } catch (e: PackageManager.NameNotFoundException) {
            null
        }
        val (kind, drawable) = when {
            banner != null -> 0 to banner
            else -> 1 to (try {
                pm.getActivityIcon(name)
            } catch (e: PackageManager.NameNotFoundException) {
                return null
            })
        }
        val (w, h) = if (kind == 0) width to height else iconSize to iconSize
        val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, w, h)
        drawable.draw(Canvas(bitmap))
        val out = java.io.ByteArrayOutputStream()
        out.write(kind)
        bitmap.compress(webpFormat(), 80, out)
        bitmap.recycle()
        return out.toByteArray()
    }

    /** ARGB pixels (a built-in tile's banner) as WebP, for the phones. */
    @JvmStatic
    fun webp(pixels: IntArray, width: Int, height: Int): ByteArray {
        val bitmap = Bitmap.createBitmap(pixels, width, height, Bitmap.Config.ARGB_8888)
        val out = java.io.ByteArrayOutputStream()
        bitmap.compress(webpFormat(), 80, out)
        bitmap.recycle()
        return out.toByteArray()
    }

    @Suppress("DEPRECATION") // WEBP_LOSSY needs API 30; WEBP is lossy below 100.
    private fun webpFormat() =
        if (android.os.Build.VERSION.SDK_INT >= 30) Bitmap.CompressFormat.WEBP_LOSSY else Bitmap.CompressFormat.WEBP

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

    /** The app's language as a BCP 47 tag, e.g. "uk-UA" (the first of the user's languages). */
    @JvmStatic
    fun language(context: Context): String =
        context.resources.configuration.locales[0].toLanguageTag()

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
