//! Embeds the icons, the manifest and the version details in the Windows executable.

fn main() {
    // A fork rebuilds the collector for its own instance by setting APP_URL at build time.
    println!("cargo:rerun-if-env-changed=APP_URL");
    println!("cargo:rerun-if-changed=assets/icons");
    println!("cargo:rerun-if-changed=assets/collector.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    // The ids match ICON_APP and TRAY_ICONS in src/win/ui.rs.
    resource
        .set_icon("assets/icons/app.ico")
        .set_icon_with_id("assets/icons/live.ico", "2")
        .set_icon_with_id("assets/icons/waiting.ico", "3")
        .set_icon_with_id("assets/icons/problem.ico", "4")
        .set_icon_with_id("assets/icons/live-update.ico", "5")
        .set_icon_with_id("assets/icons/waiting-update.ico", "6")
        .set_icon_with_id("assets/icons/problem-update.ico", "7")
        .set_manifest_file("assets/collector.manifest")
        .set("ProductName", "Big Rocket Camp Collector")
        .set("FileDescription", "Big Rocket Camp Collector")
        .set("LegalCopyright", "MIT");
    resource.compile().expect("the Windows resources could not be compiled");
}
