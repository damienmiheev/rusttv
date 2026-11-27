package ai.rusttv.launcher

import android.content.ContentProvider
import android.content.ContentProviderOperation
import android.content.ContentProviderResult
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.Bundle
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.content.res.AssetFileDescriptor
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.ConcurrentHashMap

/**
 * The history door (HISTORY.md § 5): the Mac's `rusttv` reads exports and
 * pending approvals and commits inbox messages through it over adb. It also
 * takes one inert package into the staging slot (SECURITY.md "Packages:
 * staging and install", class RS): `stage begin`, a write, `stage end`.
 *
 * Every entry point answers only the adb shell: [checkShell] runs first in
 * each, and a test lists them. `onCreate` does nothing (it runs on Home's
 * cold-start path); the native library and the history open on first use.
 */
class HistoryProvider : ContentProvider() {
    override fun onCreate(): Boolean = true

    /**
     * Refuses every caller but the adb shell. 2000 is Android's SHELL_UID
     * (`android.os.Process.SHELL_UID` is hidden API); on user builds only the
     * shell runs as it.
     */
    private fun checkShell() {
        if (Binder.getCallingUid() != SHELL_UID) {
            throw SecurityException("RustTV's history answers only adb")
        }
    }

    private fun appContext() = context?.applicationContext
        ?: throw IllegalStateException("no context")

    private fun inbox(name: String): File {
        require(NONCE.matches(name)) { "bad inbox name" }
        return File(appContext().cacheDir, "history-inbox").apply { mkdirs() }.resolve(name)
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        checkShell()
        History.load()
        val segments = uri.pathSegments
        return when {
            mode == "r" && segments.size == 2 && segments[0] == "export" -> {
                val since = segments[1].takeIf { it != "all" }
                readOnly(History.nativeExport(appContext(), since))
            }
            mode == "r" && segments == listOf("pending") ->
                readOnly(History.nativePending(appContext()))
            mode.startsWith("w") && segments.size == 2 && segments[0] == "stage" ->
                stageWrite(segments[1])
            mode.startsWith("w") && segments.size == 2 && segments[0] == "inbox" ->
                // A real file, so the bytes are on disk once `content write` exits.
                ParcelFileDescriptor.open(
                    inbox(segments[1]),
                    ParcelFileDescriptor.MODE_WRITE_ONLY or
                        ParcelFileDescriptor.MODE_CREATE or
                        ParcelFileDescriptor.MODE_TRUNCATE,
                )
            else -> throw IllegalArgumentException("unknown history URI")
        }
    }

    /** Hands `bytes` out as a file that is already unlinked. */
    private fun readOnly(bytes: ByteArray): ParcelFileDescriptor {
        val file = File.createTempFile("history-out", null, appContext().cacheDir)
        try {
            file.writeBytes(bytes)
            return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        } finally {
            file.delete()
        }
    }

    /** The staging slot's folder (crates/rusttv-launcher/src/staging.rs). */
    private fun stagingDir(): String = File(appContext().filesDir, "staging").path

    /**
     * The write end of a pipe for `stage/<nonce>`: a thread copies at most
     * the announced size plus one byte into the slot's incoming file, so a
     * larger stream is cut off and `stage end` refuses it.
     */
    private fun stageWrite(nonce: String): ParcelFileDescriptor {
        require(NONCE.matches(nonce)) { "bad stage name" }
        val target = History.nativeStageIncoming(stagingDir(), nonce, History.bootCount(appContext()), History.elapsedMs())
        val path = target.substringBefore(' ', "")
        val cap = target.substringAfter(' ', "").toLongOrNull()
        require(path.isNotEmpty() && cap != null) { "no staging for that name" }
        val pipe = ParcelFileDescriptor.createPipe()
        val copy = Thread {
            ParcelFileDescriptor.AutoCloseInputStream(pipe[0]).use { input ->
                FileOutputStream(path).use { output ->
                    val buffer = ByteArray(64 * 1024)
                    var left = cap + 1
                    while (left > 0) {
                        val n = input.read(buffer, 0, minOf(buffer.size.toLong(), left).toInt())
                        if (n < 0) break
                        output.write(buffer, 0, n)
                        left -= n
                    }
                }
            }
        }
        copies[nonce] = copy
        copy.start()
        return pipe[1]
    }

    override fun call(method: String, arg: String?, extras: Bundle?): Bundle {
        checkShell()
        History.load()
        if (method == "stage" && arg != null) {
            // The copy must have finished before the bytes are checked.
            if (arg.startsWith("end:")) {
                copies.remove(arg.substringAfter(':'))?.join(COPY_WAIT_MS)
            }
            val result = History.nativeStage(stagingDir(), arg, History.bootCount(appContext()), History.elapsedMs())
            return Bundle().apply { putString("rusttv", result) }
        }
        require(method == "commit" && arg != null) { "unknown history call" }
        val nonce = arg.substringAfter(':', "")
        val file = inbox(nonce)
        val result = try {
            if (file.length() > MAX_INBOX_BYTES) {
                "rusttv-result 1 error HISTORY_INVALID none"
            } else {
                History.nativeCommit(appContext(), arg, file.readBytes())
            }
        } finally {
            file.delete()
        }
        return Bundle().apply { putString("rusttv", result) }
    }

    override fun getType(uri: Uri): String = "application/octet-stream"

    override fun query(uri: Uri, projection: Array<String>?, selection: String?, selectionArgs: Array<String>?, sortOrder: String?): Cursor? {
        checkShell()
        return null
    }

    override fun query(uri: Uri, projection: Array<String>?, queryArgs: Bundle?, cancellationSignal: CancellationSignal?): Cursor? {
        checkShell()
        return null
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? {
        checkShell()
        return null
    }

    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<String>?): Int {
        checkShell()
        return 0
    }

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<String>?): Int {
        checkShell()
        return 0
    }

    override fun bulkInsert(uri: Uri, values: Array<ContentValues>): Int {
        checkShell()
        return 0
    }

    override fun applyBatch(operations: ArrayList<ContentProviderOperation>): Array<ContentProviderResult> {
        checkShell()
        throw UnsupportedOperationException()
    }

    override fun openAssetFile(uri: Uri, mode: String): AssetFileDescriptor? {
        checkShell()
        return super.openAssetFile(uri, mode)
    }

    override fun openTypedAssetFile(uri: Uri, mimeTypeFilter: String, opts: Bundle?): AssetFileDescriptor? {
        checkShell()
        return super.openTypedAssetFile(uri, mimeTypeFilter, opts)
    }

    override fun canonicalize(url: Uri): Uri? {
        checkShell()
        return null
    }

    override fun uncanonicalize(url: Uri): Uri? {
        checkShell()
        return null
    }

    override fun refresh(uri: Uri, extras: Bundle?, cancellationSignal: CancellationSignal?): Boolean {
        checkShell()
        return false
    }

    private companion object {
        const val SHELL_UID = 2000
        val NONCE = Regex("^[0-9a-f]{16}$")
        /** An export of a whole history plus its header (HISTORY.md § 3 limits). */
        const val MAX_INBOX_BYTES = 64L * 1024 * 1024 + 4096
        /** How long `stage end` waits for the bytes still in the pipe. */
        const val COPY_WAIT_MS = 10_000L
        /** The copy threads of stage writes, by nonce. */
        val copies = ConcurrentHashMap<String, Thread>()
    }
}
