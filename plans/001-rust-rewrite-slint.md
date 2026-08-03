# Plan 001: Rewrite Legion Linux Toolkit in Rust (daemon + Slint GUI + ksni tray + CLI) targeting the upstream lenovo-wmi kernel ABI

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat d2c0828..HEAD -- tray/ lib/ scripts/ udev/ polkit/ install.sh Makefile PKGBUILD`
> Note: this plan was written against a dirty working tree at `d2c0828` (several
> files had uncommitted modifications). The plan *replaces* the Python code
> wholesale, so drift in the Python files is acceptable; drift that matters is
> the appearance of any `Cargo.toml` / `crates/` / `src/` Rust code — if Rust
> code already exists in the repo, STOP and report.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED (full rewrite; mitigated by phased steps that keep the Python app untouched until the final step)
- **Depends on**: none
- **Category**: migration
- **Planned at**: commit `d2c0828`, 2026-08-03

## Why this matters

The current toolkit is ~7,700 lines of Python built on PyQt6, with a single
6,609-line GUI file (`tray/legion-gui.py`) containing an 11-language
translation dict, multi-brand (ThinkPad/Yoga/IdeaPad) dead paths, and a
duplicate `ActionsPage` class (defined twice, at `tray/legion-gui.py:5780` and
`:6053` — the first is dead code). Its hardware backend
(`lib/lll_adapter.py`) targets the out-of-tree LenovoLegionLinux (LLL)
`legion_laptop` module, which **does not work on the owner's laptop and does
not build on current kernels**:

- The owner's machine is a **Lenovo Legion R9000P ARX8 (machine type 82WM**,
  China-market Legion Pro 5 16ARX8, AMD Ryzen 7745HX + RTX 4060). LLL issue
  #71 documents this exact model failing: EC chip-id mismatch (expected
  0x8227, reads 0x5507), zero fan-curve points, loads only with `force=1`.
- LLL's last release is v0.0.12-prerelease (2025-03); its DKMS module broke on
  kernel 6.14+ (LLL issue #302) and the machine runs kernel 7.0, where the
  module isn't even present (`modinfo legion_laptop` → not found).
- Meanwhile the LLL effort was **mainlined**: kernel 7.0 on this machine ships
  `lenovo-wmi-gamezone`, `lenovo-wmi-other`, `lenovo-wmi-capdata`,
  `lenovo-wmi-events` and `ideapad_laptop`, which together expose everything
  this laptop actually supports — via stable, documented sysfs ABIs.
- The old stack depended on LLL's C daemon (`legiond`, enabled by
  `install.sh`) plus per-write `pkexec` prompts and world-writable udev
  hacks. The owner wants the daemon rewritten too, in Rust.

The rewrite: a Cargo workspace of four crates — a hardware library, a root
**D-Bus daemon** (`legiond`, systemd service), a CLI, and a Slint GUI with
ksni tray — English-only, targeting the upstream ABI with first-class support
for the 82WM, all redundant features removed. This is the architecture of
asusctl (`asusd` daemon + `rog-control-center` Slint client), the closest
comparable shipping project. "Latest LLL" in 2026 *is* the upstream
`lenovo-wmi-*` driver stack — the successor of the LLL project.

## Current state

### Files being replaced (all Python/bash — will be deleted in the final step)

- `tray/legion-gui.py` (6,609 lines) — PyQt6 dashboard: Home, Battery,
  Performance, Display, Keyboard RGB, System, Overclock, Fan, Actions, About
  pages; embedded base64 logo; `_TR` translation dict at line 114.
- `tray/legion-tray.py` (356 lines) — Qt `QSystemTrayIcon` tray: profile
  cycling, battery/system/fan toggles, launches the GUI by `pkill` + `Popen`.
- `tray/kernel_check.py` (39 lines) — LLL module load checks.
- `lib/lll_adapter.py` (572 lines) — sysfs backend; reads LLL paths under
  `/sys/module/legion_laptop/...` with pkexec fallback; all LLL paths are dead
  on the target machine.
- `scripts/legion-ctl` (bash) — wraps LLL's `legion_cli` (not installed; dead).
- `scripts/legion-helper.sh` — pkexec sysfs-write helper with a path allowlist.
- `udev/99-legion-toolkit.rules` — sets `MODE="0666"` (world-writable!) on
  platform_profile and cpufreq boost, plus RGB USB rules for IDs the target
  laptop doesn't have.
- `polkit/49-legion-toolkit.rules`, `tray/org.legion-toolkit.policy` — polkit
  wiring for the helper.
- `install.sh` (builds LLL from a git clone at install time, enables LLL's C
  `legiond`, copies Python files), `uninstall.sh`, `Makefile`, `PKGBUILD`,
  `legion-linux-toolkit.install`.

The repo's own previous daemon was already deleted (commit 6c7c504); there is
no daemon code to port — the Rust daemon is written fresh. There are **no
tests and no CI** in the current repo; the rewrite must establish both.

### The target laptop's hardware interface (verified live on the machine, kernel 7.0.0-28-generic)

All of the following was confirmed by direct inspection on the 82WM. This is
the complete backend surface for the rewrite:

| Feature | Sysfs path | Semantics (verified) |
|---|---|---|
| Power mode read/write | `/sys/class/platform-profile/platform-profile-0/profile` | RW text. `choices` = `low-power balanced performance max-power custom`. `name` = `lenovo-wmi-gamezone`. Mapping: low-power=Quiet(blue LED), balanced=Balanced(white), performance=Performance(red), max-power=Extreme(purple), custom=Custom(purple). |
| Power-mode change events (Fn+Q) | same `profile` file | `poll()` for `POLLPRI`, then re-read. This is the documented mechanism (`sysfs-class-platform-profile` ABI). No evdev/uevent from `lenovo-wmi-events`. |
| CPU power limits (Custom mode only) | `/sys/class/firmware-attributes/lenovo-wmi-other-0/attributes/{ppt_pl1_spl,ppt_pl2_sppt,ppt_pl3_fppt}/` | Each dir has `type`=integer, `current_value` (RW), `default_value`, `min_value`, `max_value`, `scalar_increment` (=1), `display_name`. Units are **watts**. Verified live: spl 70 [50–115], sppt 85 [60–130], fppt 102 [70–150]. Writes return **EBUSY unless profile is `custom`** (kernel gates it). Writes apply immediately (no `save_settings` — that's think-lmi, a different driver). |
| Battery mode (conservation / rapid charge) | `/sys/class/power_supply/BAT0/charge_types` | RW. Read shows all types with active bracketed, e.g. `Fast Standard [Long_Life]` (live value). Write one of `Standard` (normal), `Fast` (rapid charge), `Long_Life` (conservation ~60–80% cap). Mutually exclusive, enforced by the kernel. **Prefer this over the deprecated `conservation_mode`.** |
| Battery stats | `/sys/class/power_supply/BAT0/{capacity,energy_now,energy_full,energy_full_design,voltage_now,power_now,cycle_count,status}` | Standard power_supply ABI. Note: **no `temp` attribute on this battery** — do not show battery temperature. |
| AC adapter | `/sys/class/power_supply/ADP0/online` | `1` = plugged in. |
| Fn Lock | `/sys/bus/platform/devices/VPC2004:00/fn_lock` | RW `0`/`1` (ideapad_laptop). LED mirror at `/sys/class/leds/platform::fnlock/brightness`. |
| Camera power | `/sys/bus/platform/devices/VPC2004:00/camera_power` | RW `0`/`1`. |
| Always-on USB charging | `/sys/bus/platform/devices/VPC2004:00/usb_charging` | RW `0`/`1`. |
| Fan mode (ideapad) | `/sys/bus/platform/devices/VPC2004:00/fan_mode` | RW int: 0=Super Silent, 1=Standard, 2=Dust Cleaning, 4=Efficient Thermal Dissipation (3 invalid). Live value 0. |
| Keyboard backlight | `/sys/class/leds/platform::kbd_backlight/{brightness,max_brightness,brightness_hw_changed}` | 3 levels: 0/1/2 (`max_brightness`=2 verified). This laptop has the **white-only** keyboard (USB `048d:c103`, "ITE Device(8910)") — there is **no 4-zone RGB on this machine** and no userspace tool supports c103. `brightness_hw_changed` fires on Fn+Space. |
| CPU boost | `/sys/devices/system/cpu/cpufreq/boost` | RW `0`/`1` (AMD acpi-cpufreq/amd-pstate path). |
| CPU governor / EPP / freq | `/sys/devices/system/cpu/cpu0/cpufreq/{scaling_governor,energy_performance_preference,scaling_cur_freq}` | RO display only. |
| CPU temp | hwmon named `k10temp` → `temp*_input` (millidegrees) | Search `/sys/class/hwmon/hwmon*/name` for `k10temp`. |
| GPU (dGPU) presence | lspci: NVIDIA RTX 4060 + AMD Raphael iGPU | Display-only info; no control interfaces in scope. |
| Fan RPM | **absent** | No fan hwmon exists on this kernel (verified: no `fan1_input` anywhere). Newer mainline adds a `lenovo-wmi-other` hwmon with `fanX_input`/`fanX_target` — feature-detect it so the UI lights up when the kernel gains it. |

All privileged sysfs files above are `root`-writable only (mode 0644, owner
root). In the new architecture, **only the daemon writes them** (it runs as
root under systemd); clients go through D-Bus.

### What upstream does NOT provide (and therefore is out of the rewrite)

- Multi-point **fan curves** — LLL-only, and LLL's fan curve is broken on this
  model (0 points). No upstream interface exists.
- **Display Overdrive**, **G-Sync/hybrid toggle**, `fan_fullspeed`, `winkey`,
  `touchpad`, `rapidcharge` LLL sysfs nodes — none exist on this machine
  (rapid charge is covered by `charge_types` instead; `touchpad` is absent
  from this machine's VPC2004:00 — feature-detect and hide).
- GPU overclock attributes (`dgpu_boost_clk`, `gpu_nv_ctgp`, …) exist in
  newer mainline `lenovo-wmi-other` but are **not advertised by this BIOS**
  (only the three `ppt_*` attributes appear). Feature-detect by directory
  listing; do not hardcode.

### Decisions already made (do not relitigate)

Decided by the repo owner on 2026-08-03:

1. **GUI framework: Slint** (v1.17+, `slint` crate). Precedent: ASUS's
   rog-control-center (asusctl) — the closest comparable app — ships Slint +
   winit-Wayland + femtovg + `ksni` tray. Use the same shape.
2. **English only.** No translation system. Keep UI strings as plain literals.
3. **Rewrite includes a daemon** (owner's explicit instruction): a Rust
   system daemon (`legiond`) replaces both LLL's C `legiond` and the
   pkexec-per-write model.
4. **Removed features** (redundant): Display page (brightness/resolution/
   refresh/VRR via kscreen/xrandr/gsettings), desktop theme switcher,
   envycontrol GPU-mode switching, all ThinkPad support (charge thresholds,
   fan levels, TrackPoint), all Yoga support (hinge mode, auto-rotate),
   fingerprint, multi-brand first-run wizard, keyboard RGB page (hardware
   absent), "L1 AI Engine" EPP toggle, the 11-language i18n, Overclock page's
   LLL-only controls (replaced by the three PPT sliders), the "Actions" page
   (custom user scripts), and the LLL install/force-load machinery in
   `install.sh`.
5. **Backend: upstream kernel ABI only.** No LLL library, no `legion_cli`, no
   DKMS. LLL sysfs paths must not appear in the Rust code.
   (Terminology, to avoid confusion: "LLL" = the LenovoLegionLinux project —
   an out-of-tree kernel module `legion_laptop.ko` + Python library/CLI + a
   small C daemon named `legiond`. None of it is used here. The daemon in
   this plan is **new Rust code**, which reuses the good name `legiond`.)
6. **Primary platform: Ubuntu 26.04 LTS + GNOME (Wayland).** Verified on the
   owner's machine: GNOME Shell 50.1, Wayland session, `ubuntu-appindicators`
   extension enabled by default, apt cargo candidate 1.93.1. First-class =
   .deb packaging, tray working on stock Ubuntu GNOME, MSRV buildable with
   Ubuntu's rustc. Arch (PKGBUILD) remains supported but secondary.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Toolchain check | `cargo --version` | ≥ 1.97 (1.97.1 verified installed) |
| Build | `cargo build --workspace` | exit 0 |
| Tests | `cargo test --workspace` | exit 0, all pass |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, no warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| Run daemon in foreground (target laptop) | `sudo target/debug/legiond --foreground` | logs "listening on system bus" |
| Run GUI (target laptop) | `cargo run -p legion-gui` | window opens |

Slint needs no system packages for the femtovg renderer beyond a working
OpenGL/Wayland session; `ksni` and `zbus` are pure Rust (no libdbus needed).

## Suggested executor toolkit

Reference material (read-only; do not vendor code from these):

- **asusctl / rog-control-center** (github.com/OpenGamingCollective/asusctl,
  MPL-2.0) — the architectural template: `asusd` root daemon over zbus +
  Slint client (`backend-winit-wayland` + femtovg) + `ksni` tray + typed
  sysfs wrappers (`rog-platform` crate) + `logind-zbus` for sleep/resume.
  When unsure how to structure the daemon interface, a Slint page, or the
  tray, look there.
- **Windows Lenovo Legion Toolkit** (github.com/BartoszCichecki/LenovoLegionToolkit,
  archived 2025-07) — the owner confirms it works with this laptop on
  Windows. It drives the same GameZone/Other WMI firmware interfaces the
  Linux `lenovo-wmi-*` drivers wrap, and confirms the firmware feature set:
  Quiet/Balance/Performance/Custom modes, Custom-mode power-limit tuning
  (Gen 7+; the 82WM is Gen 8), conservation/charging modes, and the
  **3-level white backlight** variant. Use it as a UX benchmark and for WMI
  method semantics only — its extra features (hybrid GPU mode, overdrive,
  Spectrum RGB, automation actions) have no Linux kernel interface on this
  machine and stay out of scope.
- Kernel ABI docs: `Documentation/ABI/testing/sysfs-class-platform-profile`,
  `sysfs-class-firmware-attributes`, `sysfs-platform-ideapad-laptop`,
  `sysfs-class-power` (charge_types), and
  `Documentation/wmi/devices/lenovo-wmi-gamezone.rst` /
  `lenovo-wmi-other.rst` in the Linux source — the authoritative value
  formats, mirrored in the hardware table above.

## Scope

**In scope** (create/modify):
- `Cargo.toml` (workspace root), `rust-toolchain.toml`, `.gitignore` (add `/target`)
- `crates/legion-hw/**` (new — backend library)
- `crates/legion-daemon/**` (new — `legiond` D-Bus system daemon)
- `crates/legion-ctl/**` (new — CLI binary)
- `crates/legion-gui/**` (new — Slint GUI + ksni tray binary)
- `.github/workflows/ci.yml` (new)
- `dbus/org.legiontoolkit.Daemon.conf` (new — system-bus policy)
- `systemd/legiond.service` (new)
- `desktop/legion-toolkit.desktop`, `desktop/legion-toolkit-tray.desktop` (new)
- `polkit/org.legion-toolkit.policy`, `polkit/49-legion-toolkit.rules` (rewrite)
- `Makefile`, `PKGBUILD`, `install.sh`, `uninstall.sh`, `README.md` (rewrite —
  **exact contents for these are given in Appendix A; use them verbatim**)
- **Deletion (final step only)**: `tray/`, `lib/`, `scripts/`,
  `udev/99-legion-toolkit.rules`, `legion-linux-toolkit.install`

**Out of scope** (do NOT touch):
- `screenshots/` — stale but harmless; replacing screenshots needs a human.
- `LICENSE`, `logo.png` (reuse the PNG as the app/tray icon asset — copy,
  don't edit).
- Any interface for fan curves, RGB, overclocking beyond the three PPT
  attributes — the hardware/kernel doesn't support them (see Current state).
- Writing to any sysfs path not listed in the hardware table above.

## Git workflow

- Branch: `advisor/001-rust-rewrite-slint` (branch from `main`; note the
  working tree at planning time had uncommitted changes — commit or stash them
  first and report what they were).
- Commit per step, conventional-commit style matching repo history (e.g.
  `feat: add legion-hw sysfs backend crate`, like existing
  `feat: add Makefile, PKGBUILD, and install script…`).
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Scaffold the Cargo workspace + CI

Create:

```
Cargo.toml                # [workspace] members = ["crates/*"], resolver = "2"
rust-toolchain.toml       # [toolchain] channel = "stable"
crates/legion-hw/         # cargo new --lib
crates/legion-daemon/     # cargo new (bin name: legiond)
crates/legion-ctl/        # cargo new (bin)
crates/legion-gui/        # cargo new (bin)
.github/workflows/ci.yml
```

Workspace `Cargo.toml` also carries `[workspace.package]` (version = "1.0.0-alpha1",
license = "MIT", edition = "2024", **rust-version = "1.93"** — Ubuntu 26.04's
apt cargo candidate; keeps a pure-apt build possible) and
`[workspace.dependencies]` pinning:
`slint = "1.17"`, `ksni = "0.3"`, `zbus = "5"`, `logind-zbus = "5"`,
`clap = { version = "4", features = ["derive"] }`, `thiserror = "2"`,
`serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`,
`tokio = { version = "1", features = ["rt", "macros", "signal"] }` (zbus 5's
default async runtime pairing), `tempfile = "3"` (dev), `assert_cmd = "2"` (dev).
Add `/target` to `.gitignore`.

CI (`.github/workflows/ci.yml`): on push/PR — `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, on `ubuntu-latest` with `dtolnay/rust-toolchain@stable`.
Add `dbus-daemon` availability note: the daemon integration test (step 3)
uses `dbus-run-session`, present on ubuntu-latest.

**Verify**: `cargo build --workspace && cargo test --workspace` → exit 0
(empty crates compile; default tests pass).

### Step 2: `legion-hw` — sysfs core with a test-injectable root

The whole backend is a library crate that never hardcodes `/sys` at call
sites. Central type:

```rust
// crates/legion-hw/src/sysfs.rs
pub struct SysRoot(PathBuf);            // default: SysRoot::system() -> "/"
impl SysRoot {
    pub fn system() -> Self { Self(PathBuf::from("/")) }
    pub fn at(root: impl Into<PathBuf>) -> Self { Self(root.into()) }  // tests
    pub(crate) fn path(&self, rel: &str) -> PathBuf { self.0.join(rel) }
}
```

Every module takes `&SysRoot` and joins **relative** paths
(`sys/class/platform-profile/...`). Tests build a fake tree under a
`tempfile::TempDir` and use `SysRoot::at(tmp.path())`. This is the standard
pattern (see asusctl's `rog-platform` crate for a reference implementation of
typed sysfs wrappers — read-only reference, do not vendor it; it is MPL-2.0).

Error type: one `thiserror` enum `HwError { NotSupported, PermissionDenied,
Busy, Io(#[from] io::Error), Parse(String) }`. Map `EBUSY` from
firmware-attribute writes to `HwError::Busy` (clients turn this into "switch
to Custom mode first").

Modules and their public API (all feature-detecting — return
`Err(NotSupported)` or `None` when the path is absent, never panic):

- `profile.rs` — `PowerProfile` enum { Quiet, Balanced, Performance, Extreme,
  Custom } ↔ sysfs strings `low-power`/`balanced`/`performance`/`max-power`/
  `custom`; `choices()`, `get()`, `set()`. Include the LED-color metadata
  (blue/white/red/purple/purple) as a method — the tray uses it.
- `profile_watch.rs` — blocking `wait_for_change(&SysRoot, timeout)` using
  `poll(2)` with `POLLPRI | POLLERR` on the `profile` file (open, read to
  clear, poll, re-read). Use the `rustix` or `nix` crate for `poll`. Only the
  **daemon** runs this watcher; clients receive D-Bus signals.
- `firmware_attrs.rs` — `PptAttr` { Spl, Sppt, Fppt } with
  `read(&SysRoot, attr) -> AttrInfo { current, default, min, max }` and
  `write(&SysRoot, attr, watts)`; directory-scan `available()` so future
  attributes (e.g. `dgpu_boost_clk`) are visible via `list_all()`.
- `battery.rs` — `ChargeMode` { Standard, Fast, LongLife } parse/format for
  `charge_types` (parse the bracketed-active format, e.g.
  `Fast Standard [Long_Life]`); `stats()` reading the BAT0 attributes in the
  hardware table (skip absent files gracefully); `ac_online()` scanning
  `sys/class/power_supply/*/online` for name prefix ADP/AC.
- `ideapad.rs` — fn_lock, camera_power, usb_charging, fan_mode get/set at
  `sys/bus/platform/devices/VPC2004:00/`; each `Option`-returning if absent.
- `kbd_backlight.rs` — get/set/max over
  `sys/class/leds/platform::kbd_backlight/`.
- `cpu.rs` — boost get/set, governor/EPP/cur-freq getters, `temp()` scanning
  hwmon for name `k10temp`.
- `fan.rs` — `rpm()` scanning hwmon for any `fan[12]_input` (returns
  `None` today; lights up when the kernel gains the lenovo-wmi hwmon).
- `detect.rs` — `Capabilities` struct (serde-serializable — the daemon sends
  it to clients over D-Bus as JSON) built by probing every path once at
  startup. Also `is_legion()` via `sys/class/dmi/id/product_family`
  containing "Legion" (warn, don't refuse, when unmatched).

Writes in this crate write sysfs directly; they succeed for root (the
daemon) and fail with `PermissionDenied` otherwise.

**Verify**: `cargo test -p legion-hw` → all pass (tests per Test plan,
≥ 20 tests). `cargo clippy -p legion-hw --all-targets -- -D warnings` → clean.

### Step 3: `legion-daemon` — the `legiond` D-Bus system service

Binary name `legiond` (LLL's C daemon of the same name is gone with LLL; note
a `conflicts=('lenovolegionlinux')` in PKGBUILD). Runs as root under systemd.
Built on `zbus` 5 + tokio. Structure:

```
crates/legion-daemon/
  src/main.rs        # clap: --foreground, --sysfs-root <path> (hidden, tests),
                     # --bus {system|session} (hidden; session used by tests)
  src/iface.rs       # the zbus #[interface] implementation
  src/state.rs       # persisted state: /var/lib/legiond/state.json
  src/watch.rs       # profile POLLPRI watcher thread → signal emission
