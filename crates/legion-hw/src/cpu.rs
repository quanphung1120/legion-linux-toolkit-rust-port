//! CPU boost, governor and temperature.
//!
//! Boost lives at `/sys/devices/system/cpu/cpufreq/boost` (the AMD
//! acpi-cpufreq / amd-pstate path). Governor, EPP and current frequency are
//! read-only display values. Temperature comes from the `k10temp` hwmon, found
//! by scanning `name` files rather than hardcoding an index — hwmon numbering
//! is not stable across boots.

use serde::{Deserialize, Serialize};

use crate::sysfs::find_hwmon;
use crate::{Result, SysRoot};

const CPUFREQ: &str = "sys/devices/system/cpu/cpufreq";
const CPU0: &str = "sys/devices/system/cpu/cpu0/cpufreq";

/// Is the global boost toggle present?
pub fn has_boost(root: &SysRoot) -> bool {
    root.exists(&format!("{CPUFREQ}/boost"))
}

/// Is boost enabled?
pub fn get_boost(root: &SysRoot) -> Option<bool> {
    root.read_parsed_opt::<u32>(&format!("{CPUFREQ}/boost"))
        .map(|v| v != 0)
}

/// Enable or disable boost.
pub fn set_boost(root: &SysRoot, on: bool) -> Result<()> {
    root.write(&format!("{CPUFREQ}/boost"), if on { "1" } else { "0" })
}

/// Active scaling governor (e.g. `powersave` with amd-pstate).
pub fn governor(root: &SysRoot) -> Option<String> {
    root.read_opt(&format!("{CPU0}/scaling_governor"))
}

/// Active energy/performance preference.
pub fn epp(root: &SysRoot) -> Option<String> {
    root.read_opt(&format!("{CPU0}/energy_performance_preference"))
}

/// Current frequency of cpu0, in kHz as the kernel reports it.
pub fn cur_freq_khz(root: &SysRoot) -> Option<u64> {
    root.read_parsed_opt(&format!("{CPU0}/scaling_cur_freq"))
}

/// CPU package temperature in degrees Celsius, from the `k10temp` hwmon.
///
/// Prefers the sensor labelled `Tctl`/`Tdie` when labels are present, else
/// falls back to the lowest-numbered `tempN_input`.
pub fn temp_celsius(root: &SysRoot) -> Option<f64> {
    let hwmon = find_hwmon(root, "k10temp")?;
    let entries = root.list_dir(&hwmon);

    let mut inputs: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("temp") && n.ends_with("_input"))
        .collect();
    inputs.sort();

    // Prefer a labelled Tctl/Tdie sensor when the driver provides labels.
    let preferred = inputs.iter().find(|input| {
        let label_file = input.replace("_input", "_label");
        root.read_opt(&format!("{hwmon}/{label_file}"))
            .is_some_and(|l| l.eq_ignore_ascii_case("Tctl") || l.eq_ignore_ascii_case("Tdie"))
    });

    let chosen = preferred.copied().or_else(|| inputs.first().copied())?;
    let millidegrees: f64 = root.read_parsed_opt(&format!("{hwmon}/{chosen}"))?;
    Some(millidegrees / 1000.0)
}

/// A snapshot of the read-only CPU display values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub boost: Option<bool>,
    pub governor: Option<String>,
    pub epp: Option<String>,
    pub cur_freq_khz: Option<u64>,
    pub temp_celsius: Option<f64>,
}

/// Read everything the CPU pages display, skipping absent values.
pub fn info(root: &SysRoot) -> CpuInfo {
    CpuInfo {
        boost: get_boost(root),
        governor: governor(root),
        epp: epp(root),
        cur_freq_khz: cur_freq_khz(root),
        temp_celsius: temp_celsius(root),
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

    #[test]
    fn boost_round_trips() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        write(&root, &format!("{CPUFREQ}/boost"), "1");
        assert_eq!(get_boost(&root), Some(true));
        set_boost(&root, false).unwrap();
        assert_eq!(get_boost(&root), Some(false));
    }

    #[test]
    fn absent_boost_is_none() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        assert!(!has_boost(&root));
        assert_eq!(get_boost(&root), None);
    }

    #[test]
    fn k10temp_found_among_other_hwmons() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        // An nvme hwmon sorts first; name filtering must skip it.
        write(&root, "sys/class/hwmon/hwmon0/name", "nvme");
        write(&root, "sys/class/hwmon/hwmon0/temp1_input", "45000");
        write(&root, "sys/class/hwmon/hwmon1/name", "k10temp");
        write(&root, "sys/class/hwmon/hwmon1/temp1_input", "52125");
        assert_eq!(temp_celsius(&root), Some(52.125));
    }

    #[test]
    fn k10temp_prefers_tctl_label() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        write(&root, "sys/class/hwmon/hwmon2/name", "k10temp");
        write(&root, "sys/class/hwmon/hwmon2/temp1_input", "30000");
        write(&root, "sys/class/hwmon/hwmon2/temp1_label", "Tccd1");
        write(&root, "sys/class/hwmon/hwmon2/temp2_input", "61000");
        write(&root, "sys/class/hwmon/hwmon2/temp2_label", "Tctl");
        assert_eq!(temp_celsius(&root), Some(61.0));
    }

    #[test]
    fn no_k10temp_is_none() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        write(&root, "sys/class/hwmon/hwmon0/name", "nvme");
        assert_eq!(temp_celsius(&root), None);
    }
}
