//! Subcommand implementations.
//!
//! Each one reads through the daemon when it is available and through
//! `legion-hw` otherwise, so behaviour is identical either way apart from who
//! is allowed to write.

use legion_hw::battery::ChargeMode;
use legion_hw::firmware_attrs::{self, AttrInfo, PptAttr};
use legion_hw::ideapad::{self, Toggle};
use legion_hw::profile::{self, PowerProfile};
use legion_hw::{battery, cpu, detect, fan, kbd_backlight};

use crate::backend::{Backend, describe_error, describe_hw_error};

/// Every command returns a message on failure; `main` prints it and exits 1.
pub type CmdResult = Result<(), String>;

/// Parse an on/off argument.
fn parse_on_off(s: &str) -> Result<bool, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" | "enable" | "enabled" => Ok(true),
        "off" | "false" | "0" | "no" | "disable" | "disabled" => Ok(false),
        other => Err(format!("expected 'on' or 'off', got {other:?}")),
    }
}

fn show_on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

// ── profile ─────────────────────────────────────────────────────────────────

pub async fn profile(backend: &Backend, value: Option<&str>) -> CmdResult {
    match value {
        None => {
            let current = match backend.proxy() {
                Some(p) => p
                    .get_property::<String>("Profile")
                    .await
                    .map_err(|e| describe_error(&e))?,
                None => profile::get(backend.root())
                    .map_err(|e| describe_hw_error(&e))?
                    .as_sysfs()
                    .to_string(),
            };
            let label = PowerProfile::from_sysfs(&current)
                .map(|p| p.label())
                .unwrap_or("unknown");
            println!("profile: {current} ({label})");
            Ok(())
        }
        Some(v) => {
            // Validate before dispatching so a typo produces the same message
            // in both backends.
            let p = PowerProfile::from_user(v).map_err(|e| describe_hw_error(&e))?;
            match backend.proxy() {
                Some(proxy) => proxy
                    .call_method("SetProfile", &(p.as_sysfs()))
                    .await
                    .map(|_| ())
                    .map_err(|e| describe_error(&e))?,
                None => profile::set(backend.root(), p).map_err(|e| describe_hw_error(&e))?,
            }
            println!("profile: {} ({})", p.as_sysfs(), p.label());
            Ok(())
        }
    }
}

/// Follow `PropertiesChanged` and print every profile change, including the
/// ones the firmware makes when the user presses Fn+Q.
pub async fn profile_watch(backend: &Backend) -> CmdResult {
    use futures_util::StreamExt;

    let proxy = backend.proxy().ok_or_else(|| {
        "--watch needs the daemon: start it with 'systemctl enable --now legiond'".to_string()
    })?;

    let current: String = proxy
        .get_property("Profile")
        .await
        .map_err(|e| describe_error(&e))?;
    println!("profile: {current}");

    let mut changes = proxy.receive_property_changed::<String>("Profile").await;
    while let Some(change) = changes.next().await {
        match change.get().await {
            Ok(v) => println!("profile: {v}"),
            Err(e) => return Err(describe_error(&e)),
        }
    }
    Ok(())
}

// ── ppt ─────────────────────────────────────────────────────────────────────

fn print_attr(a: &AttrInfo) {
    println!(
        "{}: {} W (default {}, range {}–{})",
        a.key, a.current, a.default, a.min, a.max
    );
}

async fn read_ppt(backend: &Backend) -> Result<Vec<AttrInfo>, String> {
    match backend.proxy() {
        Some(proxy) => {
            let json: String = proxy
                .call_method("GetPpt", &())
                .await
                .map_err(|e| describe_error(&e))?
                .body()
                .deserialize()
                .map_err(|e| e.to_string())?;
            serde_json::from_str(&json).map_err(|e| e.to_string())
        }
        None => Ok(firmware_attrs::list_all(backend.root())),
    }
}

