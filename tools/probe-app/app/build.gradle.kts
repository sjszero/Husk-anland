plugins { id("com.android.application") }
android {
    namespace = "com.linuxhub.probe"
    compileSdk = 36
    defaultConfig {
        applicationId = "com.linuxhub.probe"
        minSdk = 24
        targetSdk = 33
        versionCode = 2
        versionName = "0.2"
    }
    packaging { jniLibs { useLegacyPackaging = true } }
    buildTypes { getByName("debug") { isMinifyEnabled = false } }
}
