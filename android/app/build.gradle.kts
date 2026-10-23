import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// The app version is the Rust workspace version ([workspace.package] in
// Cargo.toml), so the APK, the CLI and the About screen always agree.
// versionCode = major * 10000 + minor * 100 + patch (0.1.0 -> 100).
val cargoVersion: String = run {
    val cargoToml = rootProject.file("../Cargo.toml").readText()
    val section = cargoToml.substringAfter("[workspace.package]")
    Regex("""(?m)^version\s*=\s*"([^"]+)"""").find(section)?.groupValues?.get(1)
        ?: throw GradleException("no version in [workspace.package] of Cargo.toml")
}
val cargoVersionCode: Int = run {
    val parts = Regex("""^(\d+)\.(\d+)\.(\d+)$""").find(cargoVersion)?.destructured?.toList()
        ?.map { it.toInt() }
        ?: throw GradleException("Cargo version $cargoVersion is not plain MAJOR.MINOR.PATCH")
    if (parts.drop(1).any { it > 99 }) {
        throw GradleException("minor and patch must be below 100 for versionCode: $cargoVersion")
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

// Release signing key (RELEASE.md "Signing"): from RUSTTV_KEYSTORE_FILE,
// RUSTTV_KEYSTORE_PASSWORD, RUSTTV_KEY_ALIAS and RUSTTV_KEY_PASSWORD, else from
// the git-ignored android/keystore.properties (storeFile, storePassword,
// keyAlias, keyPassword). With neither, release builds are signed with the
// debug key so local and CI builds still install. A partial setup fails the
// build rather than quietly falling back to the debug key.
val releaseSigning: Map<String, String>? = run {
    val props = Properties()
    val file = rootProject.file("keystore.properties")
    if (file.isFile) file.inputStream().use { props.load(it) }
    val values = mapOf(
        "storeFile" to (System.getenv("RUSTTV_KEYSTORE_FILE") ?: props.getProperty("storeFile")),
        "storePassword" to (System.getenv("RUSTTV_KEYSTORE_PASSWORD") ?: props.getProperty("storePassword")),
        "keyAlias" to (System.getenv("RUSTTV_KEY_ALIAS") ?: props.getProperty("keyAlias")),
        "keyPassword" to (System.getenv("RUSTTV_KEY_PASSWORD") ?: props.getProperty("keyPassword")),
    ).filterValues { !it.isNullOrEmpty() }.mapValues { it.value!! }
    when (values.size) {
        0 -> null
        4 -> values
        else -> throw GradleException(
            "release signing is partly configured; missing ${listOf("storeFile", "storePassword", "keyAlias", "keyPassword") - values.keys} (RELEASE.md)"
        )
    }
}

android {
    namespace = "ai.rusttv.launcher"
    compileSdk = 34

    defaultConfig {
        applicationId = "ai.rusttv.launcher"
        minSdk = 26
        targetSdk = 34
        versionCode = cargoVersionCode
        versionName = cargoVersion
    }

    signingConfigs {
        if (releaseSigning != null) {
            create("release") {
                // A relative storeFile is resolved against android/.
                storeFile = rootProject.file(releaseSigning.getValue("storeFile"))
                storePassword = releaseSigning.getValue("storePassword")
                keyAlias = releaseSigning.getValue("keyAlias")
                keyPassword = releaseSigning.getValue("keyPassword")
            }
        }
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
            signingConfig = signingConfigs.getByName(if (releaseSigning != null) "release" else "debug")
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
