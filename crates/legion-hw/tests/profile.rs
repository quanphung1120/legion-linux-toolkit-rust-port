mod common;

use common::{fixture, read};
use legion_hw::profile::{self, LedColor, PowerProfile};
use legion_hw::profile_watch::ProfileWatchFd;
use legion_hw::{HwError, SysRoot, detect};
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

#[test]
fn device_is_discovered_by_name_not_by_index() {
    // The out-of-tree LenovoLegionLinux `legion_laptop` module registers a
    // second platform-profile handler named `lenovo-legion`. Module load order
    // is not stable across boots, so it can land on `platform-profile-0` and
    // push the upstream driver to `platform-profile-1`. Everything must follow
    // the name.
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());

    let lll = "sys/class/platform-profile/platform-profile-0";
    common::write(&root, &format!("{lll}/name"), "lenovo-legion");
    // Note the missing `max-power`: LLL's handler advertises a different set.
    common::write(
        &root,
        &format!("{lll}/choices"),
        "low-power balanced performance custom",
    );
    common::write(&root, &format!("{lll}/profile"), "balanced");

    let gz = "sys/class/platform-profile/platform-profile-1";
    common::write(&root, &format!("{gz}/name"), "lenovo-wmi-gamezone");
    common::write(
        &root,
        &format!("{gz}/choices"),
        "low-power balanced performance max-power custom",
    );
    common::write(&root, &format!("{gz}/profile"), "performance");

    assert_eq!(profile::device_dir(&root).as_deref(), Some(gz));
    assert_eq!(
        profile::profile_file(&root).as_deref(),
        Some(format!("{gz}/profile").as_str())
    );

    // Reads come from the gamezone device, not the one at index 0.
    assert!(profile::available(&root));
    assert_eq!(profile::get(&root).unwrap(), PowerProfile::Performance);
    assert_eq!(
        profile::driver_name(&root).as_deref(),
        Some("lenovo-wmi-gamezone")
    );
    assert_eq!(profile::choices(&root).unwrap(), PowerProfile::ALL.to_vec());

    // Writes land on the gamezone device and leave the LLL handler untouched.
    profile::set(&root, PowerProfile::Extreme).unwrap();
    assert_eq!(read(&root, &format!("{gz}/profile")), "max-power");
    assert_eq!(read(&root, &format!("{lll}/profile")), "balanced");

    // Capability detection reports the gamezone driver and its five choices.
    let caps = detect::probe(&root);
    assert!(caps.platform_profile);
    assert_eq!(
        caps.platform_profile_driver.as_deref(),
        Some("lenovo-wmi-gamezone")
    );
    assert_eq!(caps.profile_choices.len(), 5);

    // The Fn+Q watcher opens the gamezone `profile` file: it sees `max-power`,
    // which the LLL handler at index 0 cannot even represent.
    let mut watch = ProfileWatchFd::open(&root).unwrap();
    assert_eq!(watch.consume().unwrap(), PowerProfile::Extreme);
}

#[test]
fn falls_back_to_the_first_device_without_a_gamezone() {
    // A machine where only some other handler is registered still works — this
    // is the pre-discovery behaviour, preserved.
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());

    let dir = "sys/class/platform-profile/platform-profile-0";
    common::write(&root, &format!("{dir}/name"), "lenovo-legion");
    common::write(
        &root,
        &format!("{dir}/choices"),
        "low-power balanced performance custom",
    );
    common::write(&root, &format!("{dir}/profile"), "performance");

    assert_eq!(profile::device_dir(&root).as_deref(), Some(dir));
    assert!(profile::available(&root));
    assert_eq!(profile::get(&root).unwrap(), PowerProfile::Performance);
    assert_eq!(
        profile::driver_name(&root).as_deref(),
        Some("lenovo-legion")
    );
    assert_eq!(profile::choices(&root).unwrap().len(), 4);

    profile::set(&root, PowerProfile::Quiet).unwrap();
    assert_eq!(read(&root, &format!("{dir}/profile")), "low-power");
}

#[test]
fn a_device_without_a_name_file_is_still_used() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    common::write(
        &root,
        "sys/class/platform-profile/platform-profile-0/profile",
        "balanced",
    );

    assert_eq!(
        profile::device_dir(&root).as_deref(),
        Some("sys/class/platform-profile/platform-profile-0")
    );
    assert!(profile::available(&root));
    assert_eq!(profile::get(&root).unwrap(), PowerProfile::Balanced);
    assert_eq!(profile::driver_name(&root), None);
}

#[test]
fn an_empty_class_directory_is_not_supported() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    std::fs::create_dir_all(root.path("sys/class/platform-profile")).unwrap();

    assert_eq!(profile::device_dir(&root), None);
    assert_eq!(profile::profile_file(&root), None);
    assert!(!profile::available(&root));
    assert!(matches!(profile::get(&root), Err(HwError::NotSupported)));
    assert!(matches!(
        profile::choices(&root),
        Err(HwError::NotSupported)
    ));
    assert!(matches!(
        profile::set(&root, PowerProfile::Balanced),
        Err(HwError::NotSupported)
    ));
}
