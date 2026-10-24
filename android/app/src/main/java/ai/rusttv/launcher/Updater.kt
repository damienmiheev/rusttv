package ai.rusttv.launcher

import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.PackageInfo
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.content.pm.Signature
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.SocketTimeoutException
import java.net.URL
import java.net.UnknownHostException
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import javax.net.ssl.HttpsURLConnection
import org.json.JSONException

/**
 * The launcher's updater (SECURITY.md § Launcher updates, ARCHITECTURE.md D18).
 *
 * Once a day, on resume, it reads the latest release from GitHub (a small
 * JSON); on OK in About it downloads the APK for this device, checks it, and
 * hands it to Android's PackageInstaller, which asks the user to confirm.
 * Network work runs on one background thread. The Rust side polls [status].
 */
object Updater {
    private const val FEED = "https://api.github.com/repos/damienmiheev/rusttv/releases/latest"
    private const val CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000L

    /** A failed daily check (e.g. no network yet after boot) is retried on resume after this long. */
    private const val RETRY_INTERVAL_MS = 60 * 60 * 1000L
    private const val CONNECT_TIMEOUT_MS = 15_000
    private const val READ_TIMEOUT_MS = 30_000
    private const val MAX_REDIRECTS = 5
    private const val MAX_FEED_BYTES = 1 shl 20
    private const val MAX_SUMS_BYTES = 64 shl 10

    /** Far above a launcher APK (~10 MB); the download must also match the asset's size. */
    private const val MAX_APK_BYTES = 64L shl 20

    private const val PREFS = "updater"
    private const val KEY_LAST_CHECK = "lastCheck"
    private const val KEY_AVAILABLE = "available"
    private const val APK_FILE = "update.apk"

    // Status kinds, parsed by `UpdateStatus::from_bridge` in platform.rs; a
    // detail follows a tab.
    private const val IDLE = "idle"
    private const val CHECKING = "checking"
    private const val UP_TO_DATE = "uptodate"
    private const val INSTALLING = "installing"
    private const val INSTALLED = "installed"

    private val executor = Executors.newSingleThreadExecutor()

    /** Set while a check or install runs on [executor]; a second request is dropped. */
    private val busy = AtomicBoolean(false)

    @Volatile
    private var current: String? = null

    /** [SystemClock.elapsedRealtime] of the last daily check started, to space out retries. */
    @Volatile
    private var lastAttempt = 0L

    private class UpdateException(message: String) : Exception(message)

    /** The feed answered 404: the repository has no release yet. */
    private class NoRelease : Exception()

    /** The status for the About page: "kind" or "kind\tdetail". */
    fun status(context: Context): String = current ?: initialStatus(context.applicationContext).also { current = it }

    /** Called on every resume: checks if the last successful check is a day old. */
    fun checkIfDue(context: Context) {
        val last = prefs(context).getLong(KEY_LAST_CHECK, 0)
        // A clock set back counts as due.
        if (System.currentTimeMillis() - last in 0 until CHECK_INTERVAL_MS) return
        val now = SystemClock.elapsedRealtime()
        if (lastAttempt != 0L && now - lastAttempt < RETRY_INTERVAL_MS) return
        lastAttempt = now
        check(context)
    }

    /** Starts a check in the background, unless a check or install runs. */
    fun check(context: Context) {
        val app = context.applicationContext
        if (!busy.compareAndSet(false, true)) return
        current = CHECKING
        executor.execute {
            try {
                val installed = installedVersionCode(app)
                val release = fetchRelease()
                val edit = prefs(app).edit().putLong(KEY_LAST_CHECK, System.currentTimeMillis())
                current = if (release != null && release.versionCode > installed) {
                    edit.putString(KEY_AVAILABLE, release.version)
                    available(release.version)
                } else {
                    edit.remove(KEY_AVAILABLE)
                    UP_TO_DATE
                }
                edit.apply()
            } catch (e: Exception) {
                // A version found earlier stays installable.
                current = storedAvailable(app)?.let(::available) ?: failed(e)
            } finally {
                busy.set(false)
            }
        }
    }

