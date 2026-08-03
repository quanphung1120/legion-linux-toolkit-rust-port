//! CLI behaviour in direct-sysfs mode.
//!
//! These run without a daemon and without root: `--sysfs-root` points the
//! binary at a fake 82WM tree, and `--bus session` keeps it from finding a
//! real daemon on the system bus. That exercises the fallback path — the one
//! a user hits before installing the service.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use tempfile::TempDir;

fn write(root: &Path, rel: &str, val: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, format!("{val}\n")).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel))
        .unwrap()
        .trim()
        .to_string()
}

/// The live 82WM values.
fn fake_82wm(root: &Path) {
    write(
        root,
        "sys/class/dmi/id/product_family",
        "Legion R9000P ARX8",
    );
    write(root, "sys/class/dmi/id/product_name", "82WM");

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
    write(root, &format!("{bat}/energy_full_design"), "99900000");
    write(root, &format!("{bat}/cycle_count"), "42");
    write(root, &format!("{bat}/status"), "Discharging");
    write(root, "sys/class/power_supply/ADP0/type", "Mains");
    write(root, "sys/class/power_supply/ADP0/online", "1");

    let vpc = "sys/bus/platform/devices/VPC2004:00";
    write(root, &format!("{vpc}/fn_lock"), "0");
    write(root, &format!("{vpc}/camera_power"), "1");
    write(root, &format!("{vpc}/usb_charging"), "0");
    write(root, &format!("{vpc}/fan_mode"), "0");

    let led = "sys/class/leds/platform::kbd_backlight";
    write(root, &format!("{led}/brightness"), "1");
    write(root, &format!("{led}/max_brightness"), "2");

    write(root, "sys/devices/system/cpu/cpufreq/boost", "1");
    let c0 = "sys/devices/system/cpu/cpu0/cpufreq";
    write(root, &format!("{c0}/scaling_governor"), "powersave");
    write(root, &format!("{c0}/scaling_cur_freq"), "3593000");
    write(root, "sys/class/hwmon/hwmon1/name", "k10temp");
    write(root, "sys/class/hwmon/hwmon1/temp1_input", "52125");
}

fn fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    fake_82wm(tmp.path());
    tmp
}

/// A `legion-ctl` invocation wired to the fake tree, with no daemon reachable.
fn ctl(root: &Path) -> Command {
    let mut cmd = Command::cargo_bin("legion-ctl").unwrap();
    cmd.arg("--sysfs-root")
        .arg(root)
        .arg("--bus")
        .arg("session")
        // Point at a bus address nothing is listening on, so the daemon is
        // definitively absent and the direct-sysfs path is what gets tested.
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/legion-ctl-test",
        );
    cmd
}

#[test]
fn status_reports_the_machine_and_is_parseable() {
    let tmp = fixture();
    let out = ctl(tmp.path()).arg("status").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("profile: balanced"), "{stdout}");
    assert!(stdout.contains("machine: 82WM"), "{stdout}");
    assert!(stdout.contains("daemon: not running"), "{stdout}");
    assert!(stdout.contains("battery-mode: Long_Life"), "{stdout}");
    assert!(stdout.contains("ppt-spl: 70 W"), "{stdout}");
    assert!(stdout.contains("cpu-temp: 52.1 C"), "{stdout}");

    // Every line is `key: value`, so scripts can parse it.
    for line in stdout.lines().filter(|l| !l.is_empty()) {
        assert!(line.contains(": "), "unparseable status line: {line:?}");
    }
}

#[test]
fn status_says_so_when_no_fan_sensor_exists() {
    let tmp = fixture();
    let out = ctl(tmp.path()).arg("status").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("fan-rpm: unavailable"), "{stdout}");
}

#[test]
fn status_on_an_unsupported_machine_says_so() {
    let tmp = TempDir::new().unwrap();
    let out = ctl(tmp.path()).arg("status").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("supported: no"), "{stdout}");
    assert!(stdout.contains("lenovo-wmi-gamezone"), "{stdout}");
}

#[test]
fn profile_reads_and_writes() {
    let tmp = fixture();
    ctl(tmp.path())
        .arg("profile")
        .assert()
        .success()
        .stdout(predicates::str::contains("profile: balanced (Balanced)"));

    ctl(tmp.path())
        .args(["profile", "performance"])
        .assert()
        .success();
    assert_eq!(
        read(
            tmp.path(),
            "sys/class/platform-profile/platform-profile-0/profile"
        ),
        "performance"
    );

    // The friendly name works too, and maps to the kernel's spelling.
    ctl(tmp.path())
        .args(["profile", "extreme"])
        .assert()
        .success();
    assert_eq!(
        read(
            tmp.path(),
            "sys/class/platform-profile/platform-profile-0/profile"
        ),
        "max-power"
    );
}

