// Throwaway spike (PANEL_STUDIO.md § 15 step 1): its own applicationId and the
// debug key, so it can never update or replace the real launcher. No HOME filter.
plugins { id("com.android.application") }
android {
    namespace = "ai.rusttv.spike"
    compileSdk = 34
    defaultConfig {
        applicationId = "ai.rusttv.spike"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.0.0-spike"
    }
    buildTypes { getByName("debug") { isDebuggable = false } }
}
