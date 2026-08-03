<div align="center">

<img src="logo.png" alt="Legion Linux Toolkit" width="120">

# Legion Linux Toolkit

Daemon, dashboard, tray and CLI for Lenovo Legion laptops on Linux —
built on the **mainline kernel drivers**, not an out-of-tree module.

![version](https://img.shields.io/badge/version-1.0.0--alpha1-red?style=flat-square)
![platform](https://img.shields.io/badge/platform-Linux-blue?style=flat-square)
![language](https://img.shields.io/badge/rust-2024-orange?style=flat-square)
![backend](https://img.shields.io/badge/backend-lenovo--wmi%20(mainline)-blueviolet?style=flat-square)
![license](https://img.shields.io/badge/license-MIT-white?style=flat-square)

</div>

---

## What it is

A root D-Bus daemon (`legiond`) owns all hardware access. A Slint dashboard, a
tray icon and a CLI are all clients of it. Nothing but the daemon writes to
sysfs, and every write is authorized once, by polkit.

```
                  ┌──────────────────────────────────────────┐
  Fn+Q, Fn+Space  │  kernel: lenovo-wmi-gamezone             │
       ─────────► │          lenovo-wmi-other                │
                  │          ideapad_laptop, power_supply    │
                  └────────────────────┬─────────────────────┘
                                       │ sysfs (root-only writes)
                              ┌────────▼────────┐
                              │     legiond     │  systemd, runs as root
                              │  (Rust daemon)  │  polkit-checked methods
                              └────────┬────────┘
                                       │ D-Bus: org.legiontoolkit.Daemon1
              ┌────────────────────────┼────────────────────────┐
              │                        │                        │
      ┌───────▼───────┐        ┌───────▼───────┐        ┌───────▼───────┐
      │  legion-gui   │        │  legion-gui   │        │  legion-ctl   │
      │  (dashboard)  │        │    --tray     │        │     (CLI)     │
      └───────────────┘        └───────────────┘        └───────────────┘
```

Firmware-side changes propagate the same way: pressing **Fn+Q** wakes the
daemon through `EPOLLPRI` on the platform-profile attribute, and one
`PropertiesChanged` signal updates the dashboard, the tray and
`legion-ctl profile --watch` together.

## Why upstream, not LenovoLegionLinux

Earlier versions of this toolkit drove the out-of-tree LenovoLegionLinux (LLL)
`legion_laptop` module. That module does not work on the machine this project
targets — a **Legion R9000P ARX8 (82WM)** hits an EC chip-id mismatch and
reports zero fan-curve points — and its DKMS build broke on kernel 6.14+.

Meanwhile the effort was mainlined. Kernel 6.17+ ships `lenovo-wmi-gamezone`,
`lenovo-wmi-other`, `lenovo-wmi-capdata` and `lenovo-wmi-events`, which expose
everything this hardware actually supports through documented, stable sysfs
ABIs. This rewrite targets those exclusively: **no DKMS, no kernel module to
build, no `legion_cli`.**

## Features

| | |
|---|---|
| **Power modes** | Quiet / Balanced / Performance / Extreme / Custom, with the matching power-button LED colour. Fn+Q stays in sync. |
| **CPU power limits** | SPL, SPPT and FPPT sliders in watts, bounded by the firmware's own min/max. Active in Custom mode only — the kernel enforces that. |
| **Battery** | Conservation, Standard and Rapid charge via `charge_types`, plus charge, health, cycles, draw and voltage. |
| **System switches** | Fn Lock, camera power, always-on USB charging, CPU boost, 3-level keyboard backlight. |
| **Monitoring** | CPU temperature, frequency and governor; fan mode; battery gauge. |
| **Tray** | Profile and battery radio menus, quick toggles, single-instance window. |
| **CLI** | Scriptable `key: value` status output; reads still work without the daemon. |

### Deliberately not included

Fan curves, RGB lighting, display overdrive, G-Sync / hybrid-GPU switching and
GPU overclocking. **The kernel exposes no interface for them on this
hardware**: this machine has the white-only backlight, no fan hwmon, and a BIOS
that advertises only the three `ppt_*` attributes. Anything that would need an
out-of-tree module is out of scope by design.

Features present in the old PyQt build and dropped on purpose: the display page
(brightness/resolution/refresh via kscreen), the desktop theme switcher,
envycontrol GPU switching, ThinkPad and Yoga support, the custom-actions page,
and the 11-language selector. This is an English-only tool for Legion laptops.

## Requirements

- **Kernel 6.17+**; 6.19+ recommended (`charge_types` and the PPT firmware
  attributes). Verified on Ubuntu 26.04's 7.0 kernel.
- `lenovo-wmi-gamezone` loaded — check with
  `grep . /sys/class/platform-profile/*/name`. The index is not fixed: if you
  also run the out-of-tree LenovoLegionLinux module it registers a second
  handler, and the toolkit picks the `lenovo-wmi-gamezone` one by name.
- systemd, D-Bus and polkit.
- Rust 1.93+ to build from source (Ubuntu 26.04's `cargo` is new enough).

**Supported platforms.** First-class: **Ubuntu 26.04 LTS + GNOME (Wayland)** on
a **Legion Pro 5 16ARX8 / R9000P ARX8 (82WM)** — what the project is developed
and tested against. Any laptop exposing the same upstream ABI works; whatever a
machine lacks is hidden rather than failing. Arch Linux is supported through the
PKGBUILD, secondarily.

## Install

### Debian / Ubuntu (recommended)

```bash
sudo apt install cargo build-essential pkg-config libfontconfig-dev \
                 libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev
cargo install cargo-deb
make deb
sudo apt install ./target/debian/*.deb
```

The package enables and starts `legiond` for you.

### Any distro, from source

```bash
sudo bash install.sh
```

Installs build dependencies, builds the workspace as your user, removes anything
a previous Python/LLL install left behind, installs the files, enables the
daemon and starts the tray.

### Arch Linux

```bash
makepkg -si
sudo systemctl enable --now legiond
```

### Uninstall

```bash
sudo bash uninstall.sh
```

## Usage

```bash
legion-ctl status                  # everything, as parseable key: value lines
legion-ctl profile                 # read the current power mode
legion-ctl profile performance     # set it (no password on a local session)
legion-ctl profile --watch         # follow changes, including Fn+Q

legion-ctl ppt                     # list the power limits with their ranges
legion-ctl profile custom          # power limits only apply in Custom mode
legion-ctl ppt spl 90              # set the sustained limit to 90 W

legion-ctl battery long-life       # conservation mode (~60–80% cap)
legion-ctl fnlock on
legion-ctl backlight 2
legion-ctl boost off

legion-gui                         # dashboard
legion-gui --tray                  # tray only (autostarts on login)
```

`legion-ctl` prefers the daemon and falls back to reading sysfs directly when
`legiond` is not running, so `status` works on a fresh checkout.

## Power profiles

| Profile | Kernel value | Power-button LED |
|---|---|---|
| Quiet | `low-power` | Blue |
| Balanced | `balanced` | White |
| Performance | `performance` | Red |
| Extreme | `max-power` | Purple |
| Custom | `custom` | Purple |

## Tray icon support

The tray uses **StatusNotifierItem**, so:

- **Ubuntu GNOME** — works out of the box (`ubuntu-appindicators` ships
  enabled).
- **Vanilla GNOME** — install the
  [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/).
- **KDE Plasma** and most other desktops — native support, nothing to do.

## Permissions

The daemon exposes polkit action `org.legiontoolkit.control` with
`allow_active=yes`: whoever is sitting at the laptop changes their own hardware
settings without a prompt. Inactive sessions — SSH, for instance — must
authenticate as an administrator. No sysfs file is ever made world-writable.

## Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Four crates:

| Crate | What it is |
|---|---|
| `legion-hw` | Typed sysfs backend. Every path resolves against a `SysRoot`, so tests run against a fake device tree with no hardware. |
| `legion-daemon` | `legiond`: the D-Bus service, polkit checks, state persistence, Fn+Q watcher. |
| `legion-ctl` | CLI client, with the direct-sysfs fallback. |
| `legion-gui` | Slint dashboard plus the ksni tray. |

The daemon's integration tests spawn a real `legiond` against a fake sysfs tree
on a private session bus, so they need neither root nor a Legion laptop.

A missing `fontconfig.pc` (that is, no `libfontconfig-dev`) is the usual
first-build failure — install the dependencies listed under
[Install](#debian--ubuntu-recommended).

## Licence

MIT — see [LICENSE](LICENSE).
