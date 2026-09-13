//! Ядро: скрытое окно-«сообщалка», иконка в трее, меню, настройки, лог.
//! Однопоточное GUI; опрос заряда — в отдельном потоке (worker), чтобы меню
//! и иконка никогда не замерзали на время WinRT/CfgMgr-запросов. Результаты
//! приходят в GUI-поток сообщением WM_REFRESHED.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_INFO, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIIF_WARNING, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW, PostQuitMessage,
    RegisterClassW, TranslateMessage, HCURSOR, HICON, HMENU, WNDCLASSW, WM_APP, WM_CONTEXTMENU,
    WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, CW_USEDEFAULT,
};
use winreg::enums::*;
use winreg::RegKey;

use crate::battery::{get_devices_with_battery, DeviceBattery};
use crate::icon::TrayIcon;
use crate::menu::{localized_device_name, low_battery_message, low_battery_title, run_menu, tooltip_empty, tray_tooltip, Language, MenuAction, Theme};

const APP_NAME: &str = "BtBatteryTray";
const SETTINGS_KEY: &str = r"Software\BtBatteryTray";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "BtBatteryTray";
const POLL_PERIOD: Duration = Duration::from_secs(60);
const LOW_BATTERY: u8 = 20;
const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESH: u32 = WM_APP + 2; // «обновить сейчас» (меню / левый клик)
const WM_REFRESHED: u32 = WM_APP + 3; // worker прислал свежие данные
const TRAY_ID: u32 = 1;

struct AppState {
    icon: TrayIcon,
    devices: Vec<DeviceBattery>,
    last_levels: HashMap<String, u8>,
}

// приложение: GUI-поток + worker; AppState трогается только GUI-потоком.
// HICON is an opaque GUI handle and never crosses the worker boundary.
unsafe impl Send for AppState {}

static STATE: Mutex<Option<AppState>> = Mutex::new(None);
static REFRESH_NOW: AtomicBool = AtomicBool::new(false);
static STOP_WORKER: AtomicBool = AtomicBool::new(false);
static LAST_DEVICES: Mutex<Vec<DeviceBattery>> = Mutex::new(Vec::new());
static SINGLE_INSTANCE: AtomicIsize = AtomicIsize::new(0);

// ---------- утилиты ----------

fn log_line(msg: &str) {
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        let dir = PathBuf::from(dir).join(APP_NAME);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("log.txt");
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > 200_000 {
                let _ = std::fs::write(&path, b""); // ponytail: truncate вместо ротации, лога хватит
            }
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write;
            let _ = writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
        }
    }
}

fn to_wide<const N: usize>(s: &str) -> [u16; N] {
    let mut buf = [0u16; N];
    let mut i = 0;
    for u in s.encode_utf16().take(N.saturating_sub(1)) {
        buf[i] = u;
        i += 1;
    }
    buf
}

// ---------- настройки (реестр) ----------

fn load_language() -> Language {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(SETTINGS_KEY)
        .and_then(|k| k.get_value::<String, _>("Language"))
        .map(|v| Language::from_registry(&v))
        .unwrap_or(Language::English)
}

fn save_language(language: Language) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(SETTINGS_KEY) {
        let _ = k.set_value("Language", &language.registry_value());
    }
}

fn load_theme() -> Theme {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(SETTINGS_KEY)
        .and_then(|k| k.get_value::<String, _>("Theme"))
        .map(|v| Theme::from_registry(&v))
        .unwrap_or(Theme::Dark)
}

fn save_theme(theme: Theme) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(SETTINGS_KEY) {
        let _ = k.set_value("Theme", &theme.registry_value());
    }
}

fn load_target() -> String {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(SETTINGS_KEY)
        .and_then(|k| k.get_value::<String, _>("TargetAddress"))
        .unwrap_or_default()
}

fn save_target(addr: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(SETTINGS_KEY) {
        if addr.is_empty() {
            let _ = k.delete_value("TargetAddress");
        } else {
            let _ = k.set_value("TargetAddress", &addr);
        }
    }
}

/// Имя цели (для меню, когда устройство отключено).
fn load_target_name() -> String {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(SETTINGS_KEY)
        .and_then(|k| k.get_value::<String, _>("TargetName"))
        .unwrap_or_default()
}

fn save_target_name(name: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(SETTINGS_KEY) {
        let _ = k.set_value("TargetName", &name);
    }
}

fn autostart_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

fn toggle_autostart() {
    let exe = std::env::current_exe().unwrap_or_default();
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) {
        if autostart_enabled() {
            let _ = k.delete_value(RUN_VALUE);
        } else {
            let _ = k.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()));
        }
    }
}

// ---------- состояние и обновление ----------

