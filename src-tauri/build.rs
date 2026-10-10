fn main() {
    // Release builds ship libpython beside the bundled standard library, in the resources
    // (`/usr/lib/<productName>/python/lib`, see scripts/python-runtime.mjs). On macOS it goes to
    // Contents/Frameworks, which tauri-build adds to the rpath; Windows looks beside the executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN/../lib/HSync/python/lib");
    }
    tauri_build::build();
}
