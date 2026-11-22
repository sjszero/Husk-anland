plugins { id("com.android.application") }
android {
    namespace = "com.linuxhub.husk"
    compileSdk = 36
    defaultConfig {
        applicationId = "com.linuxhub.husk"
        minSdk = 24
        targetSdk = 33
        versionCode = 1
        versionName = "0.1"
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