fn build_tooltip(devices: &[DeviceBattery], target: &str, language: Language) -> String {
    if devices.is_empty() {
        return tooltip_empty(language).to_string();
    }
    let mut ordered: Vec<&DeviceBattery> = devices.iter().collect();
    ordered.sort_by(|a, b| {
        let at = a.address.eq_ignore_ascii_case(target);
        let bt = b.address.eq_ignore_ascii_case(target);
        bt.cmp(&at)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let text = ordered
        .iter()
        .map(|d| format!("{}: {}%", localized_device_name(language, &d.name), d.level))
        .collect::<Vec<_>>()
        .join("\n");
    // лимит szTip: 128 UTF-16 единиц (последняя — терминатор). Резать можно только
    // по границам символов: text[..62] по БАЙТАМ паникует на кириллице (многобайтовые),
    // а panic=abort означает молчаливую смерть трея.
    if text.encode_utf16().count() > 63 {
        let mut acc = String::new();
        for ch in text.chars() {
            if acc.encode_utf16().count() + ch.len_utf16() > 62 {
                break;
            }
            acc.push(ch);
        }
        acc + "…"
    } else {
        text
    }
}

fn compute_shown(devices: &[DeviceBattery], target: &str) -> Option<u8> {
    if !target.is_empty() {
        if let Some(t) = devices.iter().find(|d| d.address.eq_ignore_ascii_case(target)) {
            return Some(t.level);
        }
    }
    devices.iter().map(|d| d.level).min()
}

fn update_tray_icon_and_tip(hwnd: HWND, hicon: HICON, tooltip: &str) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_ICON | NIF_TIP,
        uCallbackMessage: 0,
        hIcon: hicon,
        szTip: to_wide(tooltip),
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn show_balloon(hwnd: HWND, name: &str, level: u8, language: Language) {
    let info = low_battery_message(language, name, level);
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_INFO,
        uCallbackMessage: 0,
        hIcon: Default::default(),
        szTip: [0; 128],
        szInfo: to_wide(&info),
        szInfoTitle: to_wide(low_battery_title(language)),
        dwInfoFlags: NIIF_WARNING,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// GUI-поток: применил свежие данные от worker'а (иконка, тултип, баллун, лог).
fn ui_refresh(hwnd: HWND) {
    let devices = LAST_DEVICES.lock().unwrap().clone();
    let language = load_language();
    let target = load_target();
    let shown = compute_shown(&devices, &target);
    let tooltip = build_tooltip(&devices, &target, language);

    // запомнить имя цели (для меню, когда устройство отключено)
    if !target.is_empty() {
        if let Some(d) = devices.iter().find(|d| d.address.eq_ignore_ascii_case(&target)) {
            save_target_name(&d.name);
        }
    }

    let new_icon = match TrayIcon::create(shown) {
        Ok(i) => i,
        Err(e) => {
            log_line(&format!("icon error: {}", e));
            return;
        }
    };

    let mut guard = STATE.lock().unwrap();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    let old_icon = std::mem::replace(&mut state.icon, new_icon);
    state.devices = devices.clone();
    update_tray_icon_and_tip(hwnd, state.icon.hicon(), &tooltip);

    // баллун при падении ниже порога (только цель, если выбрана)
    let mut balloon: Option<(String, u8)> = None;
    for d in &devices {
        if d.level >= LOW_BATTERY {
            continue;
        }
        if !target.is_empty() && !d.address.eq_ignore_ascii_case(&target) {
            continue;
        }
        let prev = state.last_levels.get(&d.address).copied();
        if prev.map(|p| p < LOW_BATTERY).unwrap_or(false) {
            continue;
        }
        balloon = Some((d.name.clone(), d.level));
    }
    state.last_levels.clear();
    for d in &devices {
        state.last_levels.insert(d.address.clone(), d.level);
    }
    drop(guard);
    drop(old_icon); // старую иконку удаляем после обновления трея

    if let Some((name, level)) = balloon {
        show_balloon(hwnd, &name, level, language);
    }

    let summary = if devices.is_empty() {
        tooltip_empty(language).to_string()
    } else {
        devices
            .iter()
            .map(|d| format!("{}={}%", d.name, d.level))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let icon_info = match shown {
        None => match language {
            Language::English => "no data".to_string(),
            Language::Russian => "нет данных".to_string(),
            Language::Ukrainian => "немає даних".to_string(),
        },
        Some(v) => {
            let t = devices
                .iter()
                .find(|d| !target.is_empty() && d.address.eq_ignore_ascii_case(&target));
            match t {
                Some(d) => format!("{} {}={}%", match language {
                    Language::English => "target",
                    Language::Russian => "цель",
                    Language::Ukrainian => "ціль",
                }, d.name, v),
                None => format!("{} {}%", match language {
                    Language::English => "min",
                    Language::Russian => "мин",
                    Language::Ukrainian => "мін",
                }, v),
            }
        }
    };
    log_line(&format!("refresh ok: {} | иконка: {}", summary, icon_info));
}

// ---------- worker: опрос в отдельном потоке ----------

fn worker_loop(hwnd: HWND) {
    let mut last = Instant::now().checked_sub(POLL_PERIOD).unwrap_or_else(Instant::now);
    loop {
        if STOP_WORKER.load(Ordering::Acquire) {
            break;
        }
        if REFRESH_NOW.swap(false, Ordering::SeqCst) || last.elapsed() >= POLL_PERIOD {
            last = Instant::now();
            // Подключённость определяем прямо в CfgMgr32 (CM_Get_DevNode_Status),
            // а не через блокирующие WinRT FindAllAsync().get() — иначе каждый опрос
            // «зависает» на секунды и «Обновить сейчас» выглядит мёртвым.
            let devices = get_devices_with_battery(None);
            *LAST_DEVICES.lock().unwrap() = devices;
            if !STOP_WORKER.load(Ordering::Acquire) {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_REFRESHED, WPARAM(0), LPARAM(0));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

// ---------- меню ----------

fn on_tray_menu(hwnd: HWND) {
    let (devices, target, target_name, autostart, language, theme) = {
        let guard = STATE.lock().unwrap();
        let s = match guard.as_ref() {
            Some(s) => s,
            None => return,
        };
        (
            s.devices.clone(),
            load_target(),
            load_target_name(),
            autostart_enabled(),
            load_language(),
            load_theme(),
        )
    };

    let action = run_menu(&devices, &target, &target_name, autostart, language, theme);
    match action {
        MenuAction::Exit => {
            unsafe {
                PostQuitMessage(0);
            }
        }
        MenuAction::Refresh => {
            REFRESH_NOW.store(true, Ordering::SeqCst);
        }
        MenuAction::SetTarget(addr) => {
            save_target(&addr);
            REFRESH_NOW.store(true, Ordering::SeqCst);
        }
        MenuAction::SetLanguage(language) => {
            save_language(language);
            ui_refresh(hwnd);
        }
        MenuAction::SetTheme(theme) => {
            save_theme(theme);
            ui_refresh(hwnd);
        }
        MenuAction::ToggleAutostart => toggle_autostart(),
        MenuAction::None => {}
    }
}

// ---------- окно ----------

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let code = (lparam.0 & 0xFFFF) as u32;
            match code {
                // ЛКМ — как у обычных треевских приложений: открыть меню
                WM_LBUTTONUP | WM_RBUTTONUP | WM_CONTEXTMENU => on_tray_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_REFRESH => {
            REFRESH_NOW.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        WM_REFRESHED => {
            ui_refresh(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------- входные точки ----------

pub fn acquire_single_instance() -> bool {
    unsafe {
        let handle = match CreateMutexW(None, false, windows::core::w!("Local\\BtBatteryTray_SingleInstance")) {
            Ok(h) => h,
            Err(_) => return false,
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return false;
        }
        SINGLE_INSTANCE.store(handle.0 as isize, Ordering::Release);
        true
    }
}

pub fn run() {
    unsafe {
        let hinstance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => {
                log_line(&format!("GetModuleHandleW failed: {}", e));
                return;
            }
        };
        let class = windows::core::w!("BtTrayHidden");

        let wc = WNDCLASSW {
            style: Default::default(),
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: windows::core::PCWSTR::null(),
            lpszClassName: class,
        };
        RegisterClassW(&wc);

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            windows::core::w!("BtBatteryTray"),
            WINDOW_STYLE(0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            HWND::default(),
            HMENU::default(),
            windows::Win32::Foundation::HINSTANCE::from(hinstance),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                log_line(&format!("CreateWindowExW failed: {}", e));
                let handle = SINGLE_INSTANCE.swap(0, Ordering::AcqRel);
                if handle != 0 {
                    let _ = CloseHandle(HANDLE(handle as *mut core::ffi::c_void));
                }
                return;
            }
        };

        let icon = match TrayIcon::create(None) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("initial icon failed: {}", e);
                log_line(&format!("initial icon failed: {}", e));
                return;
            }
        };
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon.hicon(),
            szTip: to_wide(tray_tooltip(load_language())),
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);

        *STATE.lock().unwrap() = Some(AppState {
            icon,
            devices: Vec::new(),
            last_levels: HashMap::new(),
        });

        // worker опрашивает устройства; первый опрос — сразу
        // HWND не Send → передаём как usize (сырой адрес окна)
        STOP_WORKER.store(false, Ordering::Release);
        let hwnd_addr = hwnd.0 as usize;
        let worker = std::thread::spawn(move || worker_loop(HWND(hwnd_addr as *mut core::ffi::c_void)));
        REFRESH_NOW.store(true, Ordering::SeqCst);

        let mut msg = MSG {
            hwnd: HWND::default(),
            message: 0,
            wParam: WPARAM(0),
            lParam: LPARAM(0),
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        STOP_WORKER.store(true, Ordering::Release);
        let _ = worker.join();

        let del = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &del);
        *STATE.lock().unwrap() = None;
        let handle = SINGLE_INSTANCE.swap(0, Ordering::AcqRel);
        if handle != 0 {
            let _ = CloseHandle(HANDLE(handle as *mut core::ffi::c_void));
        }
    }
}
