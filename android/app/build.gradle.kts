plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// La versión sale de Cargo.toml para que el PC y la app vayan a la par.
val reflexVersion: String = rootProject.file("../Cargo.toml").readLines()
    .first { it.startsWith("version") }.substringAfter('"').substringBefore('"')
val versionParts = reflexVersion.split('.').map { it.toInt() }

android {
    namespace = "io.github.anxo0.reflex"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.github.anxo0.reflex"
        minSdk = 26
        targetSdk = 35
        versionCode = versionParts[0] * 10000 + versionParts[1] * 100 + versionParts[2]
        versionName = reflexVersion
    }

    // Firma: la keystore llega por variables de entorno (en GitHub, desde los secretos).
    val keystore = System.getenv("REFLEX_KEYSTORE")
    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("REFLEX_KEYSTORE_PASS")
                keyAlias = System.getenv("REFLEX_KEY_ALIAS")
                keyPassword = System.getenv("REFLEX_KEY_PASS")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (keystore != null) signingConfig = signingConfigs.getByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}
