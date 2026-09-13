//! Трей-приложение: НИКАКИХ окон (даже консольного). GUI-subsystem — без этого
//! при запуске через автозапуск/двойной клик Windows показывает консоль.
#![windows_subsystem = "windows"]

mod app;
mod battery;
mod icon;
mod menu;

use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();

    // служебный режим: отрисовка меню в BMP для пиксельной проверки цветов
    if args.iter().any(|a| a == "--rendertest") {
        menu::render_test();
        return;
    }

    // служебный режим: что система считает подключённым (проверка фильтра)
    if args.iter().any(|a| a == "--conntest") {
        battery::conn_test();
        return;
    }

    // служебный режим: показать уведомление о низком заряде и выйти
    // (--balloontest или --balloontest=15; реальный триггер — строго ниже 20%)
    let demo_level = args.iter().find_map(|a| {
        a.strip_prefix("--balloontest").map(|rest| {
            rest.strip_prefix('=')
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(15)
        })
    });

    // демо не берёт мьютекс: его можно запускать, не закрывая работающий экземпляр
    if demo_level.is_none() && !app::acquire_single_instance() {
        return; // уже запущено
    }
    app::run(demo_level);
}
