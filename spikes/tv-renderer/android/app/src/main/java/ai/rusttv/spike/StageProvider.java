package ai.rusttv.spike;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.os.Binder;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.util.Log;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.io.InputStream;
import java.security.MessageDigest;

/** Throwaway: measures staging bytes over `adb exec-in content write`. Keeps nothing. */
public final class StageProvider extends ContentProvider {
    private static final String TAG = "RustTVSpike";
    private static final long CAP = 4L << 20; // 4 MiB, the package cap
    private static final int SHELL_UID = 2000;

    @Override public boolean onCreate() { return true; }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        int uid = Binder.getCallingUid();
        if (uid != SHELL_UID) throw new SecurityException("shell only, not uid " + uid);
        if (!mode.startsWith("w")) throw new FileNotFoundException("write only");
        try {
            ParcelFileDescriptor[] pipe = ParcelFileDescriptor.createReliablePipe();
            new Thread(() -> drain(pipe[0])).start();
            return pipe[1];
        } catch (IOException e) {
            throw new FileNotFoundException(e.toString());
        }
    }

    private static void drain(ParcelFileDescriptor read) {
        long n = 0;
        boolean over = false;
        String error = "none";
        long t0 = SystemClock.elapsedRealtime();
        try {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            try (InputStream in = new ParcelFileDescriptor.AutoCloseInputStream(read)) {
                byte[] buf = new byte[65536];
                int k;
                while ((k = in.read(buf)) > 0) {
                    n += k;
                    if (n > CAP) { over = true; break; }
                    md.update(buf, 0, k);
                }
                try { read.checkError(); } catch (IOException e) { error = e.toString(); }
            }
            StringBuilder hex = new StringBuilder();
            for (byte b : md.digest()) hex.append(String.format("%02x", b));
            Log.i(TAG, "STAGE bytes=" + n + " over_cap=" + over + " writer_error=" + error
                + " sha256=" + (over ? "-" : hex) + " ms=" + (SystemClock.elapsedRealtime() - t0));
        } catch (Exception e) {
            Log.w(TAG, "STAGE failed after " + n + " bytes: " + e);
        }
    }

    @Override public Cursor query(Uri u, String[] p, String s, String[] a, String o) { return null; }
    @Override public String getType(Uri u) { return null; }
    @Override public Uri insert(Uri u, ContentValues v) { return null; }
    @Override public int delete(Uri u, String s, String[] a) { return 0; }
    @Override public int update(Uri u, ContentValues v, String s, String[] a) { return 0; }
}
