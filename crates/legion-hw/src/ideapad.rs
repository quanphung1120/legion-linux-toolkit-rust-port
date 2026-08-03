//! `ideapad_laptop` platform-device toggles at
//! `/sys/bus/platform/devices/VPC2004:00/`.
//!
//! The 82WM exposes `fn_lock`, `camera_power`, `usb_charging` and `fan_mode`.
//! It does **not** expose `touchpad`, so every accessor here feature-detects
//! and returns `None` for an absent attribute rather than failing — the UI
//! hides what the machine does not have.

use serde::{Deserialize, Serialize};

use crate::{HwError, Result, SysRoot};

const DEV: &str = "sys/bus/platform/devices/VPC2004:00";

/// The boolean toggles this driver exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Toggle {
    FnLock,
    CameraPower,
    UsbCharging,
    /// Deprecated in favour of `charge_types`; read-only here, exposed only so
    /// [`crate::detect`] can report it on older kernels.
    ConservationMode,
}

impl Toggle {
    pub const ALL: [Toggle; 4] = [
        Toggle::FnLock,
        Toggle::CameraPower,
        Toggle::UsbCharging,
        Toggle::ConservationMode,
    ];

    pub fn attr(self) -> &'static str {
        match self {
            Toggle::FnLock => "fn_lock",
            Toggle::CameraPower => "camera_power",
            Toggle::UsbCharging => "usb_charging",
            Toggle::ConservationMode => "conservation_mode",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Toggle::FnLock => "Fn Lock",
            Toggle::CameraPower => "Camera",
            Toggle::UsbCharging => "Always-on USB charging",
            Toggle::ConservationMode => "Conservation mode (legacy)",
        }
    }
}

/// Is this toggle present on this machine?
pub fn has(root: &SysRoot, t: Toggle) -> bool {
    root.exists(&format!("{DEV}/{}", t.attr()))
}

/// Read a toggle; `None` when the attribute is absent or unreadable.
pub fn get(root: &SysRoot, t: Toggle) -> Option<bool> {
    root.read_parsed_opt::<u32>(&format!("{DEV}/{}", t.attr()))
        .map(|v| v != 0)
}

/// Write a toggle.
pub fn set(root: &SysRoot, t: Toggle, on: bool) -> Result<()> {
    root.write(&format!("{DEV}/{}", t.attr()), if on { "1" } else { "0" })
}

/// ideapad fan modes. Value 3 is not a valid setting on this firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FanMode {
    SuperSilent,
    Standard,
    DustCleaning,
    EfficientThermalDissipation,
}

impl FanMode {
    pub const ALL: [FanMode; 4] = [
        FanMode::SuperSilent,
        FanMode::Standard,
        FanMode::DustCleaning,
        FanMode::EfficientThermalDissipation,
    ];

    pub fn value(self) -> u32 {
        match self {
            FanMode::SuperSilent => 0,
            FanMode::Standard => 1,
            FanMode::DustCleaning => 2,
            // 3 is skipped — the firmware rejects it.
            FanMode::EfficientThermalDissipation => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FanMode::SuperSilent => "Super Silent",
            FanMode::Standard => "Standard",
            FanMode::DustCleaning => "Dust Cleaning",
            FanMode::EfficientThermalDissipation => "Efficient Thermal Dissipation",
        }
    }

    /// Parse a raw sysfs integer. `3` is explicitly rejected.
    pub fn from_value(v: u32) -> Result<Self> {
        FanMode::ALL
            .into_iter()
            .find(|m| m.value() == v)
            .ok_or_else(|| HwError::Parse(format!("invalid fan_mode value {v}")))
    }
}

/// Is `fan_mode` present?
pub fn has_fan_mode(root: &SysRoot) -> bool {
    root.exists(&format!("{DEV}/fan_mode"))
}

/// Read the current fan mode.
pub fn get_fan_mode(root: &SysRoot) -> Result<FanMode> {
    FanMode::from_value(root.read_parsed(&format!("{DEV}/fan_mode"))?)
}

/// Set the fan mode.
pub fn set_fan_mode(root: &SysRoot, mode: FanMode) -> Result<()> {
    root.write(&format!("{DEV}/fan_mode"), &mode.value().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_mode_values_match_driver() {
        assert_eq!(FanMode::SuperSilent.value(), 0);
        assert_eq!(FanMode::Standard.value(), 1);
        assert_eq!(FanMode::DustCleaning.value(), 2);
        assert_eq!(FanMode::EfficientThermalDissipation.value(), 4);
    }

    #[test]
    fn fan_mode_rejects_three() {
        assert!(FanMode::from_value(3).is_err());
        assert!(FanMode::from_value(5).is_err());
        assert_eq!(
            FanMode::from_value(4).unwrap(),
            FanMode::EfficientThermalDissipation
        );
    }

    #[test]
    fn attribute_names_are_the_driver_names() {
        assert_eq!(Toggle::FnLock.attr(), "fn_lock");
        assert_eq!(Toggle::CameraPower.attr(), "camera_power");
        assert_eq!(Toggle::UsbCharging.attr(), "usb_charging");
    }
}
