//! Порт BatteryMonitor.cs на Rust: чтение заряда Bluetooth-устройств через CfgMgr32.
//! Контракт НЕ меняем — на него завязан app.rs.
//!
//! Windows хранит заряд BT-устройства в PnP-свойстве DEVPKEY_Device_BatteryLevel
//! ({104EA319-6EE2-4701-BD47-8DDBF425BBE5}, pid 2). В реестре читаемым значением
//! его нет, WMI пуст — читается только через CfgMgr32 API (raw FFI, без крейта).
//! DEVPROPTYPE эмпирически: заряд = 0x03 (UINT32) или 0x01 (BYTE) — берём первый
//! байт; имя = 0x12 (STRING, UTF-16 с \0).

use std::collections::{HashMap, HashSet};

use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

#[derive(Debug, Clone)]
pub struct DeviceBattery {
    pub address: String, // 12 hex, верхний регистр, без разделителей
    pub name: String,
    pub level: u8, // 0..=100
}

// ---------- CfgMgr32 (raw FFI) ----------

const CR_SUCCESS: u32 = 0;
const CR_BUFFER_SMALL: u32 = 0x1A;
const INITIAL_PROPERTY_SIZE: u32 = 4096;
const MAX_PROPERTY_SIZE: u32 = 64 * 1024;

const CM_LOCATE_DEVNODE_NORMAL: u32 = 0;

// DEVPKEY_Device_BatteryLevel {104EA319-6EE2-4701-BD47-8DDBF425BBE5}, pid 2.
// fmtid: Data1/Data2/Data3 little-endian + Data4 как есть.
const KEY_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: [
        0x19, 0xA3, 0x4E, 0x10, // Data1 = 0x104EA319 (LE)
        0xE2, 0x6E, // Data2 = 0x6EE2 (LE)
        0x01, 0x47, // Data3 = 0x4701 (LE)
        0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5,
    ],
    pid: 2,
};

// DEVPKEY_Device_FriendlyName {A45C254E-DF1C-4EFD-8020-67D146A850E0}, pid 14.
const KEY_FRIENDLY_NAME: DEVPROPKEY = DEVPROPKEY {
    fmtid: [
        0x4E, 0x25, 0x5C, 0xA4, // Data1 = 0xA45C254E (LE)
        0x1C, 0xDF, // Data2 = 0xDF1C (LE)
        0xFD, 0x4E, // Data3 = 0x4EFD (LE)
        0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0,
    ],
    pid: 14,
};

#[repr(C)]
struct DEVPROPKEY {
    fmtid: [u8; 16],
    pid: u32,
}

#[link(name = "CfgMgr32")]
#[allow(non_snake_case)]
extern "system" {
    fn CM_Locate_DevNodeW(pdnDevInst: *mut i32, pdeviceid: *const u16, ulflags: u32) -> u32;
    fn CM_Get_DevNode_PropertyW(
        dndevinst: i32,
        propertykey: *const DEVPROPKEY,
        propertytype: *mut u32,
        propertybuffer: *mut u8,
        propertybuffersize: *mut u32,
        ulflags: u32,
    ) -> u32;
    fn CM_Get_DevNode_Status(pulstatus: *mut u32, pulproblemnumber: *mut u32, dndevinst: i32, ulflags: u32) -> u32;
}

const DN_STARTED: u32 = 0x8; // драйвер запущен — устройство присутствует и подключено

/// True, если devnode «стартовал» (устройство подключено и активно).
fn devnode_connected(devinst: i32) -> bool {
    let mut status: u32 = 0;
    let mut problem: u32 = 0;
    let rc = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, devinst, 0) };
    rc == CR_SUCCESS && status & DN_STARTED != 0
}

/// devinst по instance ID; None, если узел не найден (rc != 0).
fn locate_devnode(instance_id: &str) -> Option<i32> {
    let wide: Vec<u16> = instance_id.encode_utf16().chain(std::iter::once(0)).collect();
    let mut devinst: i32 = 0;
    let rc = unsafe { CM_Locate_DevNodeW(&mut devinst, wide.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) };
    if rc == CR_SUCCESS {
        Some(devinst)
    } else {
        None
    }
}

/// Читает свойство devnode как сырые байты: буфер 4096, при CR_BUFFER_SMALL (0x1A)
/// повтор с нужным размером; rc != 0 → свойства нет.
fn get_devnode_property(devinst: i32, key: &DEVPROPKEY) -> Option<Vec<u8>> {
    let mut prop_type: u32 = 0;
    let mut size: u32 = INITIAL_PROPERTY_SIZE;
    let mut buf = vec![0u8; size as usize];
    let rc = unsafe {
        CM_Get_DevNode_PropertyW(devinst, key, &mut prop_type, buf.as_mut_ptr(), &mut size, 0)
    };
    if rc == CR_SUCCESS {
        if size as usize <= buf.len() {
            buf.truncate(size as usize);
            return Some(buf);
        }
        return None;
    }
    if rc == CR_BUFFER_SMALL && size > 0 && size <= MAX_PROPERTY_SIZE {
        let mut buf = vec![0u8; size as usize];
        let mut size2 = size;
        let rc = unsafe {
            CM_Get_DevNode_PropertyW(devinst, key, &mut prop_type, buf.as_mut_ptr(), &mut size2, 0)
        };
        if rc == CR_SUCCESS && size2 as usize <= buf.len() {
            buf.truncate(size2 as usize);
            return Some(buf);
        }
    }
    None
}

