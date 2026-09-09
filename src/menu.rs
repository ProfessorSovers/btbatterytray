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
    PAINTSTRUCT, PS_SOLID, Rectangle, ReleaseDC, RGBQUAD, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GWLP_USERDATA, HCURSOR, HHOOK, HICON, HMENU,
    IDC_ARROW, KillTimer, LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, SetCursor, SetTimer, SetWindowsHookExW, SetWindowLongPtrW, ShowWindow,
    SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNA, TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL,
    WindowFromPoint, WNDCLASSW, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
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
    ToggleAutostart,
    Exit,
}

// ---------- палитра ----------

const COL_BG: COLORREF = COLORREF(0x00202020); // #202020
const COL_HOVER: COLORREF = COLORREF(0x00333333); // #333333
const COL_BORDER: COLORREF = COLORREF(0x00454545); // #454545
const COL_TEXT: COLORREF = COLORREF(0x00E8E8E8); // #E8E8E8
const COL_DISABLED: COLORREF = COLORREF(0x008C8C8C); // #8C8C8C

// ---------- геометрия ----------

const WM_TRAY: u32 = WM_APP + 1; // клик по иконке трея (как в app.rs)
const WM_HOOK_CLOSE: u32 = WM_APP + 9; // хук мыши: клик вне меню → закрыть
const SUBMENU_HOVER_TIMER: usize = 0x52; // таймер открытия подменю при наведении
const SUBMENU_HOVER_MS: u32 = 300; // задержка как у нативных меню (MenuShowDelay)

// Хук мыши (WH_MOUSE_LL) — единственный надёжный способ закрывать меню по клику вне
// окна: захват мыши (SetCapture) на этой системе не редиректит клики в меню.
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
static HOOK_HWND: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
            let hwnd = HOOK_HWND.load(Ordering::SeqCst);
            if hwnd != 0 {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let under = WindowFromPoint(pt);
                if under.0 as isize != hwnd {
                    // клик вне окна меню — попросить меню закрыться
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
}

impl MenuItem {
    fn info(text: String) -> Self {
        Self { text, kind: ItemKind::Info, action: MenuAction::None }
    }
    fn separator() -> Self {
        Self { text: String::new(), kind: ItemKind::Separator, action: MenuAction::None }
    }
    fn radio(text: String, checked: bool, action: MenuAction) -> Self {
        Self { text, kind: ItemKind::Radio(checked), action }
    }
    fn check(text: String, checked: bool, action: MenuAction) -> Self {
        Self { text, kind: ItemKind::Check(checked), action }
    }
    fn action_item(text: String, action: MenuAction) -> Self {
        Self { text, kind: ItemKind::Action, action }
    }
    fn submenu(text: String) -> Self {
        Self { text, kind: ItemKind::Submenu, action: MenuAction::None }
    }
}

/// Собирает плоский список пунктов главного меню.
fn build_items(devices: &[DeviceBattery], target: &str, target_name: &str, autostart: bool, updated: &str) -> Vec<MenuItem> {
    let mut items = Vec::new();

    // 1. инфо: время обновления
    items.push(MenuItem::info(format!("Обновлено: {}", updated)));

    // 2. устройства (disabled); у target-устройства — маркер "●"
    if devices.is_empty() {
        items.push(MenuItem::info("Подключённых устройств нет".to_string()));
    } else {
        for d in devices {
            let is_target = !d.address.is_empty() && d.address.eq_ignore_ascii_case(target);
            let text = if is_target {
                format!("● {}: {}%", d.name, d.level)
            } else {
                format!("{}: {}%", d.name, d.level)
            };
            items.push(MenuItem::info(text));
        }
    }

    // 3. разделитель
    items.push(MenuItem::separator());

    // 4. цель — отдельное подменю (чтобы меню не росло при 12+ устройствах)
    let target_label = if target.is_empty() {
        "Цель: Авто (самое разряженное)".to_string()
    } else {
        match devices.iter().find(|d| d.address.eq_ignore_ascii_case(target)) {
            Some(d) => format!("Цель: {}", d.name),
            None if !target_name.is_empty() => format!("Цель: {}", target_name),
            None => "Цель: не подключено".to_string(),
        }
    };
    items.push(MenuItem::submenu(target_label));

    // 5. разделитель
    items.push(MenuItem::separator());

    // 6. обновить
    items.push(MenuItem::action_item("Обновить сейчас".to_string(), MenuAction::Refresh));

    // 7. автозапуск
    items.push(MenuItem::check("Автозапуск".to_string(), autostart, MenuAction::ToggleAutostart));

    // 8. разделитель
    items.push(MenuItem::separator());

    // 9. выход
    items.push(MenuItem::action_item("Выход".to_string(), MenuAction::Exit));

    items
}

/// Пункты вложенного меню «Цель».
fn build_target_items(devices: &[DeviceBattery], target: &str) -> Vec<MenuItem> {
    let mut items = Vec::new();
    items.push(MenuItem::radio(
        "Авто (самое разряженное)".to_string(),
        target.is_empty(),
        MenuAction::SetTarget(String::new()),
    ));
    for d in devices {
        items.push(MenuItem::radio(
            d.name.clone(),
            d.address.eq_ignore_ascii_case(target),
            MenuAction::SetTarget(d.address.clone()),
        ));
    }
    items
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
            h += SEP_H;
            continue;
        }
        let buf = to_utf16(&it.text);
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &buf, &mut sz);
        if sz.cx > max_w {
            max_w = sz.cx;
        }
        h += ITEM_H;
    }
    let _ = SelectObject(hdc, old);
    (max_w + TEXT_PAD + GLYPH_ZONE + 4, h)
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
        let _ = Ellipse(hdc, cx - 2, cy - 2, cx + 3, cy + 3);
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

