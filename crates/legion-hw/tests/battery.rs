mod common;

use common::{fixture, read};
use legion_hw::battery::{self, ChargeMode};
use legion_hw::{HwError, SysRoot};
use tempfile::TempDir;

#[test]
fn live_charge_types_value_parses() {
    let (_tmp, root) = fixture();
    let ct = battery::charge_types(&root).unwrap();
    assert_eq!(ct.active, Some(ChargeMode::LongLife));
    assert_eq!(
        ct.available,
        vec![ChargeMode::Fast, ChargeMode::Standard, ChargeMode::LongLife]
    );
    assert_eq!(battery::get_mode(&root).unwrap(), ChargeMode::LongLife);
}

#[test]
fn set_mode_writes_the_underscored_kernel_name() {
    let (_tmp, root) = fixture();
    battery::set_mode(&root, ChargeMode::LongLife).unwrap();
    assert_eq!(
        read(&root, "sys/class/power_supply/BAT0/charge_types"),
        "Long_Life"
    );

    battery::set_mode(&root, ChargeMode::Standard).unwrap();
    assert_eq!(
        read(&root, "sys/class/power_supply/BAT0/charge_types"),
        "Standard"
    );
}

#[test]
fn stats_skip_the_absent_temp_attribute() {
    let (_tmp, root) = fixture();
    let s = battery::stats(&root);
    assert_eq!(s.capacity, Some(78));
    assert_eq!(s.cycle_count, Some(42));
    assert_eq!(s.status.as_deref(), Some("Discharging"));
    assert_eq!(s.voltage_now, Some(15_400_000));
    assert_eq!(s.power_now, Some(12_000_000));
    // No panic, no error — there simply is no temperature on this battery,
    // and BatteryStats has no field for one.
}

#[test]
fn capacity_is_computed_from_energy() {
    let (_tmp, root) = fixture();
    let s = battery::stats(&root);
    // 62_000_000 / 80_000_000 = 77.5%
    let pct = s.capacity_from_energy.unwrap();
    assert!((pct - 77.5).abs() < 1e-9, "got {pct}");
}

#[test]
fn health_is_full_over_design() {
    let (_tmp, root) = fixture();
    let s = battery::stats(&root);
    // 80_000_000 / 99_900_000 = 80.08%
    let health = s.health_pct.unwrap();
    assert!((health - 80.080_080_08).abs() < 1e-6, "got {health}");
}

#[test]
fn stats_on_an_empty_tree_are_all_none() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    let s = battery::stats(&root);
    assert_eq!(s.capacity, None);
    assert_eq!(s.capacity_from_energy, None);
    assert_eq!(s.health_pct, None);
    assert_eq!(s.ac_online, None);
    assert!(!battery::available(&root));
}

#[test]
fn ac_adapter_is_found_by_type_not_by_name() {
    let (_tmp, root) = fixture();
    assert_eq!(battery::ac_online(&root), Some(true));

    common::write(&root, "sys/class/power_supply/ADP0/online", "0");
    assert_eq!(battery::ac_online(&root), Some(false));
}

#[test]
fn ac_adapter_with_an_unusual_name_still_found() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    common::write(&root, "sys/class/power_supply/AC/type", "Mains");
    common::write(&root, "sys/class/power_supply/AC/online", "1");
    assert_eq!(battery::ac_online(&root), Some(true));
}

#[test]
fn missing_charge_types_is_not_supported() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    assert!(matches!(
        battery::charge_types(&root),
        Err(HwError::NotSupported)
    ));
    assert!(matches!(
        battery::set_mode(&root, ChargeMode::Fast),
        Err(HwError::NotSupported)
    ));
}

#[test]
fn charge_types_without_an_active_entry_has_no_mode() {
    let (_tmp, root) = fixture();
    common::write(
        &root,
        "sys/class/power_supply/BAT0/charge_types",
        "Fast Standard Long_Life",
    );
    let ct = battery::charge_types(&root).unwrap();
    assert_eq!(ct.available.len(), 3);
    assert_eq!(ct.active, None);
    assert!(matches!(battery::get_mode(&root), Err(HwError::Parse(_))));
}
