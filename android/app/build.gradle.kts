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
        // Must match the ABIs build.sh builds.
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
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
