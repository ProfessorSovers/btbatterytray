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

    if target.contains("msvc") {
        if let Some(rc) = pick(&[("rc.exe", "/?"), ("rc", "/?")]) {
            let res = out.join("app.res");
            if build(&rc, &["/nologo", "/fo", &path(&res), "assets/app.rc"], &res) {
                return;
            }
        }
    } else if let Some(windres) = windres() {
        let obj = out.join("app_icon.o");
        if build(
            &windres,
            &["--include-dir", "assets", "assets/app.rc", &path(&obj)],
            &obj,
        ) {
            return;
        }
    }

    println!(
        "cargo:warning=компилятор ресурсов не найден (rc.exe или windres) — \
         exe собирается БЕЗ иконки"
    );
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
fn build(compiler: &str, args: &[&str], artifact: &Path) -> bool {
    let ok = Command::new(compiler)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && artifact.is_file();

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
