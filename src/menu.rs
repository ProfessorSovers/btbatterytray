//! Тёмное попап-меню (порт тёмной темы из WinForms-версии).
//! Собственное окно Win32: WS_POPUP + WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE|WS_EX_TOPMOST,
//! рисуется вручную в WM_PAINT (фон, рамка, пункты, radio/check).
//! run_menu() — модальный вложенный цикл сообщений до закрытия.
//! Пункт «Цель» открывает вложенное попап-меню (отдельное окно) с выбором цели.
//! render_test() — отрисовка тех же пунктов в menu_test.bmp без окна (общий код отрисовки).
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
    RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
    CreateFontW, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DIB_RGB_COLORS, DrawTextW,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, Ellipse, EndPaint, FillRect, GetDC, GetStockObject,
    GetTextExtentPoint32W, HBRUSH, HDC, HFONT, InvalidateRect, LineTo, MoveToEx, NULL_BRUSH,
    PAINTSTRUCT, Polygon, PS_SOLID, Rectangle, ReleaseDC, RGBQUAD, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GWLP_USERDATA, HCURSOR, HHOOK, HICON, HMENU,
    IDC_ARROW, KillTimer, LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW,
    RegisterClassW, SetCursor, SetTimer, SetWindowsHookExW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNA,
    TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL,
    WindowFromPoint, WNDCLASSW, WM_ACTIVATE, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::battery::DeviceBattery;

#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    None,
    Refresh,
    SetTarget(String), // адрес или "" (авто)
    SetLanguage(Language),
    SetTheme(Theme),
    ToggleAutostart,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Russian,
    Ukrainian,
}

impl Language {
    pub fn from_registry(value: &str) -> Self {
        match value {
            "ru" => Self::Russian,
            "uk" => Self::Ukrainian,
            _ => Self::English,
        }
    }

    pub fn registry_value(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Russian => "ru",
            Self::Ukrainian => "uk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn from_registry(value: &str) -> Self {
        if value == "light" {
            Self::Light
        } else {
            Self::Dark
        }
    }

    pub fn registry_value(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmenuKind {
    Target,
    Language,
    Theme,
}

fn fallback_device_name(language: Language) -> &'static str {
    match language {
        Language::English => "Bluetooth device",
        Language::Russian => "Bluetooth-устройство",
        Language::Ukrainian => "Bluetooth-пристрій",
    }
}

fn device_name(language: Language, name: &str) -> &str {
    match name {
        "Bluetooth device" | "Bluetooth-устройство" | "Bluetooth-пристрій" => fallback_device_name(language),
        _ => name,
    }
}

pub fn localized_device_name(language: Language, name: &str) -> &str {
    device_name(language, name)
}

fn text(language: Language, key: &str) -> &'static str {
    match (language, key) {
        (Language::English, "updated") => "Updated",
        (Language::Russian, "updated") => "Обновлено",
        (Language::Ukrainian, "updated") => "Оновлено",
        (Language::English, "no_devices") => "No connected devices",
        (Language::Russian, "no_devices") => "Подключённых устройств нет",
        (Language::Ukrainian, "no_devices") => "Підключених пристроїв немає",
        (Language::English, "target") => "Target",
        (Language::Russian, "target") => "Цель",
        (Language::Ukrainian, "target") => "Ціль",
        (Language::English, "auto") => "Auto (lowest battery)",
        (Language::Russian, "auto") => "Авто (самое разряженное)",
        (Language::Ukrainian, "auto") => "Авто (найменший заряд)",
        (Language::English, "not_connected") => "not connected",
        (Language::Russian, "not_connected") => "не подключено",
        (Language::Ukrainian, "not_connected") => "не підключено",
        (Language::English, "refresh") => "Refresh now",
        (Language::Russian, "refresh") => "Обновить сейчас",
        (Language::Ukrainian, "refresh") => "Оновити зараз",
        (Language::English, "autostart") => "Start with Windows",
        (Language::Russian, "autostart") => "Автозапуск",
        (Language::Ukrainian, "autostart") => "Автозапуск",
        (Language::English, "language") => "Language",
        (Language::Russian, "language") => "Язык",
        (Language::Ukrainian, "language") => "Мова",
        (Language::English, "theme") => "Theme",
        (Language::Russian, "theme") => "Тема",
        (Language::Ukrainian, "theme") => "Тема",
        (Language::English, "english") => "English",
        (Language::Russian, "english") => "Английский",
        (Language::Ukrainian, "english") => "Англійська",
        (Language::English, "russian") => "Russian",
        (Language::Russian, "russian") => "Русский",
        (Language::Ukrainian, "russian") => "Російська",
        (Language::English, "ukrainian") => "Ukrainian",
        (Language::Russian, "ukrainian") => "Украинский",
        (Language::Ukrainian, "ukrainian") => "Українська",
        (Language::English, "dark") => "Dark",
        (Language::Russian, "dark") => "Тёмная",
        (Language::Ukrainian, "dark") => "Темна",
        (Language::English, "light") => "Light",
        (Language::Russian, "light") => "Светлая",
        (Language::Ukrainian, "light") => "Світла",
        (Language::English, "exit") => "Exit",
        (Language::Russian, "exit") => "Выход",
        (Language::Ukrainian, "exit") => "Вихід",
        _ => "",
    }
}

