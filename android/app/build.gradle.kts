plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "ai.rusttv.launcher"
    compileSdk = 34

    defaultConfig {
        applicationId = "ai.rusttv.launcher"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.0.0"
    }

    // One APK per ABI (app-<abi>-release.apk) instead of a universal one: each
    // ABI's library carries its own copy of Skia. All share one versionCode,
    // which is fine for sideloading; a store that hosts several APKs of one
    // app needs a distinct versionCode per ABI (Phase 6). The ABI list must
    // match the ABIs build.sh builds (AGP forbids ndk.abiFilters with splits).
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "armeabi-v7a", "x86_64")
            isUniversalApk = false
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Until release signing exists (ROADMAP Phase 6), sign with the debug
            // key so local builds install.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

// Third-party notices written by scripts/licenses.sh (android/build.sh runs it),
// packed into every APK as assets/licenses/THIRD_PARTY_NOTICES.txt. A missing
// file fails the build, so no APK ships without them.
abstract class CopyNoticesTask : DefaultTask() {
    @get:InputFiles
    abstract val notices: ConfigurableFileCollection

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        val source = notices.singleFile
        if (!source.isFile) {
            throw GradleException("$source is missing: run scripts/licenses.sh first (android/build.sh does)")
        }
        source.copyTo(outputDir.file("licenses/THIRD_PARTY_NOTICES.txt").get().asFile, overwrite = true)
    }
}

val copyNotices = tasks.register<CopyNoticesTask>("copyThirdPartyNotices") {
    notices.from(rootProject.file("../THIRD_PARTY_NOTICES.txt"))
}

androidComponents {
    onVariants { variant ->
        variant.sources.assets?.addGeneratedSourceDirectory(copyNotices, CopyNoticesTask::outputDir)
    }
}
