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

    if !app::acquire_single_instance() {
        return; // уже запущено
    }
    app::run();
}