```

D-Bus surface — well-known name `org.legiontoolkit.Daemon`, object path
`/org/legiontoolkit/Daemon`, interface `org.legiontoolkit.Daemon1`:

- Properties (all with `emits_changed_signal`): `Profile` (s),
  `ProfileChoices` (as), `BatteryMode` (s), `FnLock` (b), `CameraPower` (b),
  `UsbCharging` (b), `KbdBacklight` (u), `CpuBoost` (b),
  `Capabilities` (s — JSON of `legion_hw::detect::Capabilities`).
- Methods: `SetProfile(s)`, `SetBatteryMode(s)`, `SetFnLock(b)`,
  `SetCameraPower(b)`, `SetUsbCharging(b)`, `SetKbdBacklight(u)`,
  `SetCpuBoost(b)`, `GetPpt() → s` (JSON array of AttrInfo),
  `SetPpt(s attr, u watts)`, `Status() → s` (JSON dump for the CLI).
- Errors: map `HwError::Busy` to a named D-Bus error
  `org.legiontoolkit.Error.CustomModeRequired` so clients can show the right
  message.

Behaviors:

- **Authorization**: every `Set*` method checks the caller with polkit action
  `org.legiontoolkit.control` via D-Bus calls to
  `org.freedesktop.PolicyKit1.Authority.CheckAuthorization` (use the
  `zbus_polkit` crate). The policy defaults `allow_active=yes` — local,
  active-session users control their own laptop with no prompt; remote/
  inactive sessions get denied. Read-only properties are unauthenticated.
- **Fn+Q propagation**: the `watch.rs` POLLPRI watcher re-reads the profile
  and emits `PropertiesChanged` on `Profile` — GUI/tray update live.
- **State restore**: persist last-set profile, battery mode, and PPT values
  to `/var/lib/legiond/state.json` (serde). Reapply on daemon start and on
  resume-from-sleep (subscribe to logind `PrepareForSleep(false)` via
  `logind-zbus`). Battery mode and PPT are the ones firmware forgets;
  reapplying profile is cheap and harmless.
- **Session-bus test mode**: `--bus session --sysfs-root <fake>` runs the
  identical interface on the session bus against a fake sysfs tree — this is
  what integration tests use; no root needed.

`systemd/legiond.service`:
`[Service] Type=dbus BusName=org.legiontoolkit.Daemon ExecStart=/usr/bin/legiond`,
hardening directives (`ProtectHome=yes`, `PrivateTmp=yes`,
`ReadWritePaths=/sys /var/lib/legiond`), `[Install] WantedBy=multi-user.target`.

`dbus/org.legiontoolkit.Daemon.conf` (system-bus policy): root may own
`org.legiontoolkit.Daemon`; everyone may send to it (polkit does the real
gating).

**Verify**:
- `cargo test -p legion-daemon` → passes, including the integration test:
  `dbus-run-session -- cargo test -p legion-daemon --test dbus_roundtrip`
  pattern — the test spawns `legiond --bus session --sysfs-root <fake 82wm>`,
  calls `SetProfile("performance")` over zbus, asserts the fake sysfs file
  now contains `performance` and a `PropertiesChanged` signal arrived.
- On the target laptop: `sudo target/debug/legiond --foreground` in one
  terminal; in another,
  `busctl --system get-property org.legiontoolkit.Daemon /org/legiontoolkit/Daemon org.legiontoolkit.Daemon1 Profile`
  → current profile string. Press Fn+Q → daemon log shows the change and
  `busctl` re-query reflects it.

### Step 4: `legion-ctl` — CLI as a D-Bus client

`clap`-derive CLI, subcommands (keep exactly this surface):

```
legion-ctl status                      # daemon Status() — capability-aware summary
legion-ctl profile [quiet|balanced|performance|extreme|custom]  # get/set
legion-ctl profile --watch             # subscribe to PropertiesChanged, print changes
legion-ctl ppt [spl|sppt|fppt] [WATTS] # get (with range) / set
legion-ctl battery [standard|fast|long-life]
legion-ctl fnlock|camera|usb-charging [on|off]
legion-ctl backlight [0|1|2]
legion-ctl boost [on|off]
```

Connection logic, implemented once: try the system-bus daemon; if the name
isn't owned, fall back to **direct sysfs** via `legion-hw` (works for reads
always, for writes when root) and print a one-line hint
`legiond not running — using direct sysfs (systemctl enable --now legiond)`.
Hidden flags `--bus`/`--sysfs-root` (clap `hide = true`) route tests.
`status` output is plain `key: value` lines (parseable).

**Verify**:
- `cargo test -p legion-ctl` → all pass (assert_cmd tests, see Test plan).
- On the target laptop with legiond running: `legion-ctl status` → prints
  profile, battery mode, ppt ~70/85/102; `legion-ctl profile performance` →
  no prompt (active session), `cat /sys/class/platform-profile/platform-profile-0/profile`
  → `performance`; set it back. `legion-ctl profile --watch` + Fn+Q → prints
  the new profile.

### Step 5: `legion-gui` — Slint dashboard (D-Bus client)

Structure (match rog-control-center's shape, simplified):

```
crates/legion-gui/
  build.rs                 # slint_build::compile("ui/app.slint")
  ui/app.slint             # main window: sidebar + page stack, one global Palette
  ui/pages/home.slint      # profile selector, live stats, quick toggles
  ui/pages/battery.slint   # charge mode radio (Standard/Fast/LongLife), stats
  ui/pages/custom.slint    # PPT sliders (enabled only when profile==custom)
  ui/pages/system.slint    # fn_lock, camera, usb_charging, kbd backlight, fan info
  ui/pages/about.slint
  src/main.rs              # wiring; --tray flag
  src/bus.rs               # zbus proxy to org.legiontoolkit.Daemon1 + signal task
  src/state.rs             # AppState: maps daemon state → Slint properties
  src/tray.rs              # ksni tray (step 6)
  assets/logo.png          # copied from repo root
