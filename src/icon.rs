//! Иконка-«батарейка»: рисуем пиксели вручную (BGRA), создаём HICON через
//! CreateIconFromResourceEx из ICO-блоба в памяти (надёжнее CreateIconIndirect,
//! который на части сборок Windows отказывается работать с 32bpp DIB без маски).

use std::os::windows::ffi::OsStrExt;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GDI_IMAGE_TYPE, HICON, IMAGE_FLAGS, LoadImageW,
};

pub struct TrayIcon {
    hicon: HICON,
}

impl TrayIcon {
    /// level: None → серая пустая батарейка.
    pub fn create(level: Option<u8>) -> windows::core::Result<TrayIcon> {
        const S: usize = 32;
        // BGRA, строки сверху вниз
        let mut px = vec![0u8; S * S * 4];

        let (r, g, b) = match level {
            None => (150, 150, 150),
            Some(l) if l < 20 => (226, 56, 56),
            Some(l) if l < 50 => (235, 160, 40),
            Some(_) => (52, 190, 110),
        };
        let (dr, dg, db) = (70, 70, 70); // обводка

        let frac = match level {
            None => 1.0, // серая = залита целиком
            Some(l) => (l as f32 / 100.0).clamp(0.0, 1.0),
        };
        let fill_w = (18.0 * frac) as usize; // ширина внутренней зоны 4..=21

        for y in 0..S {
            for x in 0..S {
                // контакт справа
                if (26..=28).contains(&x) && (13..=18).contains(&y) {
                    put(&mut px, x, y, db, dg, dr, 255);
                    continue;
                }
                // корпус
                if (2..=25).contains(&x) && (8..=24).contains(&y) {
                    let inner = (3..=24).contains(&x) && (9..=23).contains(&y);
                    if inner {
                        if x >= 4 && x < 4 + fill_w {
                            put(&mut px, x, y, b, g, r, 255);
                        } else {
                            put(&mut px, x, y, 0, 0, 0, 0); // пусто — прозрачно
                        }
                    } else {
                        put(&mut px, x, y, db, dg, dr, 255); // обводка
                    }
                }
            }
        }

        let hicon = create_icon_from_rgba(&px, S)?;
        Ok(TrayIcon { hicon })
    }

    pub fn hicon(&self) -> HICON {
        self.hicon
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.hicon);
        }
    }
}

/// Собирает ICO (ICONDIR + BITMAPINFOHEADER + XOR 32bpp + AND-маска) и создаёт иконку.
fn create_icon_from_rgba(px: &[u8], s: usize) -> windows::core::Result<HICON> {
    const XOR: usize = 32 * 32 * 4;
    const AND: usize = 32 * 4; // 1bpp, stride 4 байта
    let mut ico: Vec<u8> = Vec::with_capacity(22 + 40 + XOR + AND);

    // ICONDIR
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    ico.extend_from_slice(&1u16.to_le_bytes()); // count
    // ICONDIRENTRY
    ico.push(32);
    ico.push(32);
    ico.push(0); // colors
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bitcount
    ico.extend_from_slice(&((40 + XOR + AND) as u32).to_le_bytes()); // bytesInRes
    ico.extend_from_slice(&22u32.to_le_bytes()); // offset
    // BITMAPINFOHEADER (DIB внутри ICO: biHeight = 2*высота = XOR+AND)
    ico.extend_from_slice(&40u32.to_le_bytes()); // biSize
    ico.extend_from_slice(&(s as i32).to_le_bytes()); // biWidth
    ico.extend_from_slice(&((s * 2) as i32).to_le_bytes()); // biHeight
    ico.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    ico.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    ico.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    ico.extend_from_slice(&(XOR as u32).to_le_bytes()); // biSizeImage
    ico.extend_from_slice(&0u32.to_le_bytes()); // biXPelsPerMeter
    ico.extend_from_slice(&0u32.to_le_bytes()); // biYPelsPerMeter
    ico.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    ico.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    // XOR: 32 строки снизу-вверх, BGRA
    for row in (0..s).rev() {
        for x in 0..s {
            let i = (row * s + x) * 4;
            ico.extend_from_slice(&px[i..i + 4]);
        }
    }
    // AND-маска: 32 строки, 1bpp, bit=1 где прозрачно (alpha < 128), MSB-first
    for row in (0..s).rev() {
        for byte in 0..4 {
            let mut mask_byte = 0u8;
            for bit in 0..8 {
                let x = byte * 8 + bit;
                if x >= s {
                    break;
                }
                let alpha = px[(row * s + x) * 4 + 3];
                if alpha < 128 {
                    mask_byte |= 0x80 >> bit;
                }
            }
            ico.push(mask_byte);
        }
    }

    // На этой сборке Windows CreateIconFromResourceEx и CreateIconIndirect(32bpp без
    // маски) стабильно возвращают null → грузим иконку через LoadImageW из temp-файла.
    let ico_path = std::env::temp_dir().join(format!("bttray_icon_{}.ico", std::process::id()));
    let _ = std::fs::write(&ico_path, &ico);
    let result = unsafe {
        let wide: Vec<u16> = ico_path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let handle = LoadImageW(
            None,
            windows::core::PCWSTR(wide.as_ptr()),
            GDI_IMAGE_TYPE(1), // IMAGE_ICON
            0,
            0,
            IMAGE_FLAGS(0x10), // LR_LOADFROMFILE
        );
        handle.map(|h| HICON(h.0))
    };
    let _ = std::fs::remove_file(&ico_path);
    result.map_err(|e| {
        eprintln!("LoadImageW failed: {:?}", e);
        e
    })
}

#[inline]
fn put(px: &mut [u8], x: usize, y: usize, b: u8, g: u8, r: u8, a: u8) {
    let i = (y * 32 + x) * 4;
    px[i] = b;
    px[i + 1] = g;
    px[i + 2] = r;
    px[i + 3] = a;
}