/// Стрелка «▸» для пункта с подменю.
unsafe fn draw_arrow(hdc: HDC, cx: i32, cy: i32, color: COLORREF) {
    let pen = CreatePen(PS_SOLID, 1, color);
    let old_pen = SelectObject(hdc, pen);
    let _ = MoveToEx(hdc, cx - 3, cy - 4, None);
    let _ = LineTo(hdc, cx + 3, cy);
    let _ = LineTo(hdc, cx - 3, cy + 4);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

/// Рисует всё меню в hdc. hover: индекс подсвеченного пункта (−1 = нет).
unsafe fn paint_menu(hdc: HDC, items: &[MenuItem], hover: i32, font: HFONT, width: i32, height: i32) {
    // фон
    let bg = CreateSolidBrush(COL_BG);
    let full = RECT { left: 0, top: 0, right: width, bottom: height };
    let _ = FillRect(hdc, &full, bg);
    let _ = DeleteObject(bg);

    // рамка 1px
    let border_pen = CreatePen(PS_SOLID, 1, COL_BORDER);
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
            let _ = MoveToEx(hdc, 12, y + SEP_H / 2, None);
            let _ = LineTo(hdc, width - 12, y + SEP_H / 2);
            y += SEP_H;
            continue;
        }

        let rect = RECT { left: 1, top: y, right: width - 1, bottom: y + ITEM_H };

        // hover-подсветка
        if i as i32 == hover {
            let hb = CreateSolidBrush(COL_HOVER);
            let _ = FillRect(hdc, &rect, hb);
            let _ = DeleteObject(hb);
        }

        // цвет текста: disabled-инфо — серым
        let text_color = if it.kind == ItemKind::Info { COL_DISABLED } else { COL_TEXT };

        let mut text_rect = rect;
        text_rect.left += TEXT_PAD;
        text_rect.right -= GLYPH_ZONE;
        let mut buf = to_utf16(&it.text);
        let _ = SetTextColor(hdc, text_color);
        let _ = DrawTextW(
            hdc,
            &mut buf,
            &mut text_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );

        // глифы справа, цветом текста
        let cx = width - GLYPH_ZONE / 2 - 1;
        let cy = y + ITEM_H / 2;
        match &it.kind {
            ItemKind::Radio(checked) => draw_radio(hdc, cx, cy, *checked, COL_TEXT),
            ItemKind::Check(checked) => draw_check(hdc, cx, cy, *checked, COL_TEXT),
            ItemKind::Submenu => draw_arrow(hdc, cx, cy, COL_TEXT),
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
    open_submenu: bool,
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
            paint_menu(hdc, &state.items, state.hover, state.font, state.width, state.height);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_pos(lparam);
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
            if WindowFromPoint(pt) == hwnd {
                state.open_submenu = true;
                close_menu(hwnd, state, MenuAction::None);
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
                        state.open_submenu = true;
                        close_menu(hwnd, state, MenuAction::None);
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
    open_submenu: bool,
    x: i32,
    y: i32,
    width: i32,
}

/// Показывает попап с пунктами у (x, y), ждёт выбора (вложенный цикл сообщений).
/// Возвращает действие и итоговую геометрию окна (для позиционирования подменю).
unsafe fn show_popup(items: Vec<MenuItem>, x: i32, y: i32) -> PopupResult {
    let empty = PopupResult { action: MenuAction::None, open_submenu: false, x, y, width: 0 };

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
        open_submenu: false,
    });
    let state_ptr: *mut MenuState = &mut *state;
    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);

    // показ без активации + хук мыши для закрытия по клику вне окна
    let _ = ShowWindow(hwnd, SW_SHOWNA);
    let _ = UpdateWindow(hwnd);
    if HOOK_HANDLE.load(Ordering::SeqCst) == 0 {
        let hook = SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(mouse_hook),
            None,
            0, // текущий поток
        );
        if let Ok(h) = hook {
            HOOK_HANDLE.store(h.0 as isize, Ordering::SeqCst);
            HOOK_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        }
    }

    // вложенный цикл сообщений до закрытия
    let state_ref = &mut *state_ptr;
    let mut msg = MSG::default();
    let mut got_quit = false;
    while !state_ref.done {
        let ok = GetMessageW(&mut msg, None, 0, 0);
        if !ok.as_bool() {
            // WM_QUIT — передать дальше, в главный цикл
            got_quit = true;
            break;
        }
        // ESC в любом окне потока — закрыть без выбора
        if msg.message == WM_KEYDOWN && msg.wParam.0 as u32 == VK_ESCAPE.0 as u32 {
            close_menu(hwnd, state_ref, MenuAction::None);
            break;
        }
        if msg.hwnd == hwnd {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        } else if matches!(
            msg.message,
            WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_RBUTTONDOWN
                | WM_RBUTTONUP
                | WM_MBUTTONDOWN
                | WM_MBUTTONUP
                | WM_MOUSEMOVE
                | WM_MOUSEWHEEL
                | WM_TRAY
        ) {
            // мышь/трей вне нашего окна — закрыть без выбора
            close_menu(hwnd, state_ref, MenuAction::None);
        } else {
            // WM_TIMER / WM_REFRESH и пр. — обычный диспатч в окно-получатель
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    if got_quit {
        PostQuitMessage(msg.wParam.0 as i32);
    }
    if !state_ref.done {
        // цикл завершён без выбора (например, WM_QUIT) — закрыть окно
        close_menu(hwnd, state_ref, MenuAction::None);
    }

    let result = std::mem::replace(&mut state_ref.result, MenuAction::None);
    let open_submenu = state_ref.open_submenu;
    let font = state_ref.font;
    drop(state);
    // шрифт больше никому не нужен: HFONT не имеет Drop — без явного удаления
    // утекает GDI-объект при каждом открытии меню (лимит процесса ~10 000)
    let _ = DeleteObject(font);
    PopupResult { action: result, open_submenu, x, y, width }
}

// ---------- публичный API ----------

/// Показывает тёмное меню у курсора, ждёт выбора (вложенный цикл сообщений).
/// Возвращает действие; None — меню закрыто без выбора.
pub fn run_menu(devices: &[DeviceBattery], target: &str, target_name: &str, autostart: bool) -> MenuAction {
    let updated = chrono::Local::now().format("%H:%M:%S").to_string();
    let items = build_items(devices, target, target_name, autostart, &updated);

    // смещение пункта «Цель» от верха меню (для вертикального выравнивания подменю)
    let target_top: i32 = items
        .iter()
        .take_while(|it| it.kind != ItemKind::Submenu)
        .map(|it| if it.kind == ItemKind::Separator { SEP_H } else { ITEM_H })
        .sum::<i32>()
        + 1; // + верхняя рамка

    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let main = show_popup(items, pt.x - 4, pt.y - 4);
        if !main.open_submenu {
            return main.action;
        }

        // подменю «Цель»: у правой кромки главного меню, выровнено по пункту
        let sub_items = build_target_items(devices, target);
        let screen = GetDC(None);
        let font = create_menu_font();
        let (sub_w, sub_h) = measure(&sub_items, screen, font);
        let _ = ReleaseDC(None, screen);
        let _ = DeleteObject(font);

        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let mut sub_x = main.x + main.width;
        let mut sub_y = main.y + target_top - 1;
        if sub_x + sub_w > sw {
            sub_x = main.x - sub_w; // не влезает справа — открыть слева
        }
        if sub_y + sub_h > sh {
            sub_y = sh - sub_h;
        }
        if sub_x < 0 {
            sub_x = 0;
        }
        if sub_y < 0 {
            sub_y = 0;
        }

        let sub = show_popup(sub_items, sub_x, sub_y);
        sub.action
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
    let items = build_items(&devices, "aabbccddee01", "1MORE SonoFlow", true, "12:34:56");

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
        paint_menu(mem, &items, -1, font, width, height);

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