```

Slint config in `Cargo.toml`:
`slint = { workspace = true, default-features = false, features = ["backend-winit-wayland", "backend-winit-x11", "renderer-femtovg", "compat-1-2", "std"] }`
(the asusctl-proven combo).

Rules that keep this maintainable:
- **All hardware state comes from the daemon** via the `bus.rs` proxy; the
  GUI never touches sysfs (read-only stats like CPU temp/freq may use
  `legion-hw` getters directly — they're unprivileged — via the same
  `state.rs` tick). D-Bus calls run on a tokio task;
  UI updates cross via `slint::invoke_from_event_loop` (Slint UI types are
  not `Send`; never block the UI thread on D-Bus).
- Pages bind to a flat set of Slint properties on the main window
  (`current-profile`, `ppt-spl`, `battery-mode`, `cpu-temp`, …).
- PPT sliders: min/max/value from `GetPpt()`; disabled with an explanatory
  label unless profile == Custom; a `CustomModeRequired` D-Bus error shows a
  toast "Switch to Custom mode first".
- `PropertiesChanged` subscription keeps GUI (and tray) in sync with Fn+Q
  and CLI changes — no client-side POLLPRI.
- If the daemon name is not on the bus: show a single full-window notice
  "legiond is not running — sudo systemctl enable --now legiond" instead of
  the dashboard (no silent fallback in the GUI; keeps one code path).
- Window title "Legion Toolkit"; `logo.png` as the icon. Do not port the old
  app's per-widget stylesheet strings.

**Verify**: `cargo build -p legion-gui` → exit 0. `cargo test -p legion-gui`
→ state-layer tests pass. On the target laptop:
`cargo run -p legion-gui` → window opens on Wayland, shows current profile;
switching profile in the GUI is reflected by
`cat /sys/class/platform-profile/platform-profile-0/profile`, and pressing
Fn+Q updates the GUI within a second.

### Step 6: Tray via ksni (same process)

Add `src/tray.rs` using `ksni` (StatusNotifierItem). On the primary platform
— Ubuntu GNOME — this works out of the box: Ubuntu ships the
`ubuntu-appindicators` extension enabled by default (verified on the target,
GNOME Shell 50.1). Vanilla/non-Ubuntu GNOME needs the AppIndicator extension;
KDE Plasma supports SNI natively — note both in README. The GUI binary takes
`--tray`:

- `legion-gui --tray` starts hidden with the tray item; left-click toggles the
  window; menu: profile radio group (from `ProfileChoices`), battery mode
  radio, Fn Lock / Camera checkboxes, "Open Dashboard", "Quit". All actions
  call the daemon proxy; `PropertiesChanged` refreshes the menu state.
- Tray icon: the logo with a colored status dot per profile (LED color
  mapping from `legion_hw::profile`). Ship 5 pre-rendered PNGs under
  `assets/` — generate once and commit them.
- Single-instance: bind an abstract Unix socket (`@legion-toolkit.lock`); if
  taken, signal the owner to show its window (write a byte). Replaces the old
  `pkill` hack in `legion-tray.py:231`.
- Autostart `.desktop` runs `legion-gui --tray`.

**Verify**: `cargo clippy -p legion-gui --all-targets -- -D warnings` → clean.
On the target laptop (KDE/GNOME session): `cargo run -p legion-gui -- --tray`
→ tray icon appears, menu switches profile, second invocation raises the
window instead of duplicating the process.

### Step 7: Privilege wiring — polkit for the daemon, delete udev hacks

Delete the udev approach entirely (`udev/99-legion-toolkit.rules` made
`platform_profile` and cpufreq boost **world-writable**, and whitelists RGB
USB IDs this laptop doesn't have — both wrong). The daemon is the only
writer; authorization is polkit-checked per method call (step 3). Files:

- `polkit/org.legion-toolkit.policy` (rewrite): one action
  `org.legiontoolkit.control`, `allow_active` = `yes`, `allow_inactive` =
  `auth_admin`, `allow_any` = `no`. (Active local users control their own
  laptop without prompts; SSH/remote sessions must authenticate.)
- `polkit/49-legion-toolkit.rules`: delete or reduce to a comment — the
  group-based rule is no longer needed since `allow_active=yes` covers the
  desktop case. Keep the file only if the owner wants passwordless control
  from inactive sessions for wheel users; default: delete.

**Verify**: as the desktop user: `legion-ctl profile balanced` → no prompt,
profile changes. Over SSH (inactive session): the same command → polkit
denial/auth error, and profile unchanged.
`ls -l /sys/class/platform-profile/platform-profile-0/profile` → still
`0644 root root` (no udev widening).

### Step 8: Packaging — user-facing files, written verbatim from Appendix A

**Appendix A at the end of this plan contains the complete contents of every
file in this step** — `install.sh`, `uninstall.sh`, `Makefile`,
`systemd/legiond.service`, `dbus/org.legiontoolkit.Daemon.conf`,
`polkit/org.legion-toolkit.policy`, both `.desktop` entries. Create them with
those exact contents (adjust only if a STOP condition forces it, and
document the deviation). Summary of what they do:

- **`install.sh`** — Ubuntu-first: installs build deps via apt (`cargo`,
  `build-essential`, `pkg-config`; pacman fallback for Arch), builds the
  workspace as the invoking user, runs the **legacy cleanup** (removes every
  artifact the old Python/LLL installer left on a system: old
  `/usr/lib/legion-toolkit` Python tree, `legion_cli` shims,
  `/etc/modprobe.d/legion_laptop_force.conf`, the 0666 udev rules, old
  polkit rules, DKMS `legion-laptop` module if registered — the old
  installer also ran `chmod a+w` on sysfs, which does not persist across
  reboots, so no cleanup is possible or needed for that), then
  `make install`, enables `legiond`, and starts the tray for the invoking
  user. **No LLL clone, no DKMS build, no modprobe, no pip.**
- **`uninstall.sh`** — stops the GUI/daemon, `make uninstall`, removes
  `/var/lib/legiond`, and runs the same legacy cleanup.
- **`Makefile`** — `build` / `deb` / `install` / `uninstall`; installs the
  three binaries, unit, D-Bus conf, polkit policy, desktop entries, icon.
  No udev lines anywhere.
- **.deb** via `cargo-deb` (`cargo install cargo-deb`; `make deb` runs
  `cargo deb -p legion-daemon`): add `[package.metadata.deb]` to
  `crates/legion-daemon` (package name `legion-linux-toolkit`) with `assets`
  mirroring the Makefile's install list and cargo-deb's `systemd-units`
  support so `legiond.service` is enabled/started on install. Verify on the
  target: `sudo apt install ./target/debian/*.deb`.
- **`PKGBUILD`** (secondary platform): `arch=('x86_64')`,
  `makedepends=('cargo')`, minimal `depends`,
  `conflicts=('lenovolegionlinux')` (binary name `legiond` collides),
  standard `cargo build --release --locked`, package() mirroring the
  Makefile install list; replace `legion-linux-toolkit.install` with a
  post-install message enabling the unit.
- `README.md`: rewrite — architecture diagram (daemon ↔ D-Bus ↔ GUI/tray/CLI;
  upstream drivers, not LLL), supported platform statement ("first-class:
  Ubuntu 26.04 LTS + GNOME Wayland on Legion Pro 5 16ARX8 / R9000P ARX8
  (82WM); anything exposing the same upstream ABI works; Arch supported"),
  requirements: "kernel 6.19+ recommended (`charge_types`, PPT attributes),
  Ubuntu 26.04's 7.0 kernel verified; `lenovo-wmi-gamezone` required";
  tray note (stock Ubuntu GNOME works; vanilla GNOME needs the AppIndicator
  extension); build/install instructions (.deb first, then make/PKGBUILD).

**Verify**: `make deb && sudo apt install ./target/debian/*.deb &&
systemctl status legiond` → active. `legion-ctl status` → works.
`bash -n install.sh uninstall.sh` → exit 0; `makepkg --printsrcinfo` (only
if on Arch; otherwise skip) → exit 0.

### Step 9: Delete the Python implementation

Only after steps 1–8 verify on the target laptop:

```
git rm -r tray/ lib/ scripts/ udev/ legion-linux-toolkit.install
```

Search for stragglers:
`grep -rn "legion_laptop\|legion_cli\|lll\|LLL\|pyqt\|PyQt" --include='*.rs' --include='*.md' --include='Makefile' --include='PKGBUILD' --include='*.sh' .`
→ only historical mentions in README's "why upstream" note and the plans/
directory are acceptable.

**Verify**: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
→ all exit 0. `git status` → only in-scope paths changed/deleted.

## Test plan

Framework: **built-in `cargo test`** with `tempfile` fake sysfs trees,
`assert_cmd` for the CLI, and a session-bus integration test for the daemon
(`dbus-run-session`). No further test-framework crates.

`crates/legion-hw/tests/` (one file per module), each building a fake tree:

```rust
fn fake_82wm(tmp: &TempDir) { // helper in tests/common/mod.rs
    // writes sys/class/platform-profile/platform-profile-0/{name,choices,profile},
    // sys/class/firmware-attributes/lenovo-wmi-other-0/attributes/ppt_pl1_spl/{...},
    // sys/class/power_supply/BAT0/{charge_types,...}, VPC2004:00 attrs,
    // leds, cpufreq, k10temp hwmon — exactly the live values recorded in
    // the "Current state" table of plans/001-rust-rewrite-slint.md.
}
```

Required legion-hw cases (≥ 20):
- profile: get maps `low-power`→Quiet; set writes `max-power` for Extreme;
  unknown string in file → `Parse` error; missing file → `NotSupported`;
  choices parsing.
- firmware_attrs: read returns {70, 70, 50, 115} for spl from fake; write out
  of range rejected client-side with `Parse`; EBUSY errno → `Busy` (unit-test
  the errno mapping); absent attr dir → `NotSupported`.
- battery: parse `Fast Standard [Long_Life]` → LongLife active, all three
  available; parse `[Standard]` alone; write formats `Long_Life` with
  underscore; stats skip missing `temp` without error; capacity computed from
  energy_now/energy_full.
- ideapad: each toggle round-trips 0/1; fan_mode rejects 3; absent attr →
  None (hidden).
- kbd_backlight: set above max_brightness → error.
- cpu: k10temp discovery among multiple hwmons (fake a `nvme` hwmon too, to
  prove name filtering); millidegree conversion.
- fan: None on the 82WM fake; Some when a fake `fan1_input` exists.
- detect: Capabilities on the full 82WM fake has everything except fan RPM;
  on an empty tree has nothing and nothing panics; serializes to JSON.

`crates/legion-daemon/tests/dbus_roundtrip.rs`: run under a private session
bus (`--bus session --sysfs-root <fake>`, polkit check skipped on session
bus): SetProfile writes the fake file and emits PropertiesChanged;
SetPpt on non-custom profile returns the `CustomModeRequired` error name;
state.json is written and reapplied on daemon restart.
`crates/legion-daemon/src/state.rs`: unit tests for state (de)serialization.

`crates/legion-ctl/tests/cli.rs` (assert_cmd): `status` in direct-sysfs mode
(`--sysfs-root <fake>`) → exit 0, contains `profile: balanced`; write
subcommand against a read-only fake as non-root → non-zero exit with a
permission message; argument validation (`ppt spl 20` below min → error
naming the 50–115 range).

`crates/legion-gui/src/state.rs`: unit tests for pure mapping functions
(profile → LED color/icon name, slider bounds from AttrInfo). UI rendering is
not unit-tested (needs a display) — on-hardware smoke checks in steps 5–6.

Verification: `cargo test --workspace` → all pass; count ≥ 30 total.

## Done criteria

- [ ] `cargo build --workspace --release` exits 0
- [ ] `cargo test --workspace` exits 0 with ≥ 30 tests
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --all --check` exits 0
- [ ] `grep -rn "legion_laptop\|legion_cli" crates/` → no matches (no LLL paths)
- [ ] `grep -rn "0666" polkit/ Makefile install.sh` → no matches; `udev/` deleted
- [ ] `grep -c "legion_laptop_force\|legion_cli\|dkms" install.sh uninstall.sh`
      → ≥ 1 match per file, **all inside the `legacy_cleanup` function only**
      (removal logic, not installation logic)
- [ ] `bash -n install.sh uninstall.sh` → exit 0
- [ ] On the target laptop: `systemctl status legiond` active; `legion-ctl
      status`, profile set + `--watch` + Fn+Q propagation, battery mode set,
      PPT set (custom mode), backlight set all work as described in steps 3–7
- [ ] `tray/`, `lib/`, `scripts/` no longer exist; `git status` shows only
      in-scope changes
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- Rust code already exists in the repo at start (drift check).
- `/sys/class/platform-profile/platform-profile-0/name` on the target machine
  is not `lenovo-wmi-gamezone`, or the firmware-attributes directory
  `lenovo-wmi-other-0` is absent — the kernel/driver landscape changed and
  the backend assumptions in "Current state" no longer hold.
- Writing `custom` to the profile file, then writing a PPT `current_value`,
  still returns EBUSY on the target machine — the gating model differs from
  documentation; report the errno and dmesg tail.
- `slint` 1.17+ fails to compile on stable Rust 1.97 or the winit-Wayland
  backend cannot create a window in the target session — report before
  swapping renderers/backends.
- `ksni` produces no visible tray item on the target desktop — check whether
  the session has StatusNotifierItem support; report findings first.
- `zbus_polkit`'s CheckAuthorization does not distinguish active vs inactive
  sessions as expected — report before weakening the policy.
- Any step's verification fails twice after a reasonable fix attempt.
- You are tempted to add a feature not in the hardware table (fan curves,
  RGB, overdrive, G-Sync, envycontrol) — these are decided-out; report
  instead.

## Maintenance notes

- **Kernel evolution is the main moving part.** Newer mainline adds fan RPM
  hwmon (`lenovo-wmi-other` hwmon with `fanX_input`/`fanX_target`) and more
  firmware attributes (`dgpu_boost_clk`, `gpu_nv_ctgp`, `gpu_mode`, …). The
  feature-detect design (`detect.rs`, `firmware_attrs::list_all()`,
  `fan::rpm()`) is what lets those light up without code changes — a reviewer
  should reject any PR that hardcodes attribute lists instead of scanning.
- The EBUSY-unless-custom gate for PPT writes is kernel-enforced policy; if a
  future kernel relaxes it, only the slider-enable condition in
  `custom.slint` and the daemon's error mapping need revisiting.
- `charge_types` supersedes `conservation_mode`; targeting kernels < 6.19
  would need a `conservation_mode` fallback — deliberately deferred (owner
  runs 7.0).
- The daemon's D-Bus interface (`org.legiontoolkit.Daemon1`) is now the
  public API — version it: additive changes are fine; renames/removals need
  a `Daemon2` interface alongside `Daemon1`.
- GNOME tray support depends on the AppIndicator extension — a
  support-question magnet; the README note is the mitigation.
- Slint's built-in `SystemTrayIcon` (new in 1.17) may mature into a
  replacement for `ksni`; revisit once its Linux protocol is documented.
- Deferred explicitly: fan-curve editor (no kernel interface), any RGB
  support (hardware absent on 82WM; a 4-zone SKU (048d:c985) user should be
  pointed at 4JX/L5P-Keyboard-RGB rather than reimplementing), re-adding
  i18n (structure strings before accepting such a PR), automation
  actions/macros à la Windows LLT (big scope; needs its own plan).

---

## Appendix A — exact contents of the user-facing files (step 8)

Create these files with exactly this content. The legacy-cleanup lists were
compiled from the old installer's actual actions (`install.sh` at commit
`d2c0828`, lines 60–171): LLL git clone + DKMS, `legion_cli` shim,
`modprobe.d` force config, 0666 udev rules, sysfs `chmod a+w` (non-persistent
— nothing to clean), old polkit rules, Python tree in
`/usr/lib/legion-toolkit`.

### `install.sh`

```bash
#!/usr/bin/env bash
# Legion Linux Toolkit — Installer (Rust)
# First-class: Ubuntu 26.04 + GNOME. Also supports Arch Linux.
set -euo pipefail

GREEN='\033[0;32m'; CYAN='\033[0;36m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
ok()   { echo -e "  ${GREEN}✓${NC}  $*"; }
info() { echo -e "  ${CYAN}→${NC}  $*"; }
warn() { echo -e "  ${YELLOW}⚠${NC}  $*"; }
err()  { echo -e "  ${RED}✗${NC}  $*" >&2; exit 1; }

[[ $EUID -ne 0 ]] && err "Run as root: sudo bash install.sh"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REAL_USER="${SUDO_USER:-}"

# ── 1. Build dependencies ────────────────────────────────────────────────────
info "Checking build dependencies…"
if command -v apt-get &>/dev/null; then
    PKGS=()
    command -v cargo      &>/dev/null || PKGS+=(cargo)
    command -v cc         &>/dev/null || PKGS+=(build-essential)
    command -v pkg-config &>/dev/null || PKGS+=(pkg-config)
    if [[ ${#PKGS[@]} -gt 0 ]]; then
        apt-get update -qq
        apt-get install -y --no-install-recommends "${PKGS[@]}"
    fi
elif command -v pacman &>/dev/null; then
    pacman -S --noconfirm --needed rust base-devel
fi
command -v cargo &>/dev/null || err "cargo not found — install Rust via your package manager or https://rustup.rs"
ok "Build dependencies ready"

# ── 2. Sanity-check the kernel interface ─────────────────────────────────────
if [[ ! -e /sys/class/platform-profile/platform-profile-0/profile ]]; then
    warn "No platform-profile device found. This toolkit needs the upstream"
    warn "lenovo-wmi-gamezone driver (kernel 6.17+; 6.19+ recommended)."
    warn "Installing anyway — features will be hidden until the driver is present."
fi

# ── 3. Build (as the invoking user, so ~/.cargo stays user-owned) ────────────
info "Building release binaries…"
if [[ -n "$REAL_USER" ]]; then
    sudo -u "$REAL_USER" cargo build --release --workspace
else
    cargo build --release --workspace
fi
ok "Build complete"

# ── 4. Remove artifacts of the old Python/LLL toolkit (pre-1.0) ──────────────
legacy_cleanup() {
    pkill -f legion-tray.py 2>/dev/null || true
    pkill -f legion-gui.py  2>/dev/null || true
    # Old Python app tree + CLI shims
    rm -rf /usr/lib/legion-toolkit
    rm -f  /usr/local/bin/legion-ctl /usr/local/bin/legion_cli /usr/bin/legion_cli
    # LLL force-load config and DKMS module (never worked on the 82WM)
    rm -f  /etc/modprobe.d/legion_laptop_force.conf
    if command -v dkms &>/dev/null && dkms status 2>/dev/null | grep -q "legion-laptop"; then
        dkms remove -m legion-laptop -v 1.0 --all 2>/dev/null || true
        rm -rf /usr/src/legion-laptop-1.0
        info "Removed legion-laptop DKMS module (superseded by upstream drivers)"
    fi
    # World-writable udev rules and old polkit wiring
    rm -f /etc/udev/rules.d/99-legion-toolkit.rules
    rm -f /etc/polkit-1/rules.d/49-legion-toolkit.rules
    rm -f /usr/share/polkit-1/rules.d/49-legion-toolkit.rules
    udevadm control --reload-rules 2>/dev/null || true
    # (The old installer also chmod'ed sysfs files world-writable; sysfs
    # permissions reset on every boot, so there is nothing to undo there.)
}
info "Cleaning up any previous Python/LLL installation…"
legacy_cleanup
ok "Legacy cleanup done"

# ── 5. Install ───────────────────────────────────────────────────────────────
info "Installing…"
make -C "$SCRIPT_DIR" install
ok "Files installed"

# ── 6. Enable the daemon ─────────────────────────────────────────────────────
systemctl daemon-reload
systemctl enable --now legiond.service
ok "legiond running ($(systemctl is-active legiond.service))"

# ── 7. Start the tray for the invoking user ──────────────────────────────────
if [[ -n "$REAL_USER" ]]; then
    sudo -u "$REAL_USER" nohup /usr/bin/legion-gui --tray >/dev/null 2>&1 &
    ok "Tray started (autostarts on next login)"
fi

echo -e "\n${GREEN}✓ Installation complete.${NC}"
echo -e "  Daemon : systemctl status legiond"
echo -e "  CLI    : legion-ctl status"
echo -e "  GUI    : legion-gui  (tray: legion-gui --tray)"
```

### `uninstall.sh`

```bash
#!/usr/bin/env bash
# Legion Linux Toolkit — Uninstaller (Rust)
set -euo pipefail

GREEN='\033[0;32m'; CYAN='\033[0;36m'; RED='\033[0;31m'; NC='\033[0m'
ok()   { echo -e "  ${GREEN}✓${NC}  $*"; }
info() { echo -e "  ${CYAN}→${NC}  $*"; }
err()  { echo -e "  ${RED}✗${NC}  $*" >&2; exit 1; }

[[ $EUID -ne 0 ]] && err "Run as root: sudo bash uninstall.sh"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

info "Stopping processes and daemon…"
pkill -x legion-gui 2>/dev/null || true
systemctl disable --now legiond.service 2>/dev/null || true

info "Removing installed files…"
make -C "$SCRIPT_DIR" uninstall
rm -rf /var/lib/legiond
systemctl daemon-reload

# Also remove anything a pre-1.0 Python/LLL install left behind
info "Removing legacy Python/LLL artifacts (if any)…"
pkill -f legion-tray.py 2>/dev/null || true
pkill -f legion-gui.py  2>/dev/null || true
rm -rf /usr/lib/legion-toolkit
rm -f  /usr/local/bin/legion-ctl /usr/local/bin/legion_cli /usr/bin/legion_cli
rm -f  /etc/modprobe.d/legion_laptop_force.conf
rm -f  /etc/udev/rules.d/99-legion-toolkit.rules
rm -f  /etc/polkit-1/rules.d/49-legion-toolkit.rules
rm -f  /usr/share/polkit-1/rules.d/49-legion-toolkit.rules
if command -v dkms &>/dev/null && dkms status 2>/dev/null | grep -q "legion-laptop"; then
    dkms remove -m legion-laptop -v 1.0 --all 2>/dev/null || true
    rm -rf /usr/src/legion-laptop-1.0
fi
udevadm control --reload-rules 2>/dev/null || true

ok "Legion Linux Toolkit uninstalled."
```

### `Makefile`

```make
PREFIX        ?= /usr
BIN            = $(DESTDIR)$(PREFIX)/bin
UNITDIR        = $(DESTDIR)$(PREFIX)/lib/systemd/system
DBUSDIR        = $(DESTDIR)$(PREFIX)/share/dbus-1/system.d
POLKIT_ACTIONS = $(DESTDIR)$(PREFIX)/share/polkit-1/actions
APPS           = $(DESTDIR)$(PREFIX)/share/applications
AUTOSTART      = $(DESTDIR)/etc/xdg/autostart
ICONS          = $(DESTDIR)$(PREFIX)/share/icons/hicolor/512x512/apps

.PHONY: build deb install uninstall

build:
	cargo build --release --workspace

deb:
	cargo deb -p legion-daemon

install:
	install -Dm755 target/release/legiond    $(BIN)/legiond
	install -Dm755 target/release/legion-ctl $(BIN)/legion-ctl
	install -Dm755 target/release/legion-gui $(BIN)/legion-gui
	install -Dm644 systemd/legiond.service   $(UNITDIR)/legiond.service
	install -Dm644 dbus/org.legiontoolkit.Daemon.conf $(DBUSDIR)/org.legiontoolkit.Daemon.conf
	install -Dm644 polkit/org.legion-toolkit.policy $(POLKIT_ACTIONS)/org.legiontoolkit.policy
	install -Dm644 desktop/legion-toolkit.desktop $(APPS)/legion-toolkit.desktop
	install -Dm644 desktop/legion-toolkit-tray.desktop $(AUTOSTART)/legion-toolkit-tray.desktop
	install -Dm644 logo.png $(ICONS)/legion-toolkit.png

uninstall:
	rm -f $(BIN)/legiond $(BIN)/legion-ctl $(BIN)/legion-gui
	rm -f $(UNITDIR)/legiond.service
	rm -f $(DBUSDIR)/org.legiontoolkit.Daemon.conf
	rm -f $(POLKIT_ACTIONS)/org.legiontoolkit.policy
	rm -f $(APPS)/legion-toolkit.desktop
	rm -f $(AUTOSTART)/legion-toolkit-tray.desktop
	rm -f $(ICONS)/legion-toolkit.png
```

### `systemd/legiond.service`

```ini
[Unit]
Description=Legion Toolkit hardware control daemon

[Service]
Type=dbus
BusName=org.legiontoolkit.Daemon
ExecStart=/usr/bin/legiond
Restart=on-failure
StateDirectory=legiond
ProtectHome=yes
PrivateTmp=yes

[Install]
WantedBy=multi-user.target
```

### `dbus/org.legiontoolkit.Daemon.conf`

```xml
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <policy user="root">
    <allow own="org.legiontoolkit.Daemon"/>
  </policy>
  <policy context="default">
    <allow send_destination="org.legiontoolkit.Daemon"/>
  </policy>
</busconfig>
```

### `polkit/org.legion-toolkit.policy`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE policyconfig PUBLIC
 "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
<policyconfig>
  <vendor>Legion Linux Toolkit</vendor>
  <vendor_url>https://github.com/VVAT3R/legion-linux-toolkit</vendor_url>
  <action id="org.legiontoolkit.control">
    <description>Control Legion laptop hardware settings</description>
    <message>Authentication required to change Legion hardware settings</message>
    <defaults>
      <allow_any>no</allow_any>
      <allow_inactive>auth_admin</allow_inactive>
      <allow_active>yes</allow_active>
    </defaults>
  </action>
</policyconfig>
```

(`polkit/49-legion-toolkit.rules` is deleted, not rewritten —
`allow_active=yes` already gives the desktop user promptless control; see
step 7.)

### `desktop/legion-toolkit.desktop` (launcher)

```ini
[Desktop Entry]
Type=Application
Name=Legion Toolkit
Comment=Control panel for Lenovo Legion laptops
Exec=/usr/bin/legion-gui
Icon=legion-toolkit
Terminal=false
Categories=System;Settings;
```

### `desktop/legion-toolkit-tray.desktop` (autostart)

```ini
[Desktop Entry]
Type=Application
Name=Legion Toolkit Tray
Comment=Legion Toolkit system tray
Exec=/usr/bin/legion-gui --tray
Icon=legion-toolkit
Terminal=false
Categories=System;
X-GNOME-Autostart-enabled=true
```
