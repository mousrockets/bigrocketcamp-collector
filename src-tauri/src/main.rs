// No console window next to the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod rocket_league;

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

/// The Big Rocket Camp instance this build talks to; another instance rebuilds with its own APP_URL.
/// An empty APP_URL (an unset CI variable) counts as no APP_URL.
const APP_URL: &str = match option_env!("APP_URL") {
    Some(url) if !url.is_empty() => url,
    _ => "https://mousrockets.com",
};

const TITLE: &str = "Big Rocket Camp Direct";

/// wry's defaults (no mini menu, no SmartScreen on the club's own pages), plus Local Network Access:
/// the https page must reach the game on ws://localhost:49124 without a prompt the player could miss.
/// WebView2 keeps these checks off by default; disabling them by name keeps it that way if that changes.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,\
msWebViewAllowLocalNetworkAccessChecks,LocalNetworkAccessChecks,LocalNetworkAccessChecksWebSockets";

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let site = Url::parse(APP_URL)?;
            let page = site.join("/direct")?;
            let navigation_app = app.handle().clone();
            let popup_app = app.handle().clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(page))
                .title(TITLE)
                .inner_size(480.0, 760.0)
                .min_inner_size(380.0, 480.0)
                .additional_browser_args(BROWSER_ARGS)
                // The window only ever shows the club's site; any other link opens in the usual browser.
                .on_navigation(move |url| stays_inside(&site, url) || open_outside(&navigation_app, url))
                .on_new_window(move |url, _features| {
                    open_outside(&popup_app, &url);
                    NewWindowResponse::Deny
                })
                .build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || report_game_setup(&handle));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the window could not start");
}

fn stays_inside(site: &Url, url: &Url) -> bool {
    url.origin() == site.origin() || url.scheme() == "about"
}

/// Hands a link to the default browser and keeps this window where it is.
fn open_outside(app: &AppHandle, url: &Url) -> bool {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
    false
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Speaks only when something changed or needs the player; a game already set up stays silent.
fn report_game_setup(app: &AppHandle) {
    let mut changed = Vec::new();
    let mut failed = Vec::new();
    for install in rocket_league::installs() {
        match rocket_league::configure(&install) {
            Ok(rocket_league::Outcome::Updated(file)) => changed.push(file.display().to_string()),
            Ok(rocket_league::Outcome::AlreadySet | rocket_league::Outcome::NoConfigFile) => {}
            Err(error) => failed.push(format!("{} : {error}", install.display())),
        }
    }
    if !changed.is_empty() {
        app.dialog()
            .message(format!(
                "Rocket League est maintenant réglé pour envoyer les tableaux de fin de match.\n\n\
                 Si le jeu est ouvert, relance-le.\n\n\
                 Fichier modifié (une copie .bak est gardée à côté) :\n{}",
                changed.join("\n")
            ))
            .title(TITLE)
            .kind(MessageDialogKind::Info)
            .blocking_show();
    }
    if !failed.is_empty() {
        app.dialog()
            .message(format!(
                "Impossible de régler Rocket League automatiquement :\n{}\n\n\
                 Suis les étapes « À régler une seule fois sur le PC » dans la fenêtre.",
                failed.join("\n")
            ))
            .title(TITLE)
            .kind(MessageDialogKind::Warning)
            .blocking_show();
    }
}
