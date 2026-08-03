//! One-shot capability probe.
//!
//! [`Capabilities`] is built by probing every path the crate knows about. The
//! daemon builds it at startup and ships it to clients as JSON over D-Bus, so
//! the GUI and CLI can hide what a given machine does not have without ever
//! touching sysfs themselves.
//!
//! Probing must never panic and never refuse: an unrecognised machine simply
//! reports fewer capabilities.

use serde::{Deserialize, Serialize};

use crate::{Result, SysRoot};
use crate::{battery, cpu, fan, firmware_attrs, ideapad, kbd_backlight, profile};

const DMI: &str = "sys/class/dmi/id";

/// What this machine can actually do.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    // Identity
    pub product_family: Option<String>,
    pub product_name: Option<String>,
    /// Machine type, e.g. `82WM`.
    pub board_name: Option<String>,
    /// Does `product_family` look like a Legion machine?
    pub is_legion: bool,

    // Power profile
    pub platform_profile: bool,
    /// Driver behind the platform-profile device, e.g. `lenovo-wmi-gamezone`.
    pub platform_profile_driver: Option<String>,
    /// Profiles the firmware advertises, as kernel strings.
    pub profile_choices: Vec<String>,

    // Firmware attributes (power limits)
    pub firmware_attrs: bool,
    /// Every advertised attribute directory name — not a hardcoded list, so
    /// attributes a newer kernel adds show up here automatically.
    pub firmware_attr_names: Vec<String>,
    pub ppt: bool,

    // Battery
    pub charge_types: bool,
    pub charge_type_choices: Vec<String>,
    pub battery: bool,
    pub ac_adapter: bool,

    // ideapad_laptop toggles
    pub fn_lock: bool,
    pub camera_power: bool,
    pub usb_charging: bool,
    pub conservation_mode: bool,
    pub fan_mode: bool,

    // Misc
    pub kbd_backlight: bool,
    pub kbd_backlight_max: Option<u32>,
    pub cpu_boost: bool,
    pub cpu_temp: bool,
    /// False on kernel 7.0 for the 82WM; true once a fan hwmon appears.
    pub fan_rpm: bool,
}

impl Capabilities {
    /// Serialise for the daemon's `Capabilities` D-Bus property.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|e| crate::HwError::Parse(format!("serialising capabilities: {e}")))
    }

    /// Parse what the daemon sent.
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s)
            .map_err(|e| crate::HwError::Parse(format!("parsing capabilities: {e}")))
    }

    /// Nothing controllable was found — the GUI shows the "unsupported
    /// machine" notice rather than an empty dashboard.
    pub fn is_empty(&self) -> bool {
        !(self.platform_profile
            || self.firmware_attrs
            || self.charge_types
            || self.fn_lock
            || self.camera_power
            || self.usb_charging
            || self.fan_mode
            || self.kbd_backlight
            || self.cpu_boost)
    }
}

/// Does DMI identify this as a Legion machine?
///
/// A mismatch is informational only — callers warn but continue, since any
/// machine exposing the same upstream ABI works fine.
pub fn is_legion(root: &SysRoot) -> bool {
    let hit = |v: Option<String>| {
        v.is_some_and(|s| {
            let s = s.to_ascii_lowercase();
            s.contains("legion") || s.contains("loq")
        })
    };
    hit(root.read_opt(&format!("{DMI}/product_family")))
        || hit(root.read_opt(&format!("{DMI}/product_name")))
}

