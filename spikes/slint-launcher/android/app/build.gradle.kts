plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "ai.rusttv.spike"
    compileSdk = 34

    defaultConfig {
        applicationId = "ai.rusttv.spike"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.0.0-spike"
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Spike only: sign release builds with the debug key so they install.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}
