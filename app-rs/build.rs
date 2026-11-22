fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("android") {
        for lib in ["log", "android", "EGL", "GLESv2", "dl"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
}
