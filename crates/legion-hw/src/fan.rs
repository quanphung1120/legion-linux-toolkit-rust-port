//! Fan speed readout.
//!
//! No fan hwmon exists on the 82WM with kernel 7.0 — verified live, there is
//! no `fanN_input` anywhere under `/sys/class/hwmon`. Newer mainline adds a
//! `lenovo-wmi-other` hwmon carrying `fanN_input`/`fanN_target`, so this module
//! *scans* rather than hardcoding: it returns `None` today and starts
//! reporting the moment a kernel exposes the sensors. Nothing here needs to
//! change when that happens.
//!
//! Fan *curves* are deliberately absent: no upstream interface exists.

use serde::{Deserialize, Serialize};

use crate::SysRoot;

/// One fan's readings, in RPM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanReading {
    /// hwmon attribute index (`fan1_input` → 1).
    pub index: u32,
    /// Measured speed.
    pub rpm: u32,
    /// Firmware's target speed, when the driver exposes `fanN_target`.
    pub target_rpm: Option<u32>,
    /// hwmon `name` the reading came from.
    pub source: String,
}

/// Every fan the kernel currently reports, across all hwmon devices.
pub fn readings(root: &SysRoot) -> Vec<FanReading> {
    let mut out = Vec::new();
    for entry in root.list_dir("sys/class/hwmon") {
        let hwmon = format!("sys/class/hwmon/{entry}");
        let source = root
            .read_opt(&format!("{hwmon}/name"))
            .unwrap_or_else(|| entry.clone());

        let mut names: Vec<String> = root
            .list_dir(&hwmon)
            .into_iter()
            .filter(|n| n.starts_with("fan") && n.ends_with("_input"))
            .collect();
        names.sort();

        for name in names {
            let Some(index) = name
                .strip_prefix("fan")
                .and_then(|s| s.strip_suffix("_input"))
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(rpm) = root.read_parsed_opt::<u32>(&format!("{hwmon}/{name}")) else {
                continue;
            };
            out.push(FanReading {
                index,
                rpm,
                target_rpm: root.read_parsed_opt(&format!("{hwmon}/fan{index}_target")),
                source: source.clone(),
            });
        }
    }
    out
}

/// Speed of the first fan, or `None` when the kernel exposes no fan sensors.
pub fn rpm(root: &SysRoot) -> Option<u32> {
    readings(root).first().map(|r| r.rpm)
}

/// Does this kernel expose any fan sensor at all?
pub fn available(root: &SysRoot) -> bool {
    !readings(root).is_empty()
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

    #[test]
    fn no_fan_sensors_on_the_82wm_today() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        write(&root, "sys/class/hwmon/hwmon0/name", "k10temp");
        write(&root, "sys/class/hwmon/hwmon0/temp1_input", "50000");
        assert_eq!(rpm(&root), None);
        assert!(!available(&root));
    }

    #[test]
    fn lights_up_when_the_kernel_gains_the_hwmon() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        write(&root, "sys/class/hwmon/hwmon1/name", "lenovo_wmi_other");
        write(&root, "sys/class/hwmon/hwmon1/fan1_input", "2400");
        write(&root, "sys/class/hwmon/hwmon1/fan1_target", "2600");
        write(&root, "sys/class/hwmon/hwmon1/fan2_input", "2500");

        let r = readings(&root);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].index, 1);
        assert_eq!(r[0].rpm, 2400);
        assert_eq!(r[0].target_rpm, Some(2600));
        assert_eq!(r[0].source, "lenovo_wmi_other");
        assert_eq!(r[1].target_rpm, None);
        assert_eq!(rpm(&root), Some(2400));
        assert!(available(&root));
    }
}