    /**
     * Downloads, verifies and installs the newest release in the background.
     * Without permission to install apps it opens the system "Install unknown
     * apps" screen for RustTV instead; the user then presses OK again.
     * [context] is the activity, which starts that screen.
     */
    fun install(context: Context) {
        val app = context.applicationContext
        if (busy.get()) return
        if (!app.packageManager.canRequestPackageInstalls()) {
            val intent = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${app.packageName}"))
            current = try {
                context.startActivity(intent)
                "permission\t${storedAvailable(app) ?: ""}"
            } catch (e: ActivityNotFoundException) {
                failed("allow RustTV to install unknown apps in Settings")
            }
            return
        }
        if (!busy.compareAndSet(false, true)) return
        current = "downloading\t0"
        executor.execute {
            val apk = File(app.cacheDir, APK_FILE)
            try {
                // Read the feed again: the release may have changed since the check.
                val installed = installedVersionCode(app)
                val release = fetchRelease() ?: throw UpdateException("no release to install")
                if (release.versionCode <= installed) {
                    prefs(app).edit().remove(KEY_AVAILABLE).apply()
                    current = UP_TO_DATE
                    return@execute
                }
                val asset = UpdateFeed.apkFor(release, Build.SUPPORTED_ABIS)
                    ?: throw UpdateException("no APK for this TV in ${release.version}")
                val sumsAsset = UpdateFeed.sumsFor(release) ?: throw UpdateException("the release has no SHA256SUMS")
                val sums = readText(open(sumsAsset.url, "application/octet-stream"), MAX_SUMS_BYTES)
                val sha256 = UpdateFeed.sha256For(sums, asset.name)
                    ?: throw UpdateException("${asset.name} is not in SHA256SUMS")
                download(asset, apk, sha256)
                verifyArchive(app, apk, installed)
                current = INSTALLING
                commit(app, apk)
            } catch (e: Exception) {
                current = failed(e)
            } finally {
                // The session holds its own copy once written.
                apk.delete()
                busy.set(false)
            }
        }
    }

    /** From [InstallResultReceiver]: Android finished the install; [error] is null on success. */
    internal fun installFinished(context: Context, error: String?) {
        if (error == null) {
            prefs(context).edit().remove(KEY_AVAILABLE).apply()
            current = INSTALLED
        } else {
            current = failed("Android refused it: $error")
        }
    }

    /** From [InstallResultReceiver]: the user declined; the update stays available. */
    internal fun installCancelled(context: Context) {
        current = storedAvailable(context)?.let(::available) ?: IDLE
    }

    private fun initialStatus(context: Context): String {
        storedAvailable(context)?.let { return available(it) }
        return if (prefs(context).getLong(KEY_LAST_CHECK, 0) > 0) UP_TO_DATE else IDLE
    }

    /** The newer version found by the last check, if it is still newer than the installed app. */
    private fun storedAvailable(context: Context): String? {
        val version = prefs(context).getString(KEY_AVAILABLE, null) ?: return null
        val code = UpdateFeed.versionCode("v$version") ?: return null
        return version.takeIf { code > installedVersionCode(context) }
    }

    private fun prefs(context: Context): SharedPreferences =
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun available(version: String) = "available\t$version"

    private fun failed(message: String) = "failed\t" + message.replace(Regex("""\s+"""), " ").take(80)

    private fun failed(error: Exception): String = failed(
        when (error) {
            is UpdateException -> error.message ?: "unknown error"
            is UnknownHostException -> "no connection to GitHub"
            is SocketTimeoutException -> "GitHub didn't answer in time"
            is JSONException -> "unexpected answer from GitHub"
            is IOException -> "download failed (${error.javaClass.simpleName})"
            else -> error.javaClass.simpleName
        }
    )

    /**
     * The latest release, or null if it isn't installable (draft, pre-release,
     * odd tag) or there is none yet (GitHub answers 404 before the first release).
     */
    private fun fetchRelease(): UpdateFeed.Release? {
        val connection = try {
            open(FEED, "application/vnd.github+json")
        } catch (e: NoRelease) {
            return null
        }
        return UpdateFeed.parseRelease(readText(connection, MAX_FEED_BYTES))
    }

