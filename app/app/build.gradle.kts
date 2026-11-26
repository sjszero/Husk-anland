plugins { id("com.android.application") }
val huskVersion = rootProject.file("../VERSION").readText().trim()
val huskApplicationId = providers.gradleProperty("huskApplicationId").orElse("com.linuxhub.husk")
val huskVersionCode = huskVersion.split('.').fold(0) { value, part -> value * 100 + part.toInt() }
android {
    namespace = "com.linuxhub.husk"
    compileSdk = 36
    defaultConfig {
        applicationId = huskApplicationId.get()
        minSdk = 24
        targetSdk = 33
        versionCode = huskVersionCode
        versionName = huskVersion
    }
    packaging {
        jniLibs {
            useLegacyPackaging = true
            // libld.so is a patched guest ELF, not a disposable Android native library.
            // AGP must package it byte-for-byte so the loader hash and evidence remain stable.
            keepDebugSymbols += "**/libld.so"
        }
    }
    buildTypes { getByName("debug") { isMinifyEnabled = false } }
}