pub fn tray_tooltip(language: Language) -> &'static str {
    match language {
        Language::English => "Bluetooth device battery",
        Language::Russian => "Заряд Bluetooth-устройств",
        Language::Ukrainian => "Заряд Bluetooth-пристроїв",
    }
}

pub fn tooltip_empty(language: Language) -> &'static str {
    match language {
        Language::English => "No connected devices with battery level",
        Language::Russian => "Подключённых устройств с зарядом нет",
        Language::Ukrainian => "Підключених пристроїв із зарядом немає",
    }
}

pub fn low_battery_title(language: Language) -> &'static str {
    match language {
        Language::English => "Low battery",
        Language::Russian => "Низкий заряд",
        Language::Ukrainian => "Низький заряд",
    }
}

pub fn low_battery_message(language: Language, name: &str, level: u8) -> String {
    let message = match language {
        Language::English => "time to charge",
        Language::Russian => "пора зарядить",
        Language::Ukrainian => "час зарядити",
    };
    format!("{}: {}% — {}.", device_name(language, name), level, message)
}

// ---------- палитра ----------

#[derive(Clone, Copy)]
struct Palette {
    bg: COLORREF,
    hover: COLORREF,
    border: COLORREF,
    text: COLORREF,
    disabled: COLORREF,
}

const DARK_PALETTE: Palette = Palette {
    bg: COLORREF(0x00202020),
    hover: COLORREF(0x00333333),
    border: COLORREF(0x00454545),
    text: COLORREF(0x00E8E8E8),
    disabled: COLORREF(0x008C8C8C),
};

const LIGHT_PALETTE: Palette = Palette {
    bg: COLORREF(0x00FFFFFF),
    hover: COLORREF(0x00E8E8E8),
    border: COLORREF(0x00B8B8B8),
    text: COLORREF(0x00181818),
    disabled: COLORREF(0x00707070),
};

fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => DARK_PALETTE,
        Theme::Light => LIGHT_PALETTE,
    }
}

// ---------- геометрия ----------

const WM_TRAY: u32 = WM_APP + 1; // клик по иконке трея (как в app.rs)
const WM_HOOK_CLOSE: u32 = WM_APP + 9; // хук мыши: клик вне меню → закрыть
const SUBMENU_HOVER_TIMER: usize = 0x52; // таймер открытия подменю при наведении
const SUBMENU_HOVER_MS: u32 = 300; // задержка как у нативных меню (MenuShowDelay)
const FOCUS_CHECK_TIMER: usize = 0x53;

