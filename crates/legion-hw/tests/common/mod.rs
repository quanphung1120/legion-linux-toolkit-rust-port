//! A fake sysfs tree mirroring the live Lenovo Legion R9000P ARX8 (82WM)
//! running kernel 7.0.0-28-generic.
//!
//! Every value here was recorded from the real machine, so a test asserting
//! `spl == 70 [50–115]` is asserting against hardware truth rather than an
//! invented fixture. Attributes the machine genuinely lacks — battery `temp`,
//! `touchpad`, any `fanN_input` — are deliberately absent.

#![allow(dead_code)]

use std::fs;
use std::path::Path;

use legion_hw::SysRoot;
use tempfile::TempDir;

/// Write a sysfs-style file (creating parents), appending the trailing
/// newline the kernel always emits.
pub fn write(root: &SysRoot, rel: &str, val: &str) {
    let p = root.path(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, format!("{val}\n")).unwrap();
}

/// Read a file back without the trailing newline — for asserting on writes.
pub fn read(root: &SysRoot, rel: &str) -> String {
    fs::read_to_string(root.path(rel))
        .unwrap_or_else(|e| panic!("reading {rel}: {e}"))
        .trim()
        .to_string()
}

/// Make a file read-only, to exercise the permission-denied path.
pub fn make_read_only(root: &SysRoot, rel: &str) {
    let p = root.path(rel);
    let mut perms = fs::metadata(&p).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o444);
    }
    fs::set_permissions(&p, perms).unwrap();
}

/// Build the full 82WM tree under `dir` and return a [`SysRoot`] for it.
pub fn fake_82wm(dir: &Path) -> SysRoot {
    let root = SysRoot::at(dir);

    // ── DMI identity ────────────────────────────────────────────────────────
    write(
        &root,
        "sys/class/dmi/id/product_family",
        "Legion R9000P ARX8",
    );
    write(&root, "sys/class/dmi/id/product_name", "82WM");
    write(&root, "sys/class/dmi/id/board_name", "LNVNB161216");

    // ── platform-profile (lenovo-wmi-gamezone) ──────────────────────────────
    let pp = "sys/class/platform-profile/platform-profile-0";
    write(&root, &format!("{pp}/name"), "lenovo-wmi-gamezone");
    write(
        &root,
        &format!("{pp}/choices"),
        "low-power balanced performance max-power custom",
    );
    write(&root, &format!("{pp}/profile"), "balanced");

    // ── firmware-attributes (lenovo-wmi-other): the three PPT limits ────────
    let fa = "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes";
    for (dir_name, cur, def, min, max) in [
        ("ppt_pl1_spl", 70, 70, 50, 115),
        ("ppt_pl2_sppt", 85, 85, 60, 130),
        ("ppt_pl3_fppt", 102, 102, 70, 150),
    ] {
        write(&root, &format!("{fa}/{dir_name}/type"), "integer");
        write(
            &root,
            &format!("{fa}/{dir_name}/current_value"),
            &cur.to_string(),
        );
        write(
            &root,
            &format!("{fa}/{dir_name}/default_value"),
            &def.to_string(),
        );
        write(
            &root,
            &format!("{fa}/{dir_name}/min_value"),
            &min.to_string(),
        );
        write(
            &root,
            &format!("{fa}/{dir_name}/max_value"),
            &max.to_string(),
        );
        write(&root, &format!("{fa}/{dir_name}/scalar_increment"), "1");
        write(&root, &format!("{fa}/{dir_name}/display_name"), dir_name);
    }

    // ── battery + AC ────────────────────────────────────────────────────────
    let bat = "sys/class/power_supply/BAT0";
    write(&root, &format!("{bat}/type"), "Battery");
    write(
        &root,
        &format!("{bat}/charge_types"),
        "Fast Standard [Long_Life]",
    );
    write(&root, &format!("{bat}/capacity"), "78");
    write(&root, &format!("{bat}/energy_now"), "62000000");
    write(&root, &format!("{bat}/energy_full"), "80000000");
    write(&root, &format!("{bat}/energy_full_design"), "99900000");
    write(&root, &format!("{bat}/voltage_now"), "15400000");
    write(&root, &format!("{bat}/power_now"), "12000000");
    write(&root, &format!("{bat}/cycle_count"), "42");
    write(&root, &format!("{bat}/status"), "Discharging");
    // NOTE: no `temp` — this battery does not expose one.

    write(&root, "sys/class/power_supply/ADP0/type", "Mains");
    write(&root, "sys/class/power_supply/ADP0/online", "1");

    // ── ideapad_laptop toggles ──────────────────────────────────────────────
    let vpc = "sys/bus/platform/devices/VPC2004:00";
    write(&root, &format!("{vpc}/fn_lock"), "0");
    write(&root, &format!("{vpc}/camera_power"), "1");
    write(&root, &format!("{vpc}/usb_charging"), "0");
    write(&root, &format!("{vpc}/conservation_mode"), "1");
    write(&root, &format!("{vpc}/fan_mode"), "0");
    // NOTE: no `touchpad` — absent on this machine.

    // ── keyboard backlight (white-only, 3 levels) ───────────────────────────
    let led = "sys/class/leds/platform::kbd_backlight";
    write(&root, &format!("{led}/brightness"), "1");
    write(&root, &format!("{led}/max_brightness"), "2");
    write(&root, &format!("{led}/brightness_hw_changed"), "0");

    // ── CPU ─────────────────────────────────────────────────────────────────
    write(&root, "sys/devices/system/cpu/cpufreq/boost", "1");
    let c0 = "sys/devices/system/cpu/cpu0/cpufreq";
    write(&root, &format!("{c0}/scaling_governor"), "powersave");
    write(
        &root,
        &format!("{c0}/energy_performance_preference"),
        "balance_performance",
    );
    write(&root, &format!("{c0}/scaling_cur_freq"), "3593000");

    // ── hwmon: an nvme sensor plus k10temp, no fan sensors ──────────────────
    write(&root, "sys/class/hwmon/hwmon0/name", "nvme");
    write(&root, "sys/class/hwmon/hwmon0/temp1_input", "41850");
    write(&root, "sys/class/hwmon/hwmon1/name", "k10temp");
    write(&root, "sys/class/hwmon/hwmon1/temp1_input", "52125");
    write(&root, "sys/class/hwmon/hwmon1/temp1_label", "Tctl");
    // NOTE: no fanN_input anywhere — kernel 7.0 exposes no fan hwmon here.

    root
}

/// Convenience: a `TempDir` plus a populated 82WM `SysRoot`.
pub fn fixture() -> (TempDir, SysRoot) {
    let tmp = TempDir::new().unwrap();
    let root = fake_82wm(tmp.path());
    (tmp, root)
}
