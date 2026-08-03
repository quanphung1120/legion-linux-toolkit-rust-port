mod common;

use common::{fixture, read};
use legion_hw::firmware_attrs::{self, PptAttr};
use legion_hw::{HwError, SysRoot};
use tempfile::TempDir;

#[test]
fn spl_reads_the_live_values() {
    let (_tmp, root) = fixture();
    let info = firmware_attrs::read(&root, PptAttr::Spl).unwrap();
    assert_eq!(info.current, 70);
    assert_eq!(info.default, 70);
    assert_eq!(info.min, 50);
    assert_eq!(info.max, 115);
    assert_eq!(info.step, 1);
    assert_eq!(info.key, "spl");
}

#[test]
fn sppt_and_fppt_read_the_live_values() {
    let (_tmp, root) = fixture();
    let sppt = firmware_attrs::read(&root, PptAttr::Sppt).unwrap();
    assert_eq!((sppt.current, sppt.min, sppt.max), (85, 60, 130));
    let fppt = firmware_attrs::read(&root, PptAttr::Fppt).unwrap();
    assert_eq!((fppt.current, fppt.min, fppt.max), (102, 70, 150));
}

#[test]
fn write_in_range_updates_current_value() {
    let (_tmp, root) = fixture();
    firmware_attrs::write(&root, PptAttr::Spl, 90).unwrap();
    assert_eq!(
        read(
            &root,
            "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes/ppt_pl1_spl/current_value"
        ),
        "90"
    );
    assert_eq!(
        firmware_attrs::read(&root, PptAttr::Spl).unwrap().current,
        90
    );
}

#[test]
fn write_below_min_is_rejected_with_the_range_named() {
    let (_tmp, root) = fixture();
    let err = firmware_attrs::write(&root, PptAttr::Spl, 20).unwrap_err();
    match err {
        HwError::Parse(msg) => {
            assert!(msg.contains("50"), "message should name the min: {msg}");
            assert!(msg.contains("115"), "message should name the max: {msg}");
        }
        other => panic!("expected Parse, got {other:?}"),
    }
    // Rejected client-side — the firmware was never asked.
    assert_eq!(
        firmware_attrs::read(&root, PptAttr::Spl).unwrap().current,
        70
    );
}

#[test]
fn write_above_max_is_rejected() {
    let (_tmp, root) = fixture();
    assert!(matches!(
        firmware_attrs::write(&root, PptAttr::Fppt, 999),
        Err(HwError::Parse(_))
    ));
}

#[test]
fn boundary_values_are_accepted() {
    let (_tmp, root) = fixture();
    firmware_attrs::write(&root, PptAttr::Spl, 50).unwrap();
    firmware_attrs::write(&root, PptAttr::Spl, 115).unwrap();
    assert_eq!(
        firmware_attrs::read(&root, PptAttr::Spl).unwrap().current,
        115
    );
}

#[test]
fn absent_attribute_directory_is_not_supported() {
    let tmp = TempDir::new().unwrap();
    let root = SysRoot::at(tmp.path());
    assert!(!firmware_attrs::available(&root));
    assert!(matches!(
        firmware_attrs::read(&root, PptAttr::Spl),
        Err(HwError::NotSupported)
    ));
    assert!(firmware_attrs::list_all(&root).is_empty());
}

#[test]
fn list_ppt_returns_all_three_in_order() {
    let (_tmp, root) = fixture();
    let all = firmware_attrs::list_ppt(&root);
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].key, "spl");
    assert_eq!(all[1].key, "sppt");
    assert_eq!(all[2].key, "fppt");
}

#[test]
fn list_all_scans_rather_than_hardcoding() {
    let (_tmp, root) = fixture();
    let fa = "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes";
    // A future kernel starts advertising a GPU attribute.
    common::write(&root, &format!("{fa}/gpu_nv_ctgp/current_value"), "15");
    common::write(&root, &format!("{fa}/gpu_nv_ctgp/min_value"), "0");
    common::write(&root, &format!("{fa}/gpu_nv_ctgp/max_value"), "25");

    let all = firmware_attrs::list_all(&root);
    assert_eq!(all.len(), 4);
    let gpu = all.iter().find(|a| a.key == "gpu_nv_ctgp").unwrap();
    assert_eq!(gpu.current, 15);
    // Unknown attributes are still writable through the by-name API.
    firmware_attrs::write_by_name(&root, "gpu_nv_ctgp", 20).unwrap();
    assert_eq!(
        firmware_attrs::read_by_name(&root, "gpu_nv_ctgp")
            .unwrap()
            .current,
        20
    );
}

#[test]
fn attribute_without_current_value_is_ignored() {
    let (_tmp, root) = fixture();
    let fa = "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes";
    common::write(&root, &format!("{fa}/broken_attr/display_name"), "broken");
    assert_eq!(firmware_attrs::list_all(&root).len(), 3);
    assert!(matches!(
        firmware_attrs::read_by_name(&root, "broken_attr"),
        Err(HwError::NotSupported)
    ));
}
