mod common;

use common::{fixture, read};
use legion_hw::profile::{self, LedColor, PowerProfile};
use legion_hw::{HwError, SysRoot};
use tempfile::TempDir;

#[test]
fn get_maps_low_power_to_quiet() {
    let (_tmp, root) = fixture();
    common::write(
        &root,
        "sys/class/platform-profile/platform-profile-0/profile",
        "low-power",
    );
    assert_eq!(profile::get(&root).unwrap(), PowerProfile::Quiet);
}

#[test]
fn get_reads_the_live_value() {
    let (_tmp, root) = fixture();
    assert_eq!(profile::get(&root).unwrap(), PowerProfile::Balanced);
}

#[test]
fn set_extreme_writes_max_power() {
    let (_tmp, root) = fixture();
    profile::set(&root, PowerProfile::Extreme).unwrap();
    assert_eq!(
        read(
            &root,
            "sys/class/platform-profile/platform-profile-0/profile"
        ),
        "max-power"
    );
    assert_eq!(profile::get(&root).unwrap(), PowerProfile::Extreme);
}

#[test]
fn set_custom_round_trips() {
    let (_tmp, root) = fixture();
    profile::set(&root, PowerProfile::Custom).unwrap();
    assert_eq!(profile::get(&root).unwrap(), PowerProfile::Custom);
}

#[test]
fn unknown_value_in_file_is_a_parse_error() {
    let (_tmp, root) = fixture();
    common::write(
        &root,
        "sys/class/platform-profile/platform-profile-0/profile",
        "ludicrous-speed",
    );
    assert!(matches!(profile::get(&root), Err(HwError::Parse(_))));
}

#[test]
fn missing_file_is_not_supported() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    assert!(!profile::available(&root));
    assert!(matches!(profile::get(&root), Err(HwError::NotSupported)));
    assert!(matches!(
        profile::set(&root, PowerProfile::Balanced),
        Err(HwError::NotSupported)
    ));
}

#[test]
fn choices_parse_in_kernel_order() {
    let (_tmp, root) = fixture();
    let choices = profile::choices(&root).unwrap();
    assert_eq!(
        choices,
        vec![
            PowerProfile::Quiet,
            PowerProfile::Balanced,
            PowerProfile::Performance,
            PowerProfile::Extreme,
            PowerProfile::Custom,
        ]
    );
}

#[test]
fn choices_with_an_unknown_entry_fail_loudly() {
    let (_tmp, root) = fixture();
    common::write(
        &root,
        "sys/class/platform-profile/platform-profile-0/choices",
        "balanced quantum",
    );
    assert!(matches!(profile::choices(&root), Err(HwError::Parse(_))));
}

#[test]
fn driver_is_lenovo_wmi_gamezone() {
    let (_tmp, root) = fixture();
    assert_eq!(
        profile::driver_name(&root).as_deref(),
        Some("lenovo-wmi-gamezone")
    );
}

#[test]
fn led_colors_are_exposed_for_the_tray() {
    let (_tmp, root) = fixture();
    let p = profile::get(&root).unwrap();
    assert_eq!(p.led_color(), LedColor::White);
    assert!(LedColor::Blue.hex().starts_with('#'));
}
