// No console window next to the collector on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Everything below main is used by the Windows build; other systems only run the tests.
#![cfg_attr(not(windows), allow(dead_code))]

mod api;
mod bridge;
mod capture;
mod game;
mod rocket_league;
mod service;
mod status;
mod store;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    if !win::first_instance() {
        win::ui::show_running_collector();
        return;
    }
    win::stay_out_of_the_way();
    let folder = store::data_dir();
    let _ = std::fs::create_dir_all(&folder);
    let vault = win::DpapiVault::new(folder.join("token.bin"));
    let token = vault.load();
    let settings_path = folder.join("settings.json");
    let player = store::Settings::load(&settings_path).player;
    let status = status::Status::new(rocket_league::set_up(), player, token.is_some());
    let hidden = status.starts_hidden();
    if win::starts_with_windows() {
        // Keeps the startup entry pointing at this executable after an update.
        win::set_start_with_windows(true);
    }
    let service = service::Service::new(status, token, settings_path, Box::new(vault), Box::new(win::ui::notify));
    service.start(folder.join("queue.json"));
    win::ui::run(service, hidden);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Big Rocket Camp Collector runs on Windows only.");
    std::process::exit(1);
}