/// Probe every interface once.
pub fn probe(root: &SysRoot) -> Capabilities {
    let profile_choices = profile::choices(root)
        .map(|v| v.into_iter().map(|p| p.as_sysfs().to_string()).collect())
        .unwrap_or_default();

    let charge = battery::charge_types(root).ok();

    let firmware_attr_names = firmware_attrs::available_attrs(root);
    let ppt = firmware_attrs::PptAttr::ALL
        .iter()
        .any(|a| firmware_attr_names.iter().any(|n| n == a.dir_name()));

    Capabilities {
        product_family: root.read_opt(&format!("{DMI}/product_family")),
        product_name: root.read_opt(&format!("{DMI}/product_name")),
        board_name: root.read_opt(&format!("{DMI}/board_name")),
        is_legion: is_legion(root),

        platform_profile: profile::available(root),
        platform_profile_driver: profile::driver_name(root),
        profile_choices,

        firmware_attrs: firmware_attrs::available(root),
        firmware_attr_names,
        ppt,

        charge_types: battery::available(root),
        charge_type_choices: charge
            .map(|c| {
                c.available
                    .into_iter()
                    .map(|m| m.as_sysfs().to_string())
                    .collect()
            })
            .unwrap_or_default(),
        battery: root.exists("sys/class/power_supply/BAT0/capacity"),
        ac_adapter: battery::ac_online(root).is_some(),

        fn_lock: ideapad::has(root, ideapad::Toggle::FnLock),
        camera_power: ideapad::has(root, ideapad::Toggle::CameraPower),
        usb_charging: ideapad::has(root, ideapad::Toggle::UsbCharging),
        conservation_mode: ideapad::has(root, ideapad::Toggle::ConservationMode),
        fan_mode: ideapad::has_fan_mode(root),

        kbd_backlight: kbd_backlight::available(root),
        kbd_backlight_max: kbd_backlight::max(root).ok(),
        cpu_boost: cpu::has_boost(root),
        cpu_temp: cpu::temp_celsius(root).is_some(),
        fan_rpm: fan::available(root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(root: &SysRoot, rel: &str, val: &str) {
        let p = root.path(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("{val}\n")).unwrap();
    }

    /// The live 82WM tree, values exactly as recorded on the machine.
    fn fake_82wm(root: &SysRoot) {
        write(
            root,
            "sys/class/dmi/id/product_family",
            "Legion R9000P ARX8",
        );
        write(root, "sys/class/dmi/id/product_name", "82WM");
        write(root, "sys/class/dmi/id/board_name", "LNVNB161216");

        let pp = "sys/class/platform-profile/platform-profile-0";
        write(root, &format!("{pp}/name"), "lenovo-wmi-gamezone");
        write(
            root,
            &format!("{pp}/choices"),
            "low-power balanced performance max-power custom",
        );
        write(root, &format!("{pp}/profile"), "balanced");

        let fa = "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes";
        for (dir, cur, def, min, max) in [
            ("ppt_pl1_spl", 70, 70, 50, 115),
            ("ppt_pl2_sppt", 85, 85, 60, 130),
            ("ppt_pl3_fppt", 102, 102, 70, 150),
        ] {
            write(root, &format!("{fa}/{dir}/type"), "integer");
            write(root, &format!("{fa}/{dir}/current_value"), &cur.to_string());
            write(root, &format!("{fa}/{dir}/default_value"), &def.to_string());
            write(root, &format!("{fa}/{dir}/min_value"), &min.to_string());
            write(root, &format!("{fa}/{dir}/max_value"), &max.to_string());
            write(root, &format!("{fa}/{dir}/scalar_increment"), "1");
            write(root, &format!("{fa}/{dir}/display_name"), dir);
        }

        let bat = "sys/class/power_supply/BAT0";
        write(
            root,
            &format!("{bat}/charge_types"),
            "Fast Standard [Long_Life]",
        );
        write(root, &format!("{bat}/capacity"), "78");
        write(root, &format!("{bat}/energy_now"), "62000000");
        write(root, &format!("{bat}/energy_full"), "80000000");
        write(root, &format!("{bat}/energy_full_design"), "80000000");
        write(root, &format!("{bat}/voltage_now"), "15400000");
        write(root, &format!("{bat}/power_now"), "12000000");
        write(root, &format!("{bat}/cycle_count"), "42");
        write(root, &format!("{bat}/status"), "Discharging");
        // Deliberately no `temp` — this battery has none.
        write(root, "sys/class/power_supply/ADP0/type", "Mains");
        write(root, "sys/class/power_supply/ADP0/online", "1");

        let vpc = "sys/bus/platform/devices/VPC2004:00";
        write(root, &format!("{vpc}/fn_lock"), "0");
        write(root, &format!("{vpc}/camera_power"), "1");
        write(root, &format!("{vpc}/usb_charging"), "0");
        write(root, &format!("{vpc}/conservation_mode"), "1");
        write(root, &format!("{vpc}/fan_mode"), "0");
        // Deliberately no `touchpad` — absent on this machine.

        let led = "sys/class/leds/platform::kbd_backlight";
        write(root, &format!("{led}/brightness"), "1");
        write(root, &format!("{led}/max_brightness"), "2");

        write(root, "sys/devices/system/cpu/cpufreq/boost", "1");
        write(
            root,
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
            "powersave",
        );
        write(
            root,
            "sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference",
            "balance_performance",
        );
        write(
            root,
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq",
            "3593000",
        );

        write(root, "sys/class/hwmon/hwmon0/name", "nvme");
        write(root, "sys/class/hwmon/hwmon0/temp1_input", "41850");
        write(root, "sys/class/hwmon/hwmon1/name", "k10temp");
        write(root, "sys/class/hwmon/hwmon1/temp1_input", "52125");
        // Deliberately no fanN_input anywhere.
    }

    #[test]
    fn full_82wm_reports_everything_except_fan_rpm() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        fake_82wm(&root);
        let c = probe(&root);

        assert!(c.is_legion);
        assert_eq!(c.product_name.as_deref(), Some("82WM"));
        assert!(c.platform_profile);
        assert_eq!(
            c.platform_profile_driver.as_deref(),
            Some("lenovo-wmi-gamezone")
        );
        assert_eq!(c.profile_choices.len(), 5);
        assert!(c.firmware_attrs);
        assert!(c.ppt);
        assert_eq!(c.firmware_attr_names.len(), 3);
        assert!(c.charge_types);
        assert_eq!(c.charge_type_choices.len(), 3);
        assert!(c.battery && c.ac_adapter);
        assert!(c.fn_lock && c.camera_power && c.usb_charging && c.fan_mode);
        assert!(c.kbd_backlight);
        assert_eq!(c.kbd_backlight_max, Some(2));
        assert!(c.cpu_boost && c.cpu_temp);

        // The one thing this kernel does not give us.
        assert!(!c.fan_rpm);
        assert!(!c.is_empty());
    }

    #[test]
    fn empty_tree_probes_cleanly() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        let c = probe(&root);

        assert!(!c.is_legion);
        assert!(!c.platform_profile);
        assert!(!c.firmware_attrs);
        assert!(!c.charge_types);
        assert!(!c.kbd_backlight);
        assert!(c.profile_choices.is_empty());
        assert!(c.firmware_attr_names.is_empty());
        assert!(c.is_empty());
    }

    #[test]
    fn capabilities_round_trip_through_json() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        fake_82wm(&root);
        let c = probe(&root);
        let json = c.to_json().unwrap();
        assert_eq!(Capabilities::from_json(&json).unwrap(), c);
    }

    #[test]
    fn unknown_vendor_is_not_refused() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        write(&root, "sys/class/dmi/id/product_family", "Precision 7550");
        write(
            &root,
            "sys/class/platform-profile/platform-profile-0/profile",
            "balanced",
        );
        let c = probe(&root);
        // Not a Legion, but the profile interface is still reported.
        assert!(!c.is_legion);
        assert!(c.platform_profile);
    }

    #[test]
    fn new_firmware_attrs_appear_without_code_changes() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        fake_82wm(&root);
        // Pretend a newer kernel started advertising a GPU attribute.
        let fa = "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes";
        write(&root, &format!("{fa}/dgpu_boost_clk/current_value"), "150");
        write(&root, &format!("{fa}/dgpu_boost_clk/min_value"), "0");
        write(&root, &format!("{fa}/dgpu_boost_clk/max_value"), "200");

        let c = probe(&root);
        assert!(c.firmware_attr_names.iter().any(|n| n == "dgpu_boost_clk"));
        assert_eq!(firmware_attrs::list_all(&root).len(), 4);
    }
}
