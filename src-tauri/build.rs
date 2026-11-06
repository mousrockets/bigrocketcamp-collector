fn main() {
    // A fork rebuilds the app for its own instance by setting APP_URL at build time.
    println!("cargo:rerun-if-env-changed=APP_URL");
    tauri_build::build()
}
