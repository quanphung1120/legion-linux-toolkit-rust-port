//! CPU power limits via the firmware-attributes class
//! (`/sys/class/firmware-attributes/lenovo-wmi-other-0/attributes/`).
//!
//! Three integer attributes are advertised by the 82WM's BIOS, all in **watts**:
//! `ppt_pl1_spl`, `ppt_pl2_sppt`, `ppt_pl3_fppt`. Writes return `EBUSY` unless
//! the platform profile is `custom` — the kernel enforces that gate, and it
//! surfaces here as [`HwError::Busy`].
//!
//! Newer mainline advertises more attributes (`dgpu_boost_clk`, `gpu_nv_ctgp`,
//! …). Nothing here hardcodes the list beyond the three named convenience
//! variants: [`list_all`] scans the directory, so extra attributes appear
//! automatically when a kernel starts exposing them.

use serde::{Deserialize, Serialize};

use crate::{HwError, Result, SysRoot};

const ATTR_DIR: &str = "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes";

/// The three power-limit attributes the 82WM exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PptAttr {
    /// Sustained power limit (PL1).
    Spl,
    /// Slow package power tracking (PL2).
    Sppt,
    /// Fast package power tracking (PL3).
    Fppt,
}

impl PptAttr {
    pub const ALL: [PptAttr; 3] = [PptAttr::Spl, PptAttr::Sppt, PptAttr::Fppt];

    /// The attribute's directory name under `attributes/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            PptAttr::Spl => "ppt_pl1_spl",
            PptAttr::Sppt => "ppt_pl2_sppt",
            PptAttr::Fppt => "ppt_pl3_fppt",
        }
    }

    /// Short name used by the CLI and D-Bus.
    pub fn key(self) -> &'static str {
        match self {
            PptAttr::Spl => "spl",
            PptAttr::Sppt => "sppt",
            PptAttr::Fppt => "fppt",
        }
    }

    /// Human label for the GUI.
    pub fn label(self) -> &'static str {
        match self {
            PptAttr::Spl => "Sustained (SPL)",
            PptAttr::Sppt => "Slow boost (SPPT)",
            PptAttr::Fppt => "Fast boost (FPPT)",
        }
    }

    /// Parse a CLI/D-Bus key.
    pub fn from_key(s: &str) -> Result<Self> {
        let s = s.trim().to_ascii_lowercase();
        PptAttr::ALL
            .into_iter()
            .find(|a| a.key() == s || a.dir_name() == s)
            .ok_or_else(|| HwError::Parse(format!("unknown power-limit attribute {s:?}")))
    }
}

/// A firmware attribute's current value and its firmware-declared bounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttrInfo {
    /// Short key (`spl`) for known attributes, else the raw directory name.
    pub key: String,
    /// `display_name` from the firmware, falling back to the directory name.
    pub display_name: String,
    pub current: u32,
    pub default: u32,
    pub min: u32,
    pub max: u32,
    /// Step size (`scalar_increment`); 1 on the 82WM.
    pub step: u32,
}

impl AttrInfo {
    /// Is `watts` inside the firmware-declared range?
    pub fn in_range(&self, watts: u32) -> bool {
        watts >= self.min && watts <= self.max
    }
}

/// Is the firmware-attributes interface present?
pub fn available(root: &SysRoot) -> bool {
    root.exists(ATTR_DIR)
}

/// Names of every attribute directory the firmware advertises.
pub fn available_attrs(root: &SysRoot) -> Vec<String> {
    root.list_dir(ATTR_DIR)
        .into_iter()
        .filter(|name| root.exists(&format!("{ATTR_DIR}/{name}/current_value")))
        .collect()
}

/// Read one attribute by its directory name.
pub fn read_by_name(root: &SysRoot, dir_name: &str) -> Result<AttrInfo> {
    let base = format!("{ATTR_DIR}/{dir_name}");
    if !root.exists(&format!("{base}/current_value")) {
        return Err(HwError::NotSupported);
    }
    let key = PptAttr::ALL
        .into_iter()
        .find(|a| a.dir_name() == dir_name)
        .map(|a| a.key().to_string())
        .unwrap_or_else(|| dir_name.to_string());

    Ok(AttrInfo {
        key,
        display_name: root
            .read_opt(&format!("{base}/display_name"))
            .unwrap_or_else(|| dir_name.to_string()),
        current: root.read_parsed(&format!("{base}/current_value"))?,
        default: root
            .read_parsed_opt(&format!("{base}/default_value"))
            .unwrap_or(0),
        min: root
            .read_parsed_opt(&format!("{base}/min_value"))
            .unwrap_or(0),
        max: root
            .read_parsed_opt(&format!("{base}/max_value"))
            .unwrap_or(u32::MAX),
        step: root
            .read_parsed_opt(&format!("{base}/scalar_increment"))
            .unwrap_or(1),
    })
}

/// Read one of the three known power limits.
pub fn read(root: &SysRoot, attr: PptAttr) -> Result<AttrInfo> {
    read_by_name(root, attr.dir_name())
}

/// Every advertised attribute, sorted by directory name.
pub fn list_all(root: &SysRoot) -> Vec<AttrInfo> {
    available_attrs(root)
        .into_iter()
        .filter_map(|name| read_by_name(root, &name).ok())
        .collect()
}

/// The three power limits, when present.
pub fn list_ppt(root: &SysRoot) -> Vec<AttrInfo> {
    PptAttr::ALL
        .into_iter()
        .filter_map(|a| read(root, a).ok())
        .collect()
}

/// Set a power limit, in watts.
///
/// The value is range-checked against the firmware's own `min_value`/
/// `max_value` before the write, so an out-of-range request fails with
/// [`HwError::Parse`] naming the range instead of an opaque `EINVAL`. A write
/// attempted outside Custom mode fails with [`HwError::Busy`].
pub fn write_by_name(root: &SysRoot, dir_name: &str, watts: u32) -> Result<()> {
    let info = read_by_name(root, dir_name)?;
    if !info.in_range(watts) {
        return Err(HwError::Parse(format!(
            "{watts} W is out of range for {}: {}–{} W",
            info.key, info.min, info.max
        )));
    }
    root.write(
        &format!("{ATTR_DIR}/{dir_name}/current_value"),
        &watts.to_string(),
    )
}

/// Set one of the three known power limits, in watts.
pub fn write(root: &SysRoot, attr: PptAttr, watts: u32) -> Result<()> {
    write_by_name(root, attr.dir_name(), watts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        for a in PptAttr::ALL {
            assert_eq!(PptAttr::from_key(a.key()).unwrap(), a);
            assert_eq!(PptAttr::from_key(a.dir_name()).unwrap(), a);
        }
        assert!(PptAttr::from_key("gpu_ctgp").is_err());
    }

    #[test]
    fn range_check() {
        let info = AttrInfo {
            key: "spl".into(),
            display_name: "spl".into(),
            current: 70,
            default: 70,
            min: 50,
            max: 115,
            step: 1,
        };
        assert!(info.in_range(50));
        assert!(info.in_range(115));
        assert!(!info.in_range(49));
        assert!(!info.in_range(116));
    }
}
