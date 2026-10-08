//! Вшивает иконку приложения в exe: компилирует `assets/app.rc` и отдаёт
//! результат линкеру.
//!
//! - MSVC-тулчейн: `rc.exe` из Windows SDK, линкер принимает `.res` напрямую.
//! - GNU-тулчейн: `windres` → объектный файл (путь можно задать переменной
//!   окружения `WINDRES`, если его нет в PATH).
//!
//! Если компилятора ресурсов нет вообще, сборка НЕ падает: exe просто
//! соберётся без иконки. Так чужие машины и CI остаются работоспособными.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR не задан"));
    let target = env::var("TARGET").unwrap_or_default();

    // Номер версии — из `version` в Cargo.toml (один источник истины).
    // Заголовок, а не -D: строковый define через -D не доезжает как литерал.
    let header = out.join("version.h");
    if let Err(e) = std::fs::write(&header, version_header()) {
        println!("cargo:warning=не смог записать version.h: {}", e);
    }

    if target.contains("msvc") {
        if let Some(rc) = pick(&[("rc.exe", "/?"), ("rc", "/?")]) {
            let res = out.join("app.res");
            let args = vec![
                "/nologo".to_string(),
                "/i".to_string(),
                path(&out),
                "/i".to_string(),
                "assets".to_string(),
                "/fo".to_string(),
                path(&res),
                "assets/app.rc".to_string(),
            ];
            if build(&rc, &args, &res) {
                return;
            }
        }
    } else if let Some(windres) = windres() {
        let obj = out.join("app_icon.o");
        let args = vec![
            "--include-dir".to_string(),
            path(&out),
            "--include-dir".to_string(),
            "assets".to_string(),
            "assets/app.rc".to_string(),
            path(&obj),
        ];
        if build(&windres, &args, &obj) {
            return;
        }
    }

    println!(
        "cargo:warning=компилятор ресурсов не найден (rc.exe или windres) — \
         exe собирается БЕЗ иконки"
    );
}

/// Содержимое `version.h` для VERSIONINFO в `assets/app.rc`.
fn version_header() -> String {
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut nums = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
    );
    format!(
        "// Сгенерировано build.rs из Cargo.toml — не редактировать.\n\
         #define VER_MAJOR {}\n\
         #define VER_MINOR {}\n\
         #define VER_PATCH {}\n\
         #define VER_STR \"{}\"\n",
        major, minor, patch, version
    )
}

/// windres: сначала переменная окружения (её удобно прописать в локальном
/// `.cargo/config.toml`, который не попадает в репозиторий), потом PATH.
fn windres() -> Option<String> {
    if let Ok(p) = env::var("WINDRES") {
        if Path::new(&p).is_file() {
            return Some(p);
        }
    }
    pick(&[("windres.exe", "--version"), ("windres", "--version")])
}

/// Первый из кандидатов, который реально запускается.
fn pick(candidates: &[(&str, &str)]) -> Option<String> {
    candidates
        .iter()
        .find(|(cmd, flag)| runs(cmd, flag))
        .map(|(cmd, _)| cmd.to_string())
}

fn runs(cmd: &str, flag: &str) -> bool {
    Command::new(cmd)
        .arg(flag)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Компилирует ресурсы и просит линкер подхватить результат.
fn build(compiler: &str, args: &[String], artifact: &Path) -> bool {
    let mut cmd = Command::new(compiler);
    cmd.args(args);

    // windres сам не компилирует .rc: препроцессор (gcc/cpp) он ищет в PATH.
    // Если gcc лежит рядом с windres (так в winlibs), достаточно добавить эту
    // папку в PATH дочернего процесса. Задавать --preprocessor вручную нельзя:
    // вместе с ним теряются штатные аргументы windres, и gcc уходит в линковку.
    if let Some(dir) = Path::new(compiler).parent() {
        if dir.join("gcc.exe").is_file() {
            let mut paths = vec![dir.to_path_buf()];
            if let Some(existing) = env::var_os("PATH") {
                paths.extend(env::split_paths(&existing));
            }
            if let Ok(joined) = env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
    }

    let ok = match cmd.output() {
        Ok(out) => {
            if !out.status.success() {
                // Показываем причину в логе сборки, а не молчим
                let msg = String::from_utf8_lossy(&out.stderr);
                let msg = msg.trim();
                println!(
                    "cargo:warning={} завершился с ошибкой: {}",
                    compiler,
                    if msg.is_empty() { "(без сообщения)" } else { msg }
                );
            }
            out.status.success() && artifact.is_file()
        }
        Err(e) => {
            println!("cargo:warning={} не удалось запустить: {}", compiler, e);
            false
        }
    };

    if ok {
        println!("cargo:rustc-link-arg-bins={}", path(artifact));
    } else {
        println!(
            "cargo:warning={} не смог собрать ресурсы — exe без иконки",
            compiler
        );
    }
    ok
}

fn path(p: &Path) -> String {
    // Windows-линкер ждёт обратные слэши; в остальном путь как есть
    p.to_string_lossy().replace('/', "\\")
}