// Хук мыши (WH_MOUSE_LL) — единственный надёжный способ закрывать меню по клику вне
// окна: захват мыши (SetCapture) на этой системе не редиректит клики в меню.
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
static HOOK_HWND: AtomicIsize = AtomicIsize::new(0);
static HOOK_CHILD_HWND: AtomicIsize = AtomicIsize::new(0);
static MENU_ACTIVE: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        if matches!(msg, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
            let hwnd = HOOK_HWND.load(Ordering::SeqCst);
            if hwnd != 0 {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let under = WindowFromPoint(pt);
                let child = HOOK_CHILD_HWND.load(Ordering::SeqCst);
                if under.0 as isize != hwnd && under.0 as isize != child {
                    // клик вне обоих окон меню — попросить меню закрыться
                    let _ = PostMessageW(
                        HWND(hwnd as *mut core::ffi::c_void),
                        WM_HOOK_CLOSE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }
    }
    CallNextHookEx(HHOOK(HOOK_HANDLE.load(Ordering::SeqCst) as *mut core::ffi::c_void), code, wparam, lparam)
}

const ITEM_H: i32 = 28; // высота пункта
const SEP_H: i32 = 9; // высота разделителя
const GLYPH_ZONE: i32 = 36; // место под radio/check справа
const TEXT_PAD: i32 = 10; // отступ текста слева

// ---------- модель пунктов ----------

#[derive(Debug, Clone, PartialEq)]
enum ItemKind {
    Info,        // инфо, disabled
    Separator,   // линия
    Radio(bool), // checked
    Check(bool), // checked
    Action,      // обычный пункт
    Submenu,     // открывает вложенное меню
}

#[derive(Debug, Clone)]
struct MenuItem {
    text: String,
    kind: ItemKind,
    action: MenuAction,
    submenu: Option<SubmenuKind>,
}

impl MenuItem {
    fn info(text: String) -> Self {
        Self { text, kind: ItemKind::Info, action: MenuAction::None, submenu: None }
    }
    fn separator() -> Self {
        Self { text: String::new(), kind: ItemKind::Separator, action: MenuAction::None, submenu: None }
    }
    fn radio(text: String, checked: bool, action: MenuAction) -> Self {
        Self { text, kind: ItemKind::Radio(checked), action, submenu: None }
    }
    fn check(text: String, checked: bool, action: MenuAction) -> Self {
        Self { text, kind: ItemKind::Check(checked), action, submenu: None }
    }
    fn action_item(text: String, action: MenuAction) -> Self {
        Self { text, kind: ItemKind::Action, action, submenu: None }
    }
    fn submenu(text: String, submenu: SubmenuKind) -> Self {
        Self { text, kind: ItemKind::Submenu, action: MenuAction::None, submenu: Some(submenu) }
    }
}

/// Собирает плоский список пунктов главного меню.
fn build_items(devices: &[DeviceBattery], target: &str, target_name: &str, autostart: bool, updated: &str, language: Language) -> Vec<MenuItem> {
    let mut items = Vec::new();
    items.push(MenuItem::info(format!("{}: {}", text(language, "updated"), updated)));
    if devices.is_empty() {
        items.push(MenuItem::info(text(language, "no_devices").to_string()));
    } else {
        for d in devices {
            let name = device_name(language, &d.name);
            let prefix = if !d.address.is_empty() && d.address.eq_ignore_ascii_case(target) { "● " } else { "" };
            items.push(MenuItem::info(format!("{}{}: {}%", prefix, name, d.level)));
        }
    }
    items.push(MenuItem::separator());
    let target_label = if target.is_empty() {
        format!("{}: {}", text(language, "target"), text(language, "auto"))
    } else {
        match devices.iter().find(|d| d.address.eq_ignore_ascii_case(target)) {
            Some(d) => format!("{}: {}", text(language, "target"), device_name(language, &d.name)),
            None if !target_name.is_empty() => format!("{}: {}", text(language, "target"), target_name),
            None => format!("{}: {}", text(language, "target"), text(language, "not_connected")),
        }
    };
    items.push(MenuItem::submenu(target_label, SubmenuKind::Target));
    items.push(MenuItem::submenu(text(language, "language").to_string(), SubmenuKind::Language));
    items.push(MenuItem::submenu(text(language, "theme").to_string(), SubmenuKind::Theme));
    items.push(MenuItem::separator());
    items.push(MenuItem::action_item(text(language, "refresh").to_string(), MenuAction::Refresh));
    items.push(MenuItem::check(text(language, "autostart").to_string(), autostart, MenuAction::ToggleAutostart));
    items.push(MenuItem::separator());
    items.push(MenuItem::action_item(text(language, "exit").to_string(), MenuAction::Exit));
    items
}

fn build_target_items(devices: &[DeviceBattery], target: &str, language: Language) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::radio(text(language, "auto").to_string(), target.is_empty(), MenuAction::SetTarget(String::new()))];
    for d in devices {
        items.push(MenuItem::radio(device_name(language, &d.name).to_string(), d.address.eq_ignore_ascii_case(target), MenuAction::SetTarget(d.address.clone())));
    }
    items
}

fn build_language_items(language: Language) -> Vec<MenuItem> {
    vec![
        MenuItem::radio(text(language, "english").to_string(), language == Language::English, MenuAction::SetLanguage(Language::English)),
        MenuItem::radio(text(language, "russian").to_string(), language == Language::Russian, MenuAction::SetLanguage(Language::Russian)),
        MenuItem::radio(text(language, "ukrainian").to_string(), language == Language::Ukrainian, MenuAction::SetLanguage(Language::Ukrainian)),
    ]
}

fn build_theme_items(language: Language, theme: Theme) -> Vec<MenuItem> {
    vec![
        MenuItem::radio(text(language, "dark").to_string(), theme == Theme::Dark, MenuAction::SetTheme(Theme::Dark)),
        MenuItem::radio(text(language, "light").to_string(), theme == Theme::Light, MenuAction::SetTheme(Theme::Light)),
    ]
}

// ---------- шрифт ----------

/// Segoe UI 9pt (≈ −12 px при 96 dpi), либо системный шрифт.
unsafe fn create_menu_font() -> HFONT {
    CreateFontW(
        -12,  // высота в пикселях
        0,
        0,
        0,
        400, // FW_NORMAL
        0,   // не курсив
        0,   // не подчёркнут
        0,   // не зачёркнут
        1,   // DEFAULT_CHARSET
        0,   // OUT_DEFAULT_PRECIS
        0,   // CLIP_DEFAULT_PRECIS
        5,   // CLEARTYPE_QUALITY
        0,   // DEFAULT_PITCH | FF_DONTCARE
        w!("Segoe UI"),
    )
}