    /**
     * Opens [url] for reading, following redirects by hand so that every hop
     * is checked against [UpdateFeed.isAllowedUrl] (Java follows them blindly).
     */
    private fun open(url: String, accept: String): HttpsURLConnection {
        var target = url
        repeat(MAX_REDIRECTS + 1) {
            if (!UpdateFeed.isAllowedUrl(target)) throw UpdateException("refused to download from ${hostOf(target)}")
            val connection = URL(target).openConnection() as HttpsURLConnection
            connection.instanceFollowRedirects = false
            connection.connectTimeout = CONNECT_TIMEOUT_MS
            connection.readTimeout = READ_TIMEOUT_MS
            connection.setRequestProperty("Accept", accept)
            // GitHub's API requires a User-Agent; this one says nothing about the TV.
            connection.setRequestProperty("User-Agent", "RustTV-Launcher")
            val code = connection.responseCode
            if (code in 300..399) {
                val location = connection.getHeaderField("Location")
                discard(connection)
                location ?: throw UpdateException("GitHub redirected nowhere")
                target = URL(URL(target), location).toString()
                return@repeat
            }
            if (code != HttpsURLConnection.HTTP_OK) {
                discard(connection)
                if (code == HttpsURLConnection.HTTP_NOT_FOUND && url == FEED) throw NoRelease()
                throw UpdateException("GitHub answered $code")
            }
            return connection
        }
        throw UpdateException("too many redirects")
    }

    /**
     * Closes a response we don't read. Its body (often gzip-compressed) must
     * be closed too: `disconnect` alone leaves the decompressor to the
     * garbage collector ("A resource failed to call end", seen on the TV).
     */
    private fun discard(connection: HttpsURLConnection) {
        try {
            // Android serves a body of 400+ only as errorStream, others as inputStream.
            val body = if (connection.responseCode >= 400) connection.errorStream else connection.inputStream
            body?.close()
        } catch (e: IOException) {
            // Closing anyway; nothing to report.
        }
        connection.disconnect()
    }

    private fun hostOf(url: String): String = try {
        URL(url).host
    } catch (e: IOException) {
        "an unexpected address"
    }

    private fun readText(connection: HttpsURLConnection, cap: Int): String {
        try {
            if (connection.contentLengthLong > cap) throw UpdateException("answer too large")
            val out = ByteArrayOutputStream()
            connection.inputStream.use { input ->
                copy(input, cap.toLong()) { buffer, count -> out.write(buffer, 0, count) }
            }
            return out.toString("UTF-8")
        } finally {
            connection.disconnect()
        }
    }

    /** Downloads [asset] into [file], checking its size and SHA-256; reports progress in [current]. */
    private fun download(asset: UpdateFeed.Asset, file: File, sha256: String) {
        if (asset.size <= 0 || asset.size > MAX_APK_BYTES) throw UpdateException("unexpected APK size")
        val connection = open(asset.url, "application/octet-stream")
        val digest = MessageDigest.getInstance("SHA-256")
        var total = 0L
        var percent = 0
        try {
            file.outputStream().use { out ->
                connection.inputStream.use { input ->
                    copy(input, asset.size) { buffer, count ->
                        out.write(buffer, 0, count)
                        digest.update(buffer, 0, count)
                        total += count
                        val now = (total * 100 / asset.size).toInt()
                        if (now != percent) {
                            percent = now
                            current = "downloading\t$now"
                        }
                    }
                }
            }
        } finally {
            connection.disconnect()
        }
        if (total != asset.size) throw UpdateException("the download is incomplete")
        val actual = digest.digest().joinToString("") { "%02x".format(it) }
        if (actual != sha256) throw UpdateException("the APK doesn't match SHA256SUMS")
    }

    /** Copies [input] to [sink] in chunks, failing past [cap] bytes. */
    private inline fun copy(input: InputStream, cap: Long, sink: (ByteArray, Int) -> Unit) {
        val buffer = ByteArray(64 shl 10)
        var total = 0L
        while (true) {
            val count = input.read(buffer)
            if (count < 0) return
            total += count
            if (total > cap) throw UpdateException("download larger than expected")
            sink(buffer, count)
        }
    }

