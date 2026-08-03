mod common;

use common::{fixture, read};
use legion_hw::ideapad::{self, FanMode, Toggle};
use legion_hw::{HwError, SysRoot};
use tempfile::TempDir;

#[test]
fn every_toggle_round_trips() {
    let (_tmp, root) = fixture();
    for t in [Toggle::FnLock, Toggle::CameraPower, Toggle::UsbCharging] {
        assert!(ideapad::has(&root, t), "{t:?} should exist on the 82WM");

        ideapad::set(&root, t, true).unwrap();
        assert_eq!(ideapad::get(&root, t), Some(true));
        assert_eq!(
            read(
                &root,
                &format!("sys/bus/platform/devices/VPC2004:00/{}", t.attr())
            ),
            "1"
        );

        ideapad::set(&root, t, false).unwrap();
        assert_eq!(ideapad::get(&root, t), Some(false));
    }
}

#[test]
fn live_values_read_correctly() {
    let (_tmp, root) = fixture();
    assert_eq!(ideapad::get(&root, Toggle::FnLock), Some(false));
    assert_eq!(ideapad::get(&root, Toggle::CameraPower), Some(true));
    assert_eq!(ideapad::get(&root, Toggle::UsbCharging), Some(false));
}

#[test]
fn absent_attribute_is_none_and_hidden() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    for t in Toggle::ALL {
        assert!(!ideapad::has(&root, t));
        assert_eq!(ideapad::get(&root, t), None);
        assert!(matches!(
            ideapad::set(&root, t, true),
            Err(HwError::NotSupported)
        ));
    }
}

#[test]
fn fan_mode_reads_super_silent() {
    let (_tmp, root) = fixture();
    assert!(ideapad::has_fan_mode(&root));
    assert_eq!(ideapad::get_fan_mode(&root).unwrap(), FanMode::SuperSilent);
}

#[test]
fn fan_mode_round_trips_including_the_gap_at_four() {
    let (_tmp, root) = fixture();
    ideapad::set_fan_mode(&root, FanMode::EfficientThermalDissipation).unwrap();
    assert_eq!(
        read(&root, "sys/bus/platform/devices/VPC2004:00/fan_mode"),
        "4"
    );
    assert_eq!(
        ideapad::get_fan_mode(&root).unwrap(),
        FanMode::EfficientThermalDissipation
    );

    ideapad::set_fan_mode(&root, FanMode::Standard).unwrap();
    assert_eq!(ideapad::get_fan_mode(&root).unwrap(), FanMode::Standard);
}

#[test]
fn fan_mode_three_is_rejected_when_read_back() {
    let (_tmp, root) = fixture();
    // The firmware never reports 3, but if it did we must not invent a mode.
    common::write(&root, "sys/bus/platform/devices/VPC2004:00/fan_mode", "3");
    assert!(matches!(
        ideapad::get_fan_mode(&root),
        Err(HwError::Parse(_))
    ));
}

#[test]
fn touchpad_is_absent_on_this_machine() {
    let (_tmp, root) = fixture();
    // Documented in the plan: VPC2004:00 has no `touchpad` attribute here,
    // so nothing must depend on it.
    assert!(!root.exists("sys/bus/platform/devices/VPC2004:00/touchpad"));
}