fn to_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// ---------- измерение и раскладка ----------

/// Ширина = макс. ширина текста + левый отступ + 36 (галочки) + 2 (рамка) + запас.
/// Без TEXT_PAD текст обрезается ровно на величину левого отступа (например, «%» в «80%»).
unsafe fn measure(items: &[MenuItem], hdc: HDC, font: HFONT) -> (i32, i32) {
    let old = SelectObject(hdc, font);
    let mut max_w = 0i32;
    let mut h = 2i32; // верхняя и нижняя рамка
    for it in items {
        if it.kind == ItemKind::Separator {
            h = h.saturating_add(SEP_H);
            continue;
        }
        let buf = to_utf16(&it.text);
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &buf, &mut sz);
        if sz.cx > max_w {
            max_w = sz.cx;
        }
        h = h.saturating_add(ITEM_H);
    }
    let _ = SelectObject(hdc, old);
    (max_w.saturating_add(TEXT_PAD + GLYPH_ZONE + 4), h)
}

/// Возвращает индекс пункта под y (client coords), или −1 (вне пунктов/рамки).
fn hit_test(items: &[MenuItem], y: i32, height: i32) -> i32 {
    if y < 1 || y >= height - 1 {
        return -1;
    }
    let mut cur = 1i32;
    for (i, it) in items.iter().enumerate() {
        let h = if it.kind == ItemKind::Separator { SEP_H } else { ITEM_H };
        if y >= cur && y < cur + h {
            return i as i32;
        }
        cur += h;
    }
    -1
}

fn item_top(items: &[MenuItem], index: usize) -> i32 {
    1 + items
        .iter()
        .take(index)
        .map(|item| if item.kind == ItemKind::Separator { SEP_H } else { ITEM_H })
        .sum::<i32>()
}

// ---------- отрисовка (общая для окна и render_test) ----------

