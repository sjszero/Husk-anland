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
    packaging { jniLibs { useLegacyPackaging = true } }
    buildTypes { getByName("debug") { isMinifyEnabled = false } }
}
