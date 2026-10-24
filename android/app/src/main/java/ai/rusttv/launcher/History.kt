package ai.rusttv.launcher

import android.content.Context
import android.content.pm.ApplicationInfo
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import java.security.SecureRandom

/**
 * The history's Android side (HISTORY.md): helpers the Rust history thread
 * calls through JNI, and the native entry points the door
 * ([HistoryProvider]) calls into (crates/rusttv-launcher/src/android.rs).
 */
object History {
    private val random = SecureRandom()

    @Volatile
    private var loaded = false

    /**
     * Loads the native library for a process the door started without the
     * home screen (the activity loads it itself). Idempotent.
     */
    @Synchronized
    fun load() {
        if (!loaded) {
            System.loadLibrary("rusttv_launcher")
            loaded = true
        }
    }

    @JvmStatic
    fun filesDir(context: Context): String = context.filesDir.absolutePath

    @JvmStatic
    fun setting(context: Context, index: Int): String? = SecureSettings.get(context, index)

    @JvmStatic
    fun putSetting(context: Context, index: Int, value: String): Boolean =
        SecureSettings.put(context, index, value)

    @JvmStatic
    fun canWriteSettings(context: Context): Boolean = SecureSettings.canWrite(context)

    @JvmStatic
    fun bootCount(context: Context): Long =
        Settings.Global.getInt(context.contentResolver, Settings.Global.BOOT_COUNT, 0).toLong()

    @JvmStatic
    fun elapsedMs(): Long = SystemClock.elapsedRealtime()

    /** Puts the calling thread (the Rust history thread) at background priority. */
    @JvmStatic
    fun lowerPriority() {
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_BACKGROUND)
    }

    /** Random bytes from the OS source (history ids, the review code). */
    @JvmStatic
    fun randomBytes(count: Int): ByteArray = ByteArray(count).also { random.nextBytes(it) }

    /**
     * A debuggable or shell-profileable build: approvals are off, because the
     * shell could write such an app's files (`run-as`) or read its memory.
     */
    @JvmStatic
    fun testBuild(context: Context): Boolean {
        val info = context.applicationInfo
        val debuggable = info.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        val profileable = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && info.isProfileableByShell
        return debuggable || profileable
    }

    /** The door's `export/<since>` read: header line and pack, or an error line. */
    @JvmStatic
    external fun nativeExport(context: Context, since: String?): ByteArray

    /** The door's `pending` read: the list as TOML, or an error line. */
    @JvmStatic
    external fun nativePending(context: Context): ByteArray

    /** The door's `commit` call: processes an inbox file; returns the result line. */
    @JvmStatic
    external fun nativeCommit(context: Context, arg: String, bytes: ByteArray): String
}
