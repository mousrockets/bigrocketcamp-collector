//! The small window and the notification-area icon. Everything runs from Windows messages: nothing ticks.

use super::{APP_ID, set_start_with_windows, starts_with_windows, wide};
use crate::service::{Service, clean_code};
use crate::status::{Light, View, view};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, CreateFontIndirectW, CreateSolidBrush, DT_END_ELLIPSIS,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteObject, DrawTextW, Ellipse, EndPaint, FillRect,
    GetStockObject, GetSysColor, GetSysColorBrush, HBRUSH, HDC, HFONT, InvalidateRect, NULL_PEN, PAINTSTRUCT,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_SETCUEBANNER, EM_SETLIMITTEXT, LIM_SMALL, LoadIconMetric};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow, SystemParametersInfoForDpi,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, ES_AUTOHSCROLL, ES_CENTER,
    ES_UPPERCASE, FindWindowW, GetMessageW, GetWindowTextW, HICON, HMENU, IDC_ARROW, IDOK, IsDialogMessageW, IsIconic,
    LoadCursorW, LoadIconW, MB_ICONINFORMATION, MB_OK, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, MessageBoxW,
    NONCLIENTMETRICSW, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW,
    SPI_GETNONCLIENTMETRICS, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
    TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_DPICHANGED, WM_NULL, WM_PAINT,
    WM_SETFONT, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};

const TITLE: &str = "Big Rocket Camp Collector";
const WM_TRAY: u32 = WM_APP + 1;
const WM_STATUS: u32 = WM_APP + 2;
const WM_SHOW: u32 = WM_APP + 3;
/// Sent by the installer and uninstaller to close a running collector.
const WM_QUIT_COLLECTOR: u32 = WM_APP + 16;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

const ID_PAIR: i32 = IDOK;
const ID_HELP: i32 = 102;
const ID_CODE: i32 = 100;
const MENU_OPEN: usize = 1;
const MENU_HELP: usize = 2;
const MENU_STARTUP: usize = 3;
const MENU_QUIT: usize = 4;

const ICON_APP: u16 = 1;
const ICON_LIVE: u16 = 2;
const ICON_WAITING: u16 = 3;
const ICON_PROBLEM: u16 = 4;

const STYLE: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
const WIDTH: i32 = 340;
const HEIGHT: i32 = 168;

const HELP: &str = "Big Rocket Camp Collector envoie tes matchs Rocket League au club, tout seul.\n\n\
1. Associer ce PC : dans Big Rocket Camp, ouvre ton profil, clique « Associer le collecteur », \
puis recopie ici le code à 8 caractères.\n\n\
2. Régler Rocket League : le collecteur le fait au lancement. S’il n’y arrive pas, ferme le jeu, ouvre son dossier \
d’installation puis TAGame\\Config\\TAStatsAPI.ini (ou DefaultStatsAPI.ini s’il n’existe pas). Dans la section \
[TAGame.MatchStatsExporter_TA], mets PacketSendRate=10, Port=0 et WebPort=49124, enregistre et relance le jeu.\n\n\
3. Jouer : pastille verte, les matchs partent à la fin. Grise : le jeu n’est pas lancé. Rouge : lis le message.\n\n\
La croix range le collecteur près de l’horloge. Clic droit sur son icône pour le quitter ou le lancer avec Windows.";

/// The window, for the background threads that only post it a message.
static WINDOW: AtomicIsize = AtomicIsize::new(0);

struct Ui {
    service: Arc<Service>,
    window: HWND,
    code: HWND,
    pair: HWND,
    help: HWND,
    font: HFONT,
    title_font: HFONT,
    dpi: u32,
    icons: [HICON; 3],
    light: Option<Light>,
    needs_pairing: bool,
    taskbar_created: u32,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// Runs `action` on the window state; never while another handler holds it (a modal box re-enters the procedure).
fn with_ui<R>(action: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| cell.try_borrow_mut().ok().and_then(|mut ui| ui.as_mut().map(action)))
}