pub async fn ppt(backend: &Backend, attr: Option<&str>, watts: Option<u32>) -> CmdResult {
    let all = read_ppt(backend).await?;
    if all.is_empty() {
        return Err("no power-limit attributes on this machine".into());
    }

    let Some(attr) = attr else {
        for a in &all {
            print_attr(a);
        }
        return Ok(());
    };

    // Accept both the short key and the raw firmware directory name.
    let key = PptAttr::from_key(attr)
        .map(|a| a.key().to_string())
        .unwrap_or_else(|_| attr.to_string());
    let info = all
        .iter()
        .find(|a| a.key == key)
        .ok_or_else(|| {
            let known: Vec<&str> = all.iter().map(|a| a.key.as_str()).collect();
            format!("unknown power limit {attr:?} (known: {})", known.join(", "))
        })?
        .clone();

    let Some(watts) = watts else {
        print_attr(&info);
        return Ok(());
    };

    // Range-check here too, so the error names the range even in daemon mode.
    if !info.in_range(watts) {
        return Err(format!(
            "{watts} W is out of range for {}: {}–{} W",
            info.key, info.min, info.max
        ));
    }

    match backend.proxy() {
        Some(proxy) => proxy
            .call_method("SetPpt", &(info.key.as_str(), watts))
            .await
            .map(|_| ())
            .map_err(|e| describe_error(&e))?,
        None => {
            let dir = PptAttr::from_key(&info.key)
                .map(|a| a.dir_name().to_string())
                .unwrap_or_else(|_| info.key.clone());
            firmware_attrs::write_by_name(backend.root(), &dir, watts)
                .map_err(|e| describe_hw_error(&e))?
        }
    }
    println!("{}: {watts} W", info.key);
    Ok(())
}

// ── battery ─────────────────────────────────────────────────────────────────

pub async fn battery(backend: &Backend, value: Option<&str>) -> CmdResult {
    match value {
        None => {
            let current = match backend.proxy() {
                Some(p) => p
                    .get_property::<String>("BatteryMode")
                    .await
                    .map_err(|e| describe_error(&e))?,
                None => battery::get_mode(backend.root())
                    .map_err(|e| describe_hw_error(&e))?
                    .as_sysfs()
                    .to_string(),
            };
            let label = ChargeMode::from_sysfs(&current)
                .map(|m| m.label())
                .unwrap_or("unknown");
            println!("battery: {current} ({label})");
            Ok(())
        }
        Some(v) => {
            let m = ChargeMode::from_user(v).map_err(|e| describe_hw_error(&e))?;
            match backend.proxy() {
                Some(proxy) => proxy
                    .call_method("SetBatteryMode", &(m.as_sysfs()))
                    .await
                    .map(|_| ())
                    .map_err(|e| describe_error(&e))?,
                None => battery::set_mode(backend.root(), m).map_err(|e| describe_hw_error(&e))?,
            }
            println!("battery: {} ({})", m.as_sysfs(), m.label());
            Ok(())
        }
    }
}

// ── boolean toggles ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum ToggleKind {
    FnLock,
    Camera,
    UsbCharging,
    Boost,
}

impl ToggleKind {
    fn cli_name(self) -> &'static str {
        match self {
            ToggleKind::FnLock => "fnlock",
            ToggleKind::Camera => "camera",
            ToggleKind::UsbCharging => "usb-charging",
            ToggleKind::Boost => "boost",
        }
    }

    fn dbus_property(self) -> &'static str {
        match self {
            ToggleKind::FnLock => "FnLock",
            ToggleKind::Camera => "CameraPower",
            ToggleKind::UsbCharging => "UsbCharging",
            ToggleKind::Boost => "CpuBoost",
        }
    }

    fn dbus_method(self) -> &'static str {
        match self {
            ToggleKind::FnLock => "SetFnLock",
            ToggleKind::Camera => "SetCameraPower",
            ToggleKind::UsbCharging => "SetUsbCharging",
            ToggleKind::Boost => "SetCpuBoost",
        }
    }

    fn hw_toggle(self) -> Option<Toggle> {
        match self {
            ToggleKind::FnLock => Some(Toggle::FnLock),
            ToggleKind::Camera => Some(Toggle::CameraPower),
            ToggleKind::UsbCharging => Some(Toggle::UsbCharging),
            ToggleKind::Boost => None,
        }
    }
}

