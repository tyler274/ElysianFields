plugins {
    id("com.android.application") version "8.8.0" apply false
    id("org.jetbrains.kotlin.android") version "2.1.10" apply false
}

// Clean-room Gradle application descriptor for Curie Mobile (`fields.elysian.mobile`).
// Pure Kotlin OS shims in `src/main/kotlin` carry zero Android SDK compile-time
// imports so `kotlinc` compiles and executes the EFSS/Keystore/HealthConnect/Config
// test suite directly in CI without requiring an Android SDK sysroot.
//
// Android SDK packaging target configuration:
// - namespace: fields.elysian.mobile
// - applicationId: fields.elysian.mobile
// - compileSdk: 35 / targetSdk: 35 / minSdk: 28
// - jniLibs: src/main/jniLibs (libelysian_mobile.so built via cargo-ndk)
// - allowBackup: false, usesCleartextTraffic: false (enforced in AndroidManifest.xml)
extra["elysianNamespace"] = "fields.elysian.mobile"
extra["elysianApplicationId"] = "fields.elysian.mobile"
extra["elysianMinSdk"] = 28
extra["elysianTargetSdk"] = 35
extra["elysianCompileSdk"] = 35
extra["elysianVersionCode"] = 1
extra["elysianVersionName"] = "0.1.0"
