// Порт GetConnectedAddressesAsync (C#) на Rust: windows-крейт 0.58, WinRT.
// Контракт НЕ меняется.
use std::collections::HashSet;
use windows::core::{RuntimeType, HSTRING};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Foundation::IAsyncOperation;

/// Адреса устройств с поднятым радиоканалом (классика + BLE), 12 hex, верхний регистр.
/// Ошибка WinRT → Err (вызывающий делает fail-open).
pub fn get_connected_addresses() -> windows::core::Result<HashSet<String>> {
    let mut connected = HashSet::new();

    // Классические BT-устройства (BR/EDR).
    let selector = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
    let infos = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.get()?;
    for info in infos {
        let device = match open_connected(&info, BluetoothDevice::FromIdAsync) {
            Some(d) => d,
            None => continue,
        };
        match device.ConnectionStatus() {
            Ok(BluetoothConnectionStatus::Connected) => {
                if let Ok(addr) = device.BluetoothAddress() {
                    connected.insert(format!("{:012X}", addr));
                }
            }
            _ => {}
        }
    }

    // BLE-устройства.
    let selector = BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?;
    let infos = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.get()?;
    for info in infos {
        let device = match open_connected(&info, BluetoothLEDevice::FromIdAsync) {
            Some(d) => d,
            None => continue,
        };
        match device.ConnectionStatus() {
            Ok(BluetoothConnectionStatus::Connected) => {
                if let Ok(addr) = device.BluetoothAddress() {
                    connected.insert(format!("{:012X}", addr));
                }
            }
            _ => {}
        }
    }

    Ok(connected)
}

/// FromIdAsync по info.Id(); ошибки отдельных устройств пропускаем (fail-open на уровне устройства).
fn open_connected<T: RuntimeType + 'static>(
    info: &DeviceInformation,
    from_id: impl Fn(&HSTRING) -> windows::core::Result<IAsyncOperation<T>>,
) -> Option<T> {
    let id = info.Id().ok()?;
    from_id(&id).ok()?.get().ok()
}