pub async fn toggle(backend: &Backend, kind: ToggleKind, value: Option<&str>) -> CmdResult {
    match value {
        None => {
            let current = match backend.proxy() {
                Some(p) => p
                    .get_property::<bool>(kind.dbus_property())
                    .await
                    .map_err(|e| describe_error(&e))?,
                None => match kind.hw_toggle() {
                    Some(t) => ideapad::get(backend.root(), t)
                        .ok_or_else(|| "not supported on this machine".to_string())?,
                    None => cpu::get_boost(backend.root())
                        .ok_or_else(|| "not supported on this machine".to_string())?,
                },
            };
            println!("{}: {}", kind.cli_name(), show_on_off(current));
            Ok(())
        }
        Some(v) => {
            let on = parse_on_off(v)?;
            match backend.proxy() {
                Some(proxy) => proxy
                    .call_method(kind.dbus_method(), &(on))
                    .await
                    .map(|_| ())
                    .map_err(|e| describe_error(&e))?,
                None => match kind.hw_toggle() {
                    Some(t) => {
                        ideapad::set(backend.root(), t, on).map_err(|e| describe_hw_error(&e))?
                    }
                    None => {
                        cpu::set_boost(backend.root(), on).map_err(|e| describe_hw_error(&e))?
                    }
                },
            }
            println!("{}: {}", kind.cli_name(), show_on_off(on));
            Ok(())
        }
    }
}

// ── backlight ───────────────────────────────────────────────────────────────

pub async fn backlight(backend: &Backend, value: Option<u32>) -> CmdResult {
    let max = kbd_backlight::max(backend.root()).unwrap_or(2);
    match value {
        None => {
            let current = match backend.proxy() {
                Some(p) => p
                    .get_property::<u32>("KbdBacklight")
                    .await
                    .map_err(|e| describe_error(&e))?,
                None => kbd_backlight::get(backend.root()).map_err(|e| describe_hw_error(&e))?,
            };
            println!("backlight: {current} (max {max})");
            Ok(())
        }
        Some(level) => {
            if level > max {
                return Err(format!("backlight level {level} is out of range: 0–{max}"));
            }
            match backend.proxy() {
                Some(proxy) => proxy
                    .call_method("SetKbdBacklight", &(level))
                    .await
                    .map(|_| ())
                    .map_err(|e| describe_error(&e))?,
                None => {
                    kbd_backlight::set(backend.root(), level).map_err(|e| describe_hw_error(&e))?
                }
            }
            println!("backlight: {level}");
            Ok(())
        }
    }
}

// ── status ──────────────────────────────────────────────────────────────────