    /**
     * The APK must be RustTV's launcher, newer than the installed one, and
     * signed with the same certificates. Android checks the signature again
     * on install; this catches a wrong file before the user is asked.
     */
    private fun verifyArchive(context: Context, apk: File, installedVersion: Long) {
        val pm = context.packageManager
        val archive = archiveInfo(pm, apk) ?: throw UpdateException("the download is not an APK")
        if (archive.packageName != context.packageName) throw UpdateException("the APK is for another app")
        if (longVersionCode(archive) <= installedVersion) throw UpdateException("the APK is not newer")
        val expected = signers(installedInfo(pm, context.packageName, SIGNING_FLAGS))
        val actual = signers(archive)
        if (expected.isEmpty() || actual != expected) throw UpdateException("the APK is not signed with RustTV's key")
    }

    /** Writes [apk] into an install session and commits it; [InstallResultReceiver] gets the outcome. */
    private fun commit(context: Context, apk: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(apk.length())
        }
        val id = installer.createSession(params)
        var committed = false
        try {
            installer.openSession(id).use { session ->
                session.openWrite("base.apk", 0, apk.length()).use { out ->
                    apk.inputStream().use { it.copyTo(out) }
                    session.fsync(out)
                }
                // Explicit and not exported: only this app's own PendingIntent
                // reaches the receiver. Mutable, because the installer adds the
                // status extras (required to be explicit when mutable on API 34).
                val intent = Intent(context, InstallResultReceiver::class.java)
                val mutable = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
                val pending = PendingIntent.getBroadcast(context, id, intent, PendingIntent.FLAG_UPDATE_CURRENT or mutable)
                session.commit(pending.intentSender)
                committed = true
            }
        } catch (e: Exception) {
            if (!committed) installer.abandonSession(id)
            throw e
        }
    }

    // GET_SIGNATURES as well, as a fallback: archives may lack signingInfo on some
    // Android 9 builds, and before Android 9 it is the only option.
    @Suppress("DEPRECATION")
    private val SIGNING_FLAGS = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
        PackageManager.GET_SIGNING_CERTIFICATES or PackageManager.GET_SIGNATURES
    } else {
        PackageManager.GET_SIGNATURES
    }

    @Suppress("DEPRECATION")
    private fun archiveInfo(pm: PackageManager, apk: File): PackageInfo? =
        pm.getPackageArchiveInfo(apk.path, SIGNING_FLAGS)

    @Suppress("DEPRECATION")
    private fun installedInfo(pm: PackageManager, name: String, flags: Int): PackageInfo = pm.getPackageInfo(name, flags)

    private fun installedVersionCode(context: Context): Long =
        longVersionCode(installedInfo(context.packageManager, context.packageName, 0))

    @Suppress("DEPRECATION")
    private fun longVersionCode(info: PackageInfo): Long =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) info.longVersionCode else info.versionCode.toLong()

    /**
     * The certificates the APK is signed with now (not past ones from key
     * rotation, which RustTV doesn't use; rotating would need this relaxed).
     */
    @Suppress("DEPRECATION")
    private fun signers(info: PackageInfo): Set<Signature> {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.signingInfo?.apkContentsSigners?.let { return it.toSet() }
        }
        return info.signatures?.toSet() ?: emptySet()
    }
}

/**
 * Receives the outcome of an install session from Android ([Updater.commit]).
 * Declared with android:exported="false": nothing outside this app can send to it.
 */
class InstallResultReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                // Android's own confirmation screen; the launcher is in front, so it may start it.
                val confirm = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_INTENT)
                }
                if (confirm == null) {
                    Updater.installFinished(context, "no confirmation screen")
                    return
                }
                try {
                    context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                } catch (e: ActivityNotFoundException) {
                    Updater.installFinished(context, "no confirmation screen")
                }
            }
            PackageInstaller.STATUS_SUCCESS -> Updater.installFinished(context, null)
            PackageInstaller.STATUS_FAILURE_ABORTED -> Updater.installCancelled(context)
            else -> Updater.installFinished(
                context,
                intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "status $status",
            )
        }
    }
}
