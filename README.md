# BtBatteryTray

**English** · [Русский](README.ru.md) · [Українська](README.uk.md)

A small Windows tray application that shows the battery level of your Bluetooth devices — headphones, speakers, mouse, keyboard, game controller — right in the notification area.

## What it does

- shows **only currently connected** devices;
- draws the battery level of the selected **target** device into the tray icon, or the lowest level automatically;
- lists device names and levels in the tray tooltip;
- warns about a low battery;
- **dark** and **light** menu themes;
- **English**, **Russian** and **Ukrainian** interface;
- optional start with Windows;
- no installer, no runtime, no network access.

The list is refreshed every 60 seconds. **Refresh now** in the menu requests an immediate update.

## Screenshots

| Dark theme | Light theme |
|:---:|:---:|
| ![Menu, dark theme](docs/menu-dark.png) | ![Menu, light theme](docs/menu-light.png) |

Submenus — **Target**, **Language**, **Theme** — open to the left of the main menu, which stays visible:

![Submenu](docs/submenu-dark.png)

*The images above are rendered by the application itself (diagnostic mode `--rendertest`) and are pixel-identical to what appears on screen.*

## Download and run

1. Download `BtBatteryTray.exe` from [Releases](https://github.com/ProfessorSovers/btbatterytrey/releases).
2. Put it in any folder you like and run it.
3. The icon appears in the notification area. Click it to open the menu.

Nothing else is required: no installer, no .NET or Rust runtime, no extra DLLs, no configuration. The executable is self-contained, and your settings are kept in the registry (see below), so you can move or rename the file freely.

To update the application, replace the executable with a newer one and restart it. There is no auto-updater and no background service.

## Usage

Open the menu by clicking the tray icon.

- **Target** — which device the tray icon should follow: a specific device or *Auto (lowest battery)*.
- **Language** — English / Русский / Українська.
- **Theme** — dark or light.
- **Refresh now** — request an immediate update.
- **Start with Windows** — adds or removes the autostart entry.
- **Exit** — quit the application.

The tray tooltip lists every connected device with its level. Hover the icon to see it.

The application stores its settings per user in:

```text
HKCU\Software\BtBatteryTray
```

The diagnostic log is written to:

```text
%LOCALAPPDATA%\BtBatteryTray\log.txt
```

The log is capped at about 200 KB and contains only device names, battery levels and timings observed by the application. No network requests and no telemetry are used anywhere.

## Limitations

Battery reporting is provided by Windows, the Bluetooth adapter, the device and its driver. As a result:

- a paired device is listed only while it is actually connected;
- a connected device may be missing if Windows does not expose its battery level at all;
- the level can be reported with a delay, since the device itself decides how often to send it.

If Windows cannot report connection status at all, the application falls back to showing every device that has an available battery level, instead of showing nothing.

The application does not talk to vendors' cloud services and does not bypass the Windows Bluetooth APIs.

## Diagnostics

Useful when reporting a problem:

```text
BtBatteryTray.exe --conntest      # detected devices, connection verdict and timing
BtBatteryTray.exe --rendertest    # renders the menu to BMP files in the current directory
BtBatteryTray.exe --balloontest   # shows the low-battery notification immediately
                                  # (the percentage is synthetic: the real trigger is
                                  #  below 20%; use --balloontest=5 to change it)
```

The first two modes are standalone: they print or write a file and exit without starting the tray
icon. `--balloontest` shows the notification for a few seconds and exits; it can be run while the
application is already running.

## Build from source

The following is for developers. If you just want to use the application, download the executable from Releases — see above.

Requirements:

- Windows 10 or later (x64);
- Rust and Cargo: <https://rustup.rs/>.

```text
cargo build --release
```

The executable is created at `target\release\BtBatteryTray.exe`.

Run the test suite:

```text
cargo test --locked
```

## License

MIT. See [LICENSE](LICENSE).

## Status

Developed and tested on Windows 10. Whether a particular Bluetooth device reports its battery level is decided entirely by Windows and the device driver.