#[test]
fn unknown_profile_is_rejected_before_touching_sysfs() {
    let tmp = fixture();
    ctl(tmp.path())
        .args(["profile", "ludicrous"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("ludicrous"));
    assert_eq!(
        read(
            tmp.path(),
            "sys/class/platform-profile/platform-profile-0/profile"
        ),
        "balanced"
    );
}

#[test]
fn ppt_below_min_names_the_range() {
    let tmp = fixture();
    ctl(tmp.path())
        .args(["ppt", "spl", "20"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("50").and(predicates::str::contains("115")));

    // Unchanged.
    assert_eq!(
        read(
            tmp.path(),
            "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes/ppt_pl1_spl/current_value"
        ),
        "70"
    );
}

#[test]
fn ppt_lists_all_three_limits() {
    let tmp = fixture();
    let out = ctl(tmp.path()).arg("ppt").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("spl: 70 W"), "{stdout}");
    assert!(stdout.contains("sppt: 85 W"), "{stdout}");
    assert!(stdout.contains("fppt: 102 W"), "{stdout}");
    assert!(stdout.contains("range 50–115"), "{stdout}");
}

#[test]
fn ppt_unknown_attribute_lists_the_known_ones() {
    let tmp = fixture();
    ctl(tmp.path())
        .args(["ppt", "gpu"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("spl"));
}

#[test]
fn battery_mode_round_trips() {
    let tmp = fixture();
    ctl(tmp.path())
        .arg("battery")
        .assert()
        .success()
        .stdout(predicates::str::contains("Long_Life"));

    ctl(tmp.path())
        .args(["battery", "standard"])
        .assert()
        .success();
    assert_eq!(
        read(tmp.path(), "sys/class/power_supply/BAT0/charge_types"),
        "Standard"
    );

    // The hyphenated spelling maps to the kernel's underscore form.
    ctl(tmp.path())
        .args(["battery", "long-life"])
        .assert()
        .success();
    assert_eq!(
        read(tmp.path(), "sys/class/power_supply/BAT0/charge_types"),
        "Long_Life"
    );
}

#[test]
fn toggles_round_trip() {
    let tmp = fixture();
    for (sub, attr) in [
        ("fnlock", "fn_lock"),
        ("camera", "camera_power"),
        ("usb-charging", "usb_charging"),
    ] {
        ctl(tmp.path()).args([sub, "on"]).assert().success();
        assert_eq!(
            read(
                tmp.path(),
                &format!("sys/bus/platform/devices/VPC2004:00/{attr}")
            ),
            "1"
        );
        ctl(tmp.path())
            .arg(sub)
            .assert()
            .success()
            .stdout(predicates::str::contains(": on"));

        ctl(tmp.path()).args([sub, "off"]).assert().success();
        assert_eq!(
            read(
                tmp.path(),
                &format!("sys/bus/platform/devices/VPC2004:00/{attr}")
            ),
            "0"
        );
    }
}

#[test]
fn invalid_on_off_is_rejected() {
    let tmp = fixture();
    ctl(tmp.path())
        .args(["fnlock", "maybe"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("'on' or 'off'"));
}

#[test]
fn backlight_respects_max_brightness() {
    let tmp = fixture();
    ctl(tmp.path()).args(["backlight", "2"]).assert().success();
    assert_eq!(
        read(
            tmp.path(),
            "sys/class/leds/platform::kbd_backlight/brightness"
        ),
        "2"
    );

    ctl(tmp.path())
        .args(["backlight", "5"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("0–2"));
}

#[test]
fn boost_round_trips() {
    let tmp = fixture();
    ctl(tmp.path())
        .arg("boost")
        .assert()
        .success()
        .stdout(predicates::str::contains("boost: on"));

    ctl(tmp.path()).args(["boost", "off"]).assert().success();
    assert_eq!(
        read(tmp.path(), "sys/devices/system/cpu/cpufreq/boost"),
        "0"
    );
}

#[test]
fn write_to_a_read_only_tree_reports_a_permission_problem() {
    let tmp = fixture();
    let profile = tmp
        .path()
        .join("sys/class/platform-profile/platform-profile-0/profile");
    let mut perms = fs::metadata(&profile).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o444);
    }
    fs::set_permissions(&profile, perms).unwrap();

    ctl(tmp.path())
        .args(["profile", "performance"])
        .assert()
        .failure()
        .stderr(
            predicates::str::contains("permission denied").or(predicates::str::contains("legiond")),
        );
}

#[test]
fn fallback_hint_goes_to_stderr_not_stdout() {
    let tmp = fixture();
    let out = ctl(tmp.path()).arg("profile").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();

    assert!(stderr.contains("legiond not running"), "{stderr}");
    // stdout stays clean and parseable.
    assert!(!stdout.contains("legiond not running"), "{stdout}");
    assert!(stdout.starts_with("profile: "), "{stdout}");
}

#[test]
fn watch_without_a_daemon_explains_itself() {
    let tmp = fixture();
    ctl(tmp.path())
        .args(["profile", "--watch"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("legiond"));
}
