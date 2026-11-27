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

// 三块原生件都是构建产物（src/main/jniLibs/ 在 .gitignore 里，干净 clone 没有），所以
// 只跑 `./gradlew :app:assembleDebug` 会打出一个"能装、能启动、但起不了 guest"的空壳：
// hasCode=false 又没有 libhuskapp.so 就是没有任何代码，没有 libld.so 就是没有加载器。
// `make apk` 会先构建它们；这个守卫让直接走 Gradle 的路径当场失败，而不是安静地发空壳。
// 判据（第 2 轮）：干净 clone 只跑 gradlew 必须红。
val huskNativePieces = listOf("libhuskapp.so", "libhuskguard.so", "libld.so")
val huskForbiddenNativePieces = listOf("libXlorie.so")
val huskVerifyNativePieces = tasks.register("huskVerifyNativePieces") {
    val dir = layout.projectDirectory.dir("src/main/jniLibs/arm64-v8a")
    doLast {
        val missing = huskNativePieces.filter { !dir.file(it).asFile.isFile }
        if (missing.isNotEmpty()) {
            throw GradleException(
                "Husk: 缺少原生件 " + missing.joinToString(", ") + " （目录 " + dir.asFile + "）。" +
                "这三块是构建产物，不进版本库：请用 `make apk`（或先 `make loader` + " +
                "cargo build --release --manifest-path app-rs/Cargo.toml --target aarch64-linux-android），" +
                "不要直接调 Gradle。空壳 APK 装了也起不了 guest。"
            )
        }
        val forbidden = huskForbiddenNativePieces.filter { dir.file(it).asFile.isFile }
        if (forbidden.isNotEmpty()) {
            throw GradleException(
                "Husk: 检测到宿主 UI 原生件 " + forbidden.joinToString(", ") + "。" +
                "Husk 只发运行时；X server/桌面由宿主 App 提供。先运行 `make apk` 清理陈旧 jniLibs。"
            )
        }
    }
}
tasks.named("preBuild").configure { dependsOn(huskVerifyNativePieces) }