unsafe fn draw_radio(hdc: HDC, cx: i32, cy: i32, checked: bool, color: COLORREF) {
    let pen = CreatePen(PS_SOLID, 1, color);
    let old_pen = SelectObject(hdc, pen);
    let hollow = GetStockObject(NULL_BRUSH);
    let old_br = SelectObject(hdc, hollow);
    // кружок
    let _ = Ellipse(hdc, cx - 6, cy - 6, cx + 6, cy + 6);
    if checked {
        // точка внутри
        let br = CreateSolidBrush(color);
        let old2 = SelectObject(hdc, br);
        let _ = Ellipse(hdc, cx - 2, cy - 2, cx + 2, cy + 2);
        let _ = SelectObject(hdc, old2);
        let _ = DeleteObject(br);
    }
    let _ = SelectObject(hdc, old_br);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

unsafe fn draw_check(hdc: HDC, cx: i32, cy: i32, checked: bool, color: COLORREF) {
    let pen = CreatePen(PS_SOLID, 1, color);
    let old_pen = SelectObject(hdc, pen);
    let hollow = GetStockObject(NULL_BRUSH);
    let old_br = SelectObject(hdc, hollow);
    // квадратик
    let _ = Rectangle(hdc, cx - 6, cy - 6, cx + 6, cy + 6);
    if checked {
        // галочка
        let _ = MoveToEx(hdc, cx - 4, cy, None);
        let _ = LineTo(hdc, cx - 1, cy + 3);
        let _ = LineTo(hdc, cx + 4, cy - 3);
    }
    let _ = SelectObject(hdc, old_br);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

/// Заполненный треугольник, направленный влево — в сторону дочернего меню.
unsafe fn draw_arrow(hdc: HDC, cx: i32, cy: i32, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    let old_brush = SelectObject(hdc, brush);
    let points = [
        POINT { x: cx + 3, y: cy - 5 },
        POINT { x: cx - 3, y: cy },
        POINT { x: cx + 3, y: cy + 5 },
    ];
    let _ = Polygon(hdc, &points);
    let _ = SelectObject(hdc, old_brush);
    let _ = DeleteObject(brush);
}

/// Рисует всё меню в hdc. hover: индекс подсвеченного пункта (−1 = нет).
unsafe fn paint_menu(hdc: HDC, items: &[MenuItem], hover: i32, font: HFONT, width: i32, height: i32, theme: Theme) {
    let colors = palette(theme);
    // фон
    let bg = CreateSolidBrush(colors.bg);
    let full = RECT { left: 0, top: 0, right: width, bottom: height };
    let _ = FillRect(hdc, &full, bg);
    let _ = DeleteObject(bg);

    // рамка 1px
    let border_pen = CreatePen(PS_SOLID, 1, colors.border);
    let old_pen = SelectObject(hdc, border_pen);
    let hollow = GetStockObject(NULL_BRUSH);
    let old_br = SelectObject(hdc, hollow);
    let _ = Rectangle(hdc, 0, 0, width, height);
    let _ = SelectObject(hdc, old_br);

    // шрифт
    let _ = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, TRANSPARENT);

    let mut y = 1i32; // под верхней рамкой
    for (i, it) in items.iter().enumerate() {
        if it.kind == ItemKind::Separator {
            // линия-разделитель цветом рамки
            let pen = CreatePen(PS_SOLID, 1, colors.border);
            let old_separator_pen = SelectObject(hdc, pen);
            let _ = MoveToEx(hdc, 12, y + SEP_H / 2, None);
            let _ = LineTo(hdc, width - 12, y + SEP_H / 2);
            let _ = SelectObject(hdc, old_separator_pen);
            let _ = DeleteObject(pen);
            y += SEP_H;
            continue;
        }

        let rect = RECT { left: 1, top: y, right: width - 1, bottom: y + ITEM_H };

        // hover-подсветка
        if i as i32 == hover {
            let hb = CreateSolidBrush(colors.hover);
            let _ = FillRect(hdc, &rect, hb);
            let _ = DeleteObject(hb);
        }

        // цвет текста: disabled-инфо — серым
        let item_color = if it.kind == ItemKind::Info { colors.disabled } else { colors.text };

        let mut text_rect = rect;
        text_rect.left += if matches!(it.kind, ItemKind::Info) { TEXT_PAD } else { GLYPH_ZONE };
        text_rect.right -= TEXT_PAD;
        let mut buf = to_utf16(&it.text);
        let _ = SetTextColor(hdc, item_color);
        let _ = DrawTextW(
            hdc,
            &mut buf,
            &mut text_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );

        // глифы слева: дочернее меню открывается в эту сторону
        let cx = GLYPH_ZONE / 2 + 1;
        let cy = y + ITEM_H / 2;
        match &it.kind {
            ItemKind::Radio(checked) => draw_radio(hdc, cx, cy, *checked, item_color),
            ItemKind::Check(checked) => draw_check(hdc, cx, cy, *checked, item_color),
            ItemKind::Submenu => draw_arrow(hdc, cx, cy, item_color),
            _ => {}
        }

        y += ITEM_H;
    }

    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(border_pen);
}

// ---------- окно ----------

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Состояние меню, доступное из wndproc через GWLP_USERDATA.
struct MenuState {
    items: Vec<MenuItem>,
    font: HFONT,
    width: i32,
    height: i32,
    hover: i32,
    done: bool,
    result: MenuAction,
    open_submenu: Option<SubmenuKind>,
    submenu_top: i32,
    theme: Theme,
}

/// Закрывает меню: помечает done, сохраняет результат, снимает хук,
/// уничтожает окно. Вызывается из wndproc и из вложенного цикла.
unsafe fn close_menu(hwnd: HWND, state: &mut MenuState, action: MenuAction) {
    if state.done {
        return;
    }
    state.done = true;
    state.result = action;
    // userdata обнуляем до DestroyWindow, чтобы реентрантные сообщения
    // (WM_NCDESTROY) не трогали state
    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    let hook = HOOK_HANDLE.swap(0, Ordering::SeqCst);
    HOOK_HWND.store(0, Ordering::SeqCst);
    if hook != 0 {
        let _ = UnhookWindowsHookEx(HHOOK(hook as *mut core::ffi::c_void));
    }
    let _ = DestroyWindow(hwnd);
}

/// Завершает цикл меню, сохраняя окно и состояние видимыми для подменю.
unsafe fn suspend_menu(hwnd: HWND, state: &mut MenuState) {
    if state.done {
        return;
    }
    state.done = true;
    state.result = MenuAction::None;
    let _ = KillTimer(hwnd, SUBMENU_HOVER_TIMER);
    let hook = HOOK_HANDLE.swap(0, Ordering::SeqCst);
    HOOK_HWND.store(0, Ordering::SeqCst);
    if hook != 0 {
        let _ = UnhookWindowsHookEx(HHOOK(hook as *mut core::ffi::c_void));
    }
}

fn client_pos(lparam: LPARAM) -> (i32, i32) {
    let v = lparam.0 as u32;
    let x = (v & 0xFFFF) as u16 as i16 as i32;
    let y = ((v >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

unsafe extern "system" fn menu_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let userdata = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if userdata == 0 {
        // окно ещё создаётся или уже закрыто
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *(userdata as *mut MenuState);

    match msg {
        WM_ACTIVATE => {
            if wparam.0 == 0 { close_menu(hwnd, state, MenuAction::None); }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 as usize == FOCUS_CHECK_TIMER => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let under = WindowFromPoint(pt);
            let child = HOOK_CHILD_HWND.load(Ordering::SeqCst);
            if under.0 as isize != hwnd.0 as isize && under.0 as isize != child {
                close_menu(hwnd, state, MenuAction::None);
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize), // 3 — не активировать
        WM_ERASEBKGND => LRESULT(1), // фон рисуем в WM_PAINT целиком
        WM_SETCURSOR => {
            // всегда обычная стрелка — никаких busy/loading-курсоров
            if let Ok(cursor) = LoadCursorW(None, IDC_ARROW) {
                let _ = SetCursor(cursor);
            }
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_menu(hdc, &state.items, state.hover, state.font, state.width, state.height, state.theme);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_pos(lparam);
            if x < 0 || y < 0 || x >= state.width || y >= state.height {
                close_menu(hwnd, state, MenuAction::None);
                return LRESULT(0);
            }
            let hover = if x < 0 || y < 0 || x >= state.width || y >= state.height {
                -1
            } else {
                hit_test(&state.items, y, state.height)
            };
            if hover != state.hover {
                state.hover = hover;
                // подменю «Цель» — открывается при наведении (с задержкой, как нативные)
                let on_submenu = hover >= 0
                    && (hover as usize) < state.items.len()
                    && state.items[hover as usize].kind == ItemKind::Submenu;
                if on_submenu {
                    let _ = SetTimer(hwnd, SUBMENU_HOVER_TIMER, SUBMENU_HOVER_MS, None);
                } else {
                    let _ = KillTimer(hwnd, SUBMENU_HOVER_TIMER);
                }
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 as usize == SUBMENU_HOVER_TIMER => {
            let _ = KillTimer(hwnd, SUBMENU_HOVER_TIMER);
            // открываем подменю, только если курсор всё ещё над меню
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            if WindowFromPoint(pt) == hwnd && state.hover >= 0 {
                let index = state.hover as usize;
                state.open_submenu = state.items[index].submenu;
                state.submenu_top = item_top(&state.items, index);
                suspend_menu(hwnd, state);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            let (x, y) = client_pos(lparam);
            if x < 0 || y < 0 || x >= state.width || y >= state.height {
                // клик вне окна — закрыть без выбора
                close_menu(hwnd, state, MenuAction::None);
            } else {
                let idx = hit_test(&state.items, y, state.height);
                let action = if idx >= 0 {
                    let item = &state.items[idx as usize];
                    if item.kind == ItemKind::Submenu {
                        // открыть вложенное меню (продолжит run_menu)
                        state.open_submenu = item.submenu;
                        state.submenu_top = item_top(&state.items, idx as usize);
                        return LRESULT(0);
                    }
                    item.action.clone()
                } else {
                    MenuAction::None
                };
                close_menu(hwnd, state, action);
            }
            LRESULT(0)
        }
        WM_HOOK_CLOSE => {
            // клик вне окна меню (сообщил хук мыши) — закрыть без выбора
            close_menu(hwnd, state, MenuAction::None);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 as u32 == VK_ESCAPE.0 as u32 => {
            close_menu(hwnd, state, MenuAction::None);
            LRESULT(0)
        }
        WM_CLOSE => {
            close_menu(hwnd, state, MenuAction::None);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------- попап (общий для главного меню и подменю) ----------

struct PopupResult {
    action: MenuAction,
    open_submenu: Option<SubmenuKind>,
    submenu_top: i32,
    x: i32,
    y: i32,
    width: i32,
    window: Option<HWND>,
    state: Option<Box<MenuState>>,
}

/// Показывает попап с пунктами у (x, y), ждёт выбора (вложенный цикл сообщений).
/// Возвращает действие и итоговую геометрию окна (для позиционирования подменю).
unsafe fn create_popup(items: Vec<MenuItem>, x: i32, y: i32, theme: Theme) -> PopupResult {
    let empty = PopupResult {
        action: MenuAction::None,
        open_submenu: None,
        submenu_top: 0,
        x,
        y,
        width: 0,
        window: None,
        state: None,
    };

    let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();

    // оконный класс — регистрируем один раз
    if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
        let wc = WNDCLASSW {
            style: Default::default(),
            lpfnWndProc: Some(menu_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: w!("BtBatteryTray_PopupMenu"),
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return empty;
        }
    }

    let font = create_menu_font();

    // размеры
    let screen = GetDC(None);
    let (width, height) = measure(&items, screen, font);
    let _ = ReleaseDC(None, screen);

    // позиция с подгонкой под экран
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let mut x = x;
    let mut y = y;
    if x + width > sw {
        x = sw - width;
    }
    if y + height > sh {
        y = sh - height;
    }
    if x < 0 {
        x = 0;
    }
    if y < 0 {
        y = 0;
    }

    // окно-попап
    let hwnd = match CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
        w!("BtBatteryTray_PopupMenu"),
        w!(""),
        WS_POPUP,
        x,
        y,
        width,
        height,
        HWND::default(),
        HMENU::default(),
        hinstance,
        None,
    ) {
        Ok(h) => h,
        Err(_) => {
            let _ = DeleteObject(font);
            return empty;
        }
    };

    let mut state = Box::new(MenuState {
        items,
        font,
        width,
        height,
        hover: -1,
        done: false,
        result: MenuAction::None,
        open_submenu: None,
        submenu_top: 1,
        theme,
    });
    let state_ptr: *mut MenuState = &mut *state;
    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);

    // Показ без активации. Единственный глобальный хук принадлежит run_menu;
    // создание дочернего popup не должно перезаписывать HWND родителя.
    let _ = ShowWindow(hwnd, SW_SHOWNA);
    let _ = SetTimer(hwnd, FOCUS_CHECK_TIMER, 100, None);
    let _ = UpdateWindow(hwnd);

    PopupResult {
        action: MenuAction::None,
        open_submenu: None,
        submenu_top: 1,
        x, y, width,
        window: Some(hwnd),
        state: Some(state),
    }
}

unsafe fn destroy_kept_popup(mut popup: PopupResult) {
    let Some(hwnd) = popup.window else { return; };
    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    let _ = DestroyWindow(hwnd);
    if let Some(state) = popup.state.take() {
        let font = state.font;
        drop(state);
        let _ = DeleteObject(font);
    }
}

// ---------- публичный API ----------

/// Показывает тёмное меню у курсора, ждёт выбора (вложенный цикл сообщений).
/// Возвращает действие; None — меню закрыто без выбора.
pub fn run_menu(devices: &[DeviceBattery], target: &str, target_name: &str, autostart: bool, language: Language, theme: Theme) -> MenuAction {
    if MENU_ACTIVE.swap(true, Ordering::SeqCst) {
        return MenuAction::None;
    }
    let updated = chrono::Local::now().format("%H:%M:%S").to_string();
    let items = build_items(devices, target, target_name, autostart, &updated, language);
    unsafe {
        let mut pt = POINT::default(); let _ = GetCursorPos(&mut pt);
        let mut parent = create_popup(items, pt.x - 4, pt.y - 4, theme);
        let Some(parent_hwnd) = parent.window else {
            MENU_ACTIVE.store(false, Ordering::SeqCst);
            return MenuAction::None;
        };
        let parent_state = parent.state.as_mut().unwrap();
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0);
        if let Ok(h) = hook { HOOK_HANDLE.store(h.0 as isize, Ordering::SeqCst); HOOK_HWND.store(parent_hwnd.0 as isize, Ordering::SeqCst); }
        let mut child: Option<PopupResult> = None; let mut result = MenuAction::None; let mut msg=MSG::default();
        loop {
            let ok=GetMessageW(&mut msg,None,0,0); if !ok.as_bool(){break;}
            let ph=parent.window; let ch=child.as_ref().and_then(|p|p.window);
            if msg.message==WM_KEYDOWN && msg.wParam.0 as u32==VK_ESCAPE.0 as u32 {break;}
            // Dispatch menu-owned mouse messages first. In particular, do not let the
            // outer loop consume WM_LBUTTONDOWN before menu_wnd_proc can suspend the
            // parent and publish open_submenu for the child-popup step below.
            if Some(msg.hwnd)==ph || Some(msg.hwnd)==ch {
                let _=TranslateMessage(&msg); DispatchMessageW(&msg);
            } else if msg.message==WM_MOUSEMOVE {
            } else if matches!(msg.message, WM_LBUTTONDOWN|WM_LBUTTONUP|WM_RBUTTONDOWN|WM_RBUTTONUP|WM_MBUTTONDOWN|WM_MBUTTONUP|WM_MOUSEWHEEL|WM_TRAY) {break;}
            else { let _=TranslateMessage(&msg); DispatchMessageW(&msg); }
            if parent_state.open_submenu.is_some() {
                // The parent can request a different submenu while the old child is
                // still alive. Replace it immediately so only one child popup exists.
                if let Some(old_child) = child.take() {
                    HOOK_CHILD_HWND.store(0, Ordering::SeqCst);
                    destroy_kept_popup(old_child);
                }
                let kind=parent_state.open_submenu.take().unwrap();
                let sub_items=match kind {SubmenuKind::Target=>build_target_items(devices,target,language),SubmenuKind::Language=>build_language_items(language),SubmenuKind::Theme=>build_theme_items(language,theme)};
                let screen=GetDC(None); let font=create_menu_font(); let (sw,sh)=measure(&sub_items,screen,font); let _=ReleaseDC(None,screen); let _=DeleteObject(font);
                let max_x=(GetSystemMetrics(SM_CXSCREEN)-parent.width).max(0); let px=parent.x.max(sw).min(max_x);
                if px!=parent.x {let _=SetWindowPos(parent_hwnd,HWND::default(),px,parent.y,0,0,SWP_NOACTIVATE|SWP_NOSIZE|SWP_NOZORDER);parent.x=px;}
                let sy=(parent.y+parent_state.submenu_top-1).min((GetSystemMetrics(SM_CYSCREEN)-sh).max(0));
                child=Some(create_popup(sub_items,parent.x-sw,sy,theme));
                HOOK_CHILD_HWND.store(child.as_ref().and_then(|p| p.window).map(|h| h.0 as isize).unwrap_or(0), Ordering::SeqCst);
            }
            if parent_state.done && child.is_none() {result=parent_state.result.clone();break;}
            if let Some(c)=child.as_mut() {if let Some(st)=c.state.as_mut(){if st.done {result=st.result.clone();break;}}}
        }
        if let Some(c)=child {destroy_kept_popup(c);} destroy_kept_popup(parent);
        let hook=HOOK_HANDLE.swap(0,Ordering::SeqCst); HOOK_HWND.store(0,Ordering::SeqCst); HOOK_CHILD_HWND.store(0,Ordering::SeqCst); if hook!=0 {let _=UnhookWindowsHookEx(HHOOK(hook as *mut core::ffi::c_void));}
        MENU_ACTIVE.store(false, Ordering::SeqCst);
        result
    }
}

// ---------- BMP ----------

/// Пишет BMP 32bpp (BGRA, снизу-вверх) из top-down буфера.
fn write_bmp(path: &str, width: i32, height: i32, topdown: &[u8]) -> std::io::Result<()> {
    let w = width as u32;
    let h = height as u32;
    let row = w * 4; // уже кратно 4
    let data_len = row * h;
    let mut buf = Vec::with_capacity((14 + 40 + data_len) as usize);

    // BITMAPFILEHEADER (14 байт)
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(14u32 + 40 + data_len).to_le_bytes()); // bfSize
    buf.extend_from_slice(&0u16.to_le_bytes()); // bfReserved1
    buf.extend_from_slice(&0u16.to_le_bytes()); // bfReserved2
    buf.extend_from_slice(&54u32.to_le_bytes()); // bfOffBits

    // BITMAPINFOHEADER (40 байт)
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&w.to_le_bytes()); // biWidth
    buf.extend_from_slice(&h.to_le_bytes()); // biHeight (положительный = снизу-вверх)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&data_len.to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0u32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // пиксели: в файле первая строка — нижняя (top-down буфер переворачиваем)
    for r in (0..h as usize).rev() {
        let off = r * row as usize;
        buf.extend_from_slice(&topdown[off..off + row as usize]);
    }

    std::fs::write(path, buf)
}

/// Служебный режим: рисует меню в menu_test.bmp (для пиксельной проверки цветов).
pub fn render_test() {
    let devices = vec![
        DeviceBattery {
            address: "AABBCCDDEE01".to_string(),
            name: "1MORE SonoFlow".to_string(),
            level: 90,
        },
        DeviceBattery {
            address: "AABBCCDDEE02".to_string(),
            name: "Xbox Wireless Controller".to_string(),
            level: 72,
        },
    ];
    // target строчными буквами — проверка case-insensitive совпадения адреса
    let items = build_items(&devices, "aabbccddee01", "1MORE SonoFlow", true, "12:34:56", Language::English);

    unsafe {
        let screen = GetDC(None);
        let font = create_menu_font();
        let (width, height) = measure(&items, screen, font);

        // 32bpp BGRA, top-down (biHeight отрицательный)
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            bmiColors: [RGBQUAD::default(); 1],
        };
        let mem = CreateCompatibleDC(screen);
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbmp = match CreateDIBSection(screen, &mut bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(_) => {
                let _ = DeleteDC(mem);
                let _ = ReleaseDC(None, screen);
                let _ = DeleteObject(font);
                return;
            }
        };
        let old_bmp = SelectObject(mem, hbmp);

        // общий код отрисовки, без hover
        paint_menu(mem, &items, -1, font, width, height, Theme::Dark);

        let n = match (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        {
            Some(n) => n,
            None => {
                let _ = SelectObject(mem, old_bmp);
                let _ = DeleteObject(hbmp);
                let _ = DeleteDC(mem);
                let _ = ReleaseDC(None, screen);
                let _ = DeleteObject(font);
                return;
            }
        };
        if bits.is_null() {
            let _ = SelectObject(mem, old_bmp);
            let _ = DeleteObject(hbmp);
            let _ = DeleteDC(mem);
            let _ = ReleaseDC(None, screen);
            let _ = DeleteObject(font);
            return;
        }
        let pixels = std::slice::from_raw_parts(bits as *const u8, n);

        match write_bmp("menu_test.bmp", width, height, pixels) {
            Ok(()) => println!("menu_test.bmp: {}x{} px, {} bytes", width, height, n),
            Err(e) => println!("menu_test.bmp: ошибка записи: {}", e),
        }

        let _ = SelectObject(mem, old_bmp);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(mem);
        let _ = ReleaseDC(None, screen);
        let _ = DeleteObject(font);
    }
}
