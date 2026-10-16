# 🦀 OpenVXE

![Rust](https://img.shields.io/badge/Language-Rust-orange.svg)
![Linux](https://img.shields.io/badge/OS-Linux-blue.svg)
![Status](https://img.shields.io/badge/Status-Early_Beta-red.svg)

An open-source, lightweight, and blazing-fast TUI (Terminal User Interface) driver for the **VXE R1 SE+** wireless gaming mouse, written entirely in Rust.

## ✨ Features

* **Native Linux Support:** Direct hardware communication via `hidapi` (no root required with udev rules).
* **Terminal UI:** Beautiful, minimalist interface built with `ratatui`.
* **Reads the real configuration:** the current settings are loaded from the mouse's EEPROM at startup.
* **Live Configuration:** every change is written to the mouse straight away and confirmed by its echo.
  * DPI stages (50 - 10,000 in steps of 50, PAW3395SE sensor), number of stages and active stage
  * Polling Rate (125Hz / 250Hz / 500Hz / 1000Hz)
  * Lift-off Distance (1 mm / 2 mm)
  * Debounce Time (1ms - 20ms)
  * Motion Sync (On/Off)
  * Sleep timer (30 s - 30 min)
  * Angle Snapping (On/Off) and Ripple Control (On/Off)
* **Sleep aware:** while the mouse sleeps the dongle doesn't answer; changes are queued and sent when it wakes up.
* **Hotplug:** the dongle can be unplugged and plugged back in (or plugged in after starting OpenVXE); the configuration is read again on reconnect.
* **Real-time Battery Monitor:** battery level and charging state read with the vendor `GetBatteryLevel` command.
* **Tray Icon:** `openvxe-tray` shows the battery level in the KDE Plasma system tray (or any StatusNotifierItem host) and notifies when it drops below 20%.
* **Hex Sniffer:** Built-in packet monitor for debugging hardware responses.

## 🚀 Installation & Setup

### Arch Linux package

A `PKGBUILD` for `openvxe-git` lives in [packaging/arch](packaging/arch). It builds the latest `main`, installs `openvxe`, `openvxe-tray`, `vxe-probe` and `vxe-decode` to `/usr/bin`, the udev rules to `/usr/lib/udev/rules.d` and the tray icon's autostart entry to `/etc/xdg/autostart`, so steps 1-3 below are not needed:
```bash
cd packaging/arch
makepkg -si
```
Then unplug and replug the USB dongle.

### Fedora package

A spec file lives in [packaging/fedora](packaging/fedora). `make-sources.sh` packs the current commit and its vendored crates (the build runs offline, so it also works in `mock` and COPR), then builds the RPMs if `rpm-build` is installed:
```bash
sudo dnf install rpm-build dnf-plugins-core
sudo dnf builddep packaging/fedora/openvxe.spec
packaging/fedora/make-sources.sh
sudo dnf install packaging/fedora/rpmbuild/RPMS/x86_64/openvxe-0*.rpm
```
It installs the same files as the Arch package. Then unplug and replug the USB dongle.

### 1. Install Dependencies

Ensure you have Rust and the required HID libraries installed.

**Arch Linux:**
```bash
sudo pacman -S rustup base-devel hidapi
rustup default stable
```

**Debian / Ubuntu:**
```bash
sudo apt install build-essential libhidapi-dev pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Fedora:**
```bash
sudo dnf install gcc hidapi-devel pkgconf-pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Install the udev Rules

The included rule gives the logged-in user access to the mouse's `hidraw` nodes, so the driver can run without `sudo`:
```bash
sudo cp udev/70-openvxe.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```
Then unplug and replug the USB dongle.

### 3. Build & Run

```bash
cargo build --release
./target/release/openvxe
```

## 🎮 Usage

| Key | Action |
| --- | --- |
| `↑` / `↓` or `k` / `j` | Select setting |
| `←` / `→` or `h` / `l` | Change value |
| `q` or `Esc` | Quit |

Settings are read from the mouse at startup and applied **live** as soon as you change them. If the mouse is asleep, move it to wake it up: pending changes are sent then, and quitting with changes still pending asks you to press `q` again.

> ℹ️ DPI editing is enabled only for the PAW3395SE sensor (CID/MID 2,31), the only DPI encoding verified so far. Other settings work on any sensor.

### Tray icon

`openvxe-tray` puts the battery percentage in the system tray: green above 50%, orange down to 20%, red below, blue while charging and grey while the mouse sleeps (with the last reading) or the dongle is unplugged. The level is read every 30 seconds, and a notification is sent once each time it drops below 20%. Its menu opens the TUI in Konsole (or `$TERMINAL`). It can run while the TUI is open.

The Arch package starts it at login. Otherwise:
```bash
./target/release/openvxe-tray &
# start it at login
cp packaging/desktop/openvxe-tray.desktop ~/.config/autostart/  # needs openvxe-tray in PATH
```
To build only the TUI, without the tray dependencies: `cargo build --release --no-default-features`.

## 🛠️ Troubleshooting

**"Dongle not connected" / "device not found"**
* OpenVXE looks for the dongle every 2 seconds, so plugging it in is enough.
* Check that the dongle is detected by the system:
  ```bash
  lsusb | grep 373b
  ```
  If nothing shows up, try a different USB port.

**"device found but it cannot be opened"**
* Make sure the udev rules are installed and reloaded (see step 2), then replug the dongle.

**"Waiting for the mouse: move it to wake it up"**
* The mouse is asleep and the dongle doesn't answer. Move it or click a button.

## 🤝 Contributing

Contributions are welcome! See [ROADMAP.md](ROADMAP.md) for known bugs, missing features and planned work before opening an issue or pull request.

Protocol knowledge lives in [docs/PROTOCOL.md](docs/PROTOCOL.md). To help confirm it on your mouse, see [docs/CAPTURE.md](docs/CAPTURE.md): the read-only `vxe-probe --info` and `vxe-probe --dump` commands are the easiest way to start.

## 📄 License

OpenVXE is released under the **GNU General Public License v3.0**. See [LICENSE](LICENSE) for the full text.