/// Заряд: DEVPROPTYPE 0x03 (UINT32) или 0x01 (BYTE) — берём первый байт (0..=100).
fn read_battery_level(devinst: i32) -> Option<u8> {
    let data = get_devnode_property(devinst, &KEY_BATTERY)?;
    data.first().copied().map(|v| v.min(100))
}

/// Friendly name: DEVPROPTYPE 0x12 (STRING, UTF-16 с \0).
fn read_friendly_name(devinst: i32) -> Option<String> {
    let data = get_devnode_property(devinst, &KEY_FRIENDLY_NAME)?;
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    let name = String::from_utf16_lossy(&units[..end]);
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

// ---------- перечисление устройств (реестр) ----------

const BT_BRANCHES: [&str; 5] = ["BTH", "BTHLE", "BTHENUM", "BTHLEDEVICE", "BTHHFENUM"];

/// Все instance ID: "Ветка\Device\Instance" (или "Ветка\Device", если у device
/// нет подузлов-экземпляров). Префикс "SYSTEM\CurrentControlSet\Enum\" не входит.
fn enumerate_instance_ids() -> Vec<String> {
    let mut ids = Vec::new();
    let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Enum")
    else {
        return ids;
    };
    for branch in BT_BRANCHES {
        let Ok(branch_key) = root.open_subkey(branch) else {
            continue;
        };
        for device in branch_key.enum_keys().filter_map(Result::ok) {
            let Ok(device_key) = branch_key.open_subkey(&device) else {
                continue;
            };
            let instances: Vec<String> = device_key.enum_keys().filter_map(Result::ok).collect();
            if instances.is_empty() {
                ids.push(format!("{branch}\\{device}"));
            } else {
                for instance in instances {
                    ids.push(format!("{branch}\\{device}\\{instance}"));
                }
            }
        }
    }
    ids
}

// ---------- адрес устройства ----------

fn is_boundary(c: char) -> bool {
    c == '\\' || c == '&' || c == '_'
}

/// 12 шестнадцатеричных цифр на границе символов \ & _ (или начала/конца строки).
fn extract_address(instance_id: &str) -> Option<String> {
    let chars: Vec<char> = instance_id.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    while i + 12 <= n {
        let start_ok = i == 0 || is_boundary(chars[i - 1]);
        if start_ok && chars[i..i + 12].iter().all(|c| c.is_ascii_hexdigit()) {
            let end_ok = i + 12 == n || is_boundary(chars[i + 12]);
            if end_ok {
                let addr: String = chars[i..i + 12].iter().collect();
                return Some(addr.to_uppercase());
            }
        }
        i += 1;
    }
    None
}

/// Адрес базового узла: после "\dev_" идёт 12 hex (case-insensitive), дальше конец
/// строки или "\". Возвращает None для служебных узлов (напр. BTHENUM\{...}_LOCALMFG).
fn extract_base_address(instance_id: &str) -> Option<String> {
    let lower = instance_id.to_ascii_lowercase();
    let marker = "\\dev_";
    let pos = lower.find(marker)?;
    let start = pos + marker.len();
    let seg = lower.get(start..start + 12)?;
    if seg.chars().all(|c| c.is_ascii_hexdigit()) {
        let after = lower.as_bytes().get(start + 12).copied();
        if after.is_none() || after == Some(b'\\') {
            return Some(seg.to_uppercase());
        }
    }
    None
}

// ---------- группировка и результат ----------

struct Group {
    level: Option<u8>,
    base_name: Option<String>,
    charge_name: Option<String>,
}

/// Считывает заряды всех BT-устройств через CfgMgr32 (DEVPKEY_Device_BatteryLevel).
/// connected_only == None → показать все устройства с зарядом (fail-open, если WinRT упал).
pub fn get_devices_with_battery(connected_only: Option<&HashSet<String>>) -> Vec<DeviceBattery> {
    let mut groups: HashMap<String, Group> = HashMap::new();

    for instance_id in enumerate_instance_ids() {
        let Some(address) = extract_address(&instance_id) else {
            continue;
        };

        if let Some(set) = connected_only {
            if !set.contains(&address) && !set.contains(&address.to_ascii_lowercase()) {
                continue;
            }
        }

        let Some(devinst) = locate_devnode(&instance_id) else {
            continue;
        };

        if !devnode_connected(devinst) {
            continue; // устройство не подключено — пропускаем
        }

        let level = read_battery_level(devinst);
        let name = read_friendly_name(devinst);
        let is_base = extract_base_address(&instance_id).as_deref() == Some(address.as_str());

        let group = groups.entry(address.clone()).or_insert(Group {
            level: None,
            base_name: None,
            charge_name: None,
        });

        if let Some(lvl) = level {
            // уровень — максимум по группе
            group.level = Some(group.level.map_or(lvl, |cur| cur.max(lvl)));
            if group.charge_name.is_none() {
                group.charge_name = name.clone();
            }
        }
        if is_base && group.base_name.is_none() {
            group.base_name = name;
        }
    }

    let mut result: Vec<DeviceBattery> = groups
        .into_iter()
        .filter_map(|(address, g)| {
            let level = g.level?;
            let name = g
                .base_name
                .or(g.charge_name)
                .unwrap_or_else(|| "Bluetooth-устройство".to_string());
            Some(DeviceBattery { address, name, level })
        })
        .collect();

    // сортировка по имени (case-insensitive)
    result.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    result
}
