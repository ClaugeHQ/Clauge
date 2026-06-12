fn main() {
    // Weak-link CoreAudio so the process-tap symbols (e.g.
    // AudioHardwareCreateProcessTap, available since macOS 14.2) resolve to
    // NULL on older macOS instead of aborting at dyld load. The runtime gate
    // in shared/audio/system/macos.rs requires 14.4 (the tap description API
    // we use) and prevents the call on anything older.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,CoreAudio");
    }

    tauri_build::build()
}