/// Plain `key: value` lines, capability-aware — absent hardware is omitted
/// rather than printed as "unknown", so the output is parseable.
pub async fn status(backend: &Backend) -> CmdResult {
    let root = backend.root();
    let caps = detect::probe(root);

    if let Some(name) = &caps.product_name {
        println!("machine: {name}");
    }
    if let Some(family) = &caps.product_family {
        println!("family: {family}");
    }
    println!(
        "daemon: {}",
        if backend.proxy().is_some() {
            "running"
        } else {
            "not running"
        }
    );

    if caps.is_empty() {
        println!("supported: no");
        println!("hint: the lenovo-wmi-gamezone driver does not appear to be loaded");
        return Ok(());
    }

    if caps.platform_profile {
        let current = match backend.proxy() {
            Some(p) => p.get_property::<String>("Profile").await.ok(),
            None => profile::get(root).ok().map(|p| p.as_sysfs().to_string()),
        };
        if let Some(c) = current {
            let label = PowerProfile::from_sysfs(&c)
                .map(|p| p.label())
                .unwrap_or("unknown");
            println!("profile: {c} ({label})");
        }
        if let Some(driver) = &caps.platform_profile_driver {
            println!("profile-driver: {driver}");
        }
        if !caps.profile_choices.is_empty() {
            println!("profile-choices: {}", caps.profile_choices.join(" "));
        }
    }

    if caps.ppt {
        for a in read_ppt(backend).await.unwrap_or_default() {
            println!("ppt-{}: {} W (range {}–{})", a.key, a.current, a.min, a.max);
        }
    }

    if caps.charge_types
        && let Ok(m) = battery::get_mode(root)
    {
        println!("battery-mode: {} ({})", m.as_sysfs(), m.label());
    }

    let stats = battery::stats(root);
    if let Some(c) = stats.capacity {
        println!("battery-capacity: {c}%");
    }
    if let Some(s) = &stats.status {
        println!("battery-status: {s}");
    }
    if let Some(h) = stats.health_pct {
        println!("battery-health: {h:.1}%");
    }
    if let Some(c) = stats.cycle_count {
        println!("battery-cycles: {c}");
    }
    if let Some(ac) = stats.ac_online {
        println!("ac: {}", if ac { "online" } else { "offline" });
    }

    for (cap, kind) in [
        (caps.fn_lock, ToggleKind::FnLock),
        (caps.camera_power, ToggleKind::Camera),
        (caps.usb_charging, ToggleKind::UsbCharging),
    ] {
        if cap
            && let Some(t) = kind.hw_toggle()
            && let Some(v) = ideapad::get(root, t)
        {
            println!("{}: {}", kind.cli_name(), show_on_off(v));
        }
    }

    if caps.fan_mode
        && let Ok(m) = ideapad::get_fan_mode(root)
    {
        println!("fan-mode: {}", m.label());
    }

    match fan::rpm(root) {
        Some(rpm) => println!("fan-rpm: {rpm}"),
        // Explicit, because "no fan sensor" is the expected state on this
        // kernel and users otherwise assume something is broken.
        None => println!("fan-rpm: unavailable (no fan sensor on this kernel)"),
    }

    if caps.kbd_backlight
        && let Ok(level) = kbd_backlight::get(root)
    {
        println!(
            "backlight: {level} (max {})",
            caps.kbd_backlight_max.unwrap_or(2)
        );
    }

    let cpu_info = cpu::info(root);
    if let Some(b) = cpu_info.boost {
        println!("cpu-boost: {}", show_on_off(b));
    }
    if let Some(g) = &cpu_info.governor {
        println!("cpu-governor: {g}");
    }
    if let Some(e) = &cpu_info.epp {
        println!("cpu-epp: {e}");
    }
    if let Some(f) = cpu_info.cur_freq_khz {
        println!("cpu-freq: {:.2} GHz", f as f64 / 1_000_000.0);
    }
    if let Some(t) = cpu_info.temp_celsius {
        println!("cpu-temp: {t:.1} C");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_off_parsing_is_forgiving() {
        for s in ["on", "ON", "true", "1", "yes", "enable"] {
            assert!(parse_on_off(s).unwrap(), "{s}");
        }
        for s in ["off", "OFF", "false", "0", "no", "disable"] {
            assert!(!parse_on_off(s).unwrap(), "{s}");
        }
        assert!(parse_on_off("maybe").is_err());
    }

    #[test]
    fn toggle_names_map_to_the_dbus_surface() {
        assert_eq!(ToggleKind::FnLock.dbus_property(), "FnLock");
        assert_eq!(ToggleKind::FnLock.dbus_method(), "SetFnLock");
        assert_eq!(ToggleKind::UsbCharging.dbus_property(), "UsbCharging");
        assert_eq!(ToggleKind::Boost.dbus_method(), "SetCpuBoost");
        // Boost is not an ideapad attribute.
        assert!(ToggleKind::Boost.hw_toggle().is_none());
    }
}