fn scale(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

fn resource(id: u16) -> *const u16 {
    id as usize as *const u16
}

/// Asks the collector already running to show itself.
pub fn show_running_collector() {
    let class = wide(APP_ID);
    // SAFETY: a NUL-terminated class name; posting to a window of another process is allowed.
    unsafe {
        let window = FindWindowW(class.as_ptr(), ptr::null());
        if !window.is_null() {
            PostMessageW(window, WM_SHOW, 0, 0);
        }
    }
}

/// The notification for background threads: cheap, and safe from any thread.
pub fn notify() {
    let window = WINDOW.load(Ordering::Acquire);
    if window != 0 {
        // SAFETY: posting a message never touches the window's memory.
        unsafe { PostMessageW(window as HWND, WM_STATUS, 0, 0) };
    }
}

pub fn run(service: Arc<Service>, hidden: bool) {
    // SAFETY: plain Win32 calls on this thread's own window; every string outlives the call that reads it.
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let class = wide(APP_ID);
        let registration = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hIcon: LoadIconW(instance, resource(ICON_APP)),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&registration);
        let dpi = GetDpiForSystem();
        let (width, height) = outer_size(dpi);
        let title = wide(TITLE);
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            width,
            height,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if window.is_null() {
            return;
        }
        let child = |class: &str, text: &str, style: u32, extended: u32, id: i32| {
            let (class, text) = (wide(class), wide(text));
            CreateWindowExW(
                extended,
                class.as_ptr(),
                text.as_ptr(),
                WS_CHILD | WS_TABSTOP | style,
                0,
                0,
                0,
                0,
                window,
                id as usize as HMENU,
                instance,
                ptr::null(),
            )
        };
        let code = child("EDIT", "", (ES_CENTER | ES_UPPERCASE | ES_AUTOHSCROLL) as u32, WS_EX_CLIENTEDGE, ID_CODE);
        let pair = child("BUTTON", "Associer", BS_DEFPUSHBUTTON as u32, 0, ID_PAIR);
        let help = child("BUTTON", "Aide", WS_VISIBLE | BS_PUSHBUTTON as u32, 0, ID_HELP);
        SendMessageW(code, EM_SETLIMITTEXT, 12, 0);
        let cue = wide("Code à 8 caractères");
        SendMessageW(code, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);

        let dpi = GetDpiForWindow(window);
        let (font, title_font) = fonts(dpi);
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        UI.with(|cell| {
            *cell.borrow_mut() = Some(Ui {
                service,
                window,
                code,
                pair,
                help,
                font,
                title_font,
                dpi,
                icons: [ICON_LIVE, ICON_WAITING, ICON_PROBLEM].map(small_icon),
                light: None,
                needs_pairing: false,
                taskbar_created,
            })
        });
        with_ui(|ui| {
            ui.apply_fonts();
            ui.layout();
            if dpi != GetDpiForSystem() {
                let (width, height) = outer_size(dpi);
                SetWindowPos(window, ptr::null_mut(), 0, 0, width, height, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE);
            }
        });
        WINDOW.store(window as isize, Ordering::Release);
        refresh(true);
        if !hidden {
            show(window);
        }

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

/// The notification-area size of an icon resource, as Windows recommends for this screen.
fn small_icon(id: u16) -> HICON {
    let mut icon: HICON = ptr::null_mut();
    // SAFETY: an icon resource of this executable; Windows writes the handle into `icon`.
    unsafe { LoadIconMetric(GetModuleHandleW(ptr::null()), resource(id), LIM_SMALL, &mut icon) };
    icon
}

fn outer_size(dpi: u32) -> (i32, i32) {
    let mut rect = RECT { left: 0, top: 0, right: scale(WIDTH, dpi), bottom: scale(HEIGHT, dpi) };
    // SAFETY: `rect` is a valid RECT for the call.
    unsafe { AdjustWindowRectExForDpi(&mut rect, STYLE, 0, 0, dpi) };
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// The system message font at this DPI, and a larger semibold one for the status.
fn fonts(dpi: u32) -> (HFONT, HFONT) {
    // SAFETY: NONCLIENTMETRICSW is plain data, sized as Windows expects.
    unsafe {
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast::<c_void>(),
            0,
            dpi,
        );
        let mut title = metrics.lfMessageFont;
        title.lfWeight = 600;
        title.lfHeight = title.lfHeight * 13 / 10;
        (CreateFontIndirectW(&metrics.lfMessageFont), CreateFontIndirectW(&title))
    }
}

fn show(window: HWND) {
    let code = with_ui(|ui| ui.needs_pairing.then_some(ui.code)).flatten();
    // SAFETY: windows owned by this thread.
    unsafe {
        ShowWindow(window, if IsIconic(window) != 0 { SW_RESTORE } else { SW_SHOW });
        SetForegroundWindow(window);
        if let Some(code) = code {
            SetFocus(code);
        }
    }
}

impl Ui {
    fn apply_fonts(&self) {
        for control in [self.code, self.pair, self.help] {
            // SAFETY: child windows of this thread, and a live font.
            unsafe { SendMessageW(control, WM_SETFONT, self.font as WPARAM, 1) };
        }
    }

    fn layout(&self) {
        let place = |control: HWND, x: i32, y: i32, width: i32, height: i32| {
            let d = self.dpi;
            // SAFETY: a child window of this thread.
            unsafe {
                SetWindowPos(
                    control,
                    ptr::null_mut(),
                    scale(x, d),
                    scale(y, d),
                    scale(width, d),
                    scale(height, d),
                    SWP_NOZORDER,
                );
            }
        };
        place(self.code, 52, 92, 150, 28);
        place(self.pair, 210, 92, 110, 28);
        place(self.help, 220, 130, 100, 28);
    }
}

/// Reads the status again and updates the icon, the controls and the drawing.
fn refresh(first: bool) {
    let Some((window, needs_pairing_before, status)) = with_ui(|ui| (ui.window, ui.needs_pairing, ui.service.status()))
    else {
        return;
    };
    let current = view(&status);
    let paired_now = needs_pairing_before && !status.needs_pairing && !first;
    with_ui(|ui| {
        ui.needs_pairing = status.needs_pairing;
        let pairing_row = if status.needs_pairing { SW_SHOW } else { SW_HIDE };
        // SAFETY: windows owned by this thread.
        unsafe {
            ShowWindow(ui.code, pairing_row);
            ShowWindow(ui.pair, pairing_row);
            EnableWindow(ui.code, (!status.pairing) as i32);
            EnableWindow(ui.pair, (!status.pairing) as i32);
            InvalidateRect(ui.window, ptr::null(), 1);
        }
        if ui.light != Some(current.light) {
            ui.tray(if ui.light.is_none() { NIM_ADD } else { NIM_MODIFY }, &current);
            ui.light = Some(current.light);
        } else {
            ui.tray(NIM_MODIFY, &current);
        }
    });
    if paired_now {
        with_ui(|ui| {
            // SAFETY: a child window of this thread.
            unsafe { SetWindowTextW(ui.code, wide("").as_ptr()) };
            let settings = ui.service.settings();
            if !settings.autostart_chosen {
                set_start_with_windows(true);
                ui.service.change_settings(|settings| settings.autostart_chosen = true);
            }
        });
    }
    // A revoked pairing needs the player: the window comes back on its own.
    if status.needs_pairing && !needs_pairing_before && !first {
        show(window);
    }
}

impl Ui {
    fn tray(&self, message: u32, current: &View) {
        let icon = self.icons[match current.light {
            Light::Live => 0,
            Light::Waiting => 1,
            Light::Problem => 2,
        }];
        // SAFETY: NOTIFYICONDATAW is plain data; the icons live as long as the process.
        unsafe {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.window;
            data.uID = 1;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
            data.uCallbackMessage = WM_TRAY;
            data.hIcon = icon;
            let tip: Vec<u16> = format!("Big Rocket Camp · {}", current.title).encode_utf16().take(127).collect();
            data.szTip[..tip.len()].copy_from_slice(&tip);
            if Shell_NotifyIconW(message, &data) == 0 && message == NIM_MODIFY {
                Shell_NotifyIconW(NIM_ADD, &data);
            }
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
    }

    fn remove_tray(&self) {
        // SAFETY: identifies the icon added by `tray`.
        unsafe {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.window;
            data.uID = 1;
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

fn help(window: HWND) {
    let (text, title) = (wide(HELP), wide(TITLE));
    // SAFETY: NUL-terminated strings that outlive the modal box.
    unsafe { MessageBoxW(window, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

fn pair() {
    let Some((service, code)) = with_ui(|ui| {
        let mut buffer = [0u16; 32];
        // SAFETY: the buffer length is passed along.
        let length = unsafe { GetWindowTextW(ui.code, buffer.as_mut_ptr(), buffer.len() as i32) };
        (Arc::clone(&ui.service), String::from_utf16_lossy(&buffer[..length.max(0) as usize]))
    }) else {
        return;
    };
    match clean_code(&code) {
        Some(code) => service.pair(code),
        None => service.wrong_code(),
    }
}

fn menu(window: HWND, x: i32, y: i32) {
    let startup = starts_with_windows();
    // SAFETY: a menu created, shown and destroyed here; strings outlive the calls.
    let choice = unsafe {
        let menu = CreatePopupMenu();
        let items = [
            (MF_STRING, MENU_OPEN, "Ouvrir"),
            (MF_STRING, MENU_HELP, "Aide"),
            (MF_SEPARATOR, 0, ""),
            (MF_STRING | if startup { MF_CHECKED } else { 0 }, MENU_STARTUP, "Lancer au démarrage de Windows"),
            (MF_SEPARATOR, 0, ""),
            (MF_STRING, MENU_QUIT, "Quitter"),
        ];
        for (flags, id, text) in items {
            AppendMenuW(menu, flags, id, wide(text).as_ptr());
        }
        SetForegroundWindow(window);
        let choice = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, window, ptr::null()) as usize;
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
        choice
    };
    match choice {
        MENU_OPEN => show(window),
        MENU_HELP => help(window),
        MENU_STARTUP => {
            set_start_with_windows(!startup);
            with_ui(|ui| ui.service.change_settings(|settings| settings.autostart_chosen = true));
        }
        // SAFETY: this thread's own window.
        MENU_QUIT => unsafe {
            DestroyWindow(window);
        },
        _ => {}
    }
}

fn paint(window: HWND) {
    let Some((status, font, title_font, dpi)) = with_ui(|ui| (ui.service.status(), ui.font, ui.title_font, ui.dpi))
    else {
        return;
    };
    let current = view(&status);
    let color = match current.light {
        Light::Live => rgb(30, 170, 80),
        Light::Waiting => rgb(150, 150, 150),
        Light::Problem => rgb(215, 50, 50),
    };
    let d = |value: i32| scale(value, dpi);
    // SAFETY: drawing on this window's paint DC, restoring and freeing every object selected.
    unsafe {
        let mut paint: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(window, &mut paint);
        FillRect(dc, &paint.rcPaint, GetSysColorBrush(COLOR_WINDOW));
        let brush = CreateSolidBrush(color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        Ellipse(dc, d(20), d(21), d(38), d(39));
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(brush);

        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, GetSysColor(COLOR_WINDOWTEXT));
        let old_font = SelectObject(dc, title_font);
        text(
            dc,
            &current.title,
            RECT { left: d(52), top: d(14), right: d(320), bottom: d(46) },
            DT_SINGLELINE | DT_VCENTER,
        );
        SelectObject(dc, font);
        let detail_bottom = if status.needs_pairing { 88 } else { 124 };
        text(
            dc,
            &current.detail,
            RECT { left: d(52), top: d(48), right: d(320), bottom: d(detail_bottom) },
            DT_WORDBREAK,
        );
        if let (Some(player), false) = (&status.player, status.needs_pairing) {
            SetTextColor(dc, GetSysColor(COLOR_GRAYTEXT));
            let line = format!("Joueur : {player}");
            text(
                dc,
                &line,
                RECT { left: d(20), top: d(130), right: d(212), bottom: d(158) },
                DT_SINGLELINE | DT_VCENTER,
            );
        }
        SelectObject(dc, old_font);
        EndPaint(window, &paint);
    }
}

fn text(dc: HDC, value: &str, mut rect: RECT, format: u32) {
    let value: Vec<u16> = value.encode_utf16().collect();
    // SAFETY: the length is given, so no NUL terminator is needed.
    unsafe { DrawTextW(dc, value.as_ptr(), value.len() as i32, &mut rect, format | DT_NOPREFIX | DT_END_ELLIPSIS) };
}

fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16)
}

fn low_word(value: usize) -> i32 {
    (value & 0xffff) as u16 as i16 as i32
}

fn high_word(value: usize) -> i32 {
    ((value >> 16) & 0xffff) as u16 as i16 as i32
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_PAINT => paint(window),
        WM_STATUS => refresh(false),
        WM_SHOW => show(window),
        WM_TRAY => match lparam as u32 & 0xffff {
            NIN_SELECT | NIN_KEYSELECT => show(window),
            WM_CONTEXTMENU => menu(window, low_word(wparam), high_word(wparam)),
            _ => {}
        },
        WM_COMMAND => match low_word(wparam) {
            ID_PAIR => pair(),
            ID_HELP => help(window),
            _ => {}
        },
        // The cross hides the window; the collector keeps running next to the clock.
        WM_CLOSE => {
            // SAFETY: this thread's own window.
            unsafe { ShowWindow(window, SW_HIDE) };
        }
        WM_QUIT_COLLECTOR => {
            // SAFETY: this thread's own window.
            unsafe { DestroyWindow(window) };
        }
        WM_DPICHANGED => {
            let dpi = high_word(wparam) as u32;
            with_ui(|ui| {
                let old = [ui.font, ui.title_font];
                ui.dpi = dpi;
                (ui.font, ui.title_font) = fonts(dpi);
                ui.apply_fonts();
                ui.layout();
                // SAFETY: the controls now use the new fonts, so the old ones are no longer selected anywhere.
                for font in old {
                    unsafe { DeleteObject(font) };
                }
            });
            // SAFETY: Windows passes the suggested window rectangle in `lparam`.
            unsafe {
                let suggested = &*(lparam as *const RECT);
                let (width, height) = outer_size(dpi);
                SetWindowPos(
                    window,
                    ptr::null_mut(),
                    suggested.left,
                    suggested.top,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        WM_DESTROY => {
            WINDOW.store(0, Ordering::Release);
            with_ui(|ui| ui.remove_tray());
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
        }
        _ => {
            if with_ui(|ui| ui.taskbar_created == message).unwrap_or(false) {
                // Explorer restarted: the icon must be added again.
                with_ui(|ui| ui.light = None);
                refresh(true);
            }
            // SAFETY: default handling of every other message.
            return unsafe { DefWindowProcW(window, message, wparam, lparam) };
        }
    }
    0
}
