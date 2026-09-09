# BtBatteryTray

Lightweight Windows tray application that displays battery levels reported by the Windows Bluetooth stack.

It is intended for devices such as Bluetooth headphones, speakers, mice, keyboards, game controllers, and other peripherals whose battery level is exposed to Windows.

## What it does

- polls Windows for Bluetooth devices with a reported battery level;
- shows connected devices in normal operation;
- falls back to devices with an available battery level if Windows cannot provide connection status;
- displays the selected device's level in the tray icon, or the lowest level automatically;
- shows device names and levels in the tray tooltip;
- warns about low battery;
- provides a dark tray menu;
- optionally starts with Windows;
- runs without a regular window and does not appear in `Alt+Tab`;
- is distributed as a standalone executable.

The default polling interval is 60 seconds. A manual refresh is available from the tray menu.

## Download

Download the latest `BtBatteryTray.exe` from [Releases](https://github.com/ProfessorSovers/btbatterytrey/releases).

The application is portable: no installer, .NET runtime, Rust installation, DLLs, or additional folders are required. Copy the executable to any convenient folder and run it.

Windows SmartScreen may show a warning because the executable is not digitally signed. This is common for small independently distributed Windows applications. Download the file only from this repository's Releases page.

## Usage

1. Run `BtBatteryTray.exe`.
2. Open the tray menu by clicking the tray icon.
3. Use **Target** to select the device whose level should be shown in the icon, or use automatic mode.
4. Use **Refresh** to request an immediate update.
5. Enable or disable Windows startup from the same menu.
6. Select **Exit** to close the application.

The application stores its per-user settings in:

```text
HKCU\Software\BtBatteryTray
```

The diagnostic log is stored in:

```text
%LOCALAPPDATA%\BtBatteryTray\log.txt
```

The log is limited to approximately 200 KB and contains device names and battery levels observed by the application. No network service or telemetry is used.

## Limitations

Battery reporting depends on Windows, the Bluetooth adapter, the device, and its driver. A paired device may not appear if it is not connected. A connected device may also be absent when Windows does not expose its battery level.

The application does not communicate with device vendors' cloud services and does not bypass Windows Bluetooth APIs.

For development, `BtBatteryTray.exe --rendertest` renders a sample menu to `menu_test.bmp` in the current directory. This diagnostic mode is not used during normal startup.

## Build from source

Requirements:

- Windows 10 or later;
- Rust and Cargo: <https://rustup.rs/>.

Build the release executable:

```text
cargo build --release
```

The executable is created at:

```text
target\release\BtBatteryTray.exe
```

Run the test suite:

```text
cargo test --locked
```

## License

MIT. See [LICENSE](LICENSE).

## Status

The application is developed and tested on Windows 10. Hardware support depends on whether Windows exposes a battery level for the particular Bluetooth device.
