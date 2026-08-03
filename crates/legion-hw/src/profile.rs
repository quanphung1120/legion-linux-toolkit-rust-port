//! Platform profile (power mode) over
//! `/sys/class/platform-profile/platform-profile-0/`.
//!
//! Driver `lenovo-wmi-gamezone`. `choices` on the 82WM reads
//! `low-power balanced performance max-power custom`.

use serde::{Deserialize, Serialize};

use crate::{HwError, Result, SysRoot};

const DIR: &str = "sys/class/platform-profile/platform-profile-0";

/// Relative path of the `profile` file — also used by the POLLPRI watcher.
pub const PROFILE_FILE: &str = "sys/class/platform-profile/platform-profile-0/profile";

/// The power modes this laptop exposes, in the order Lenovo presents them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerProfile {
    Quiet,
    Balanced,
    Performance,
    Extreme,
    Custom,
}

impl PowerProfile {
    /// Every variant, in display order.
    pub const ALL: [PowerProfile; 5] = [
        PowerProfile::Quiet,
        PowerProfile::Balanced,
        PowerProfile::Performance,
        PowerProfile::Extreme,
        PowerProfile::Custom,
    ];

    /// The kernel's string for this profile.
    pub fn as_sysfs(self) -> &'static str {
        match self {
            PowerProfile::Quiet => "low-power",
            PowerProfile::Balanced => "balanced",
            PowerProfile::Performance => "performance",
            PowerProfile::Extreme => "max-power",
            PowerProfile::Custom => "custom",
        }
    }

    /// The name shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            PowerProfile::Quiet => "Quiet",
            PowerProfile::Balanced => "Balanced",
            PowerProfile::Performance => "Performance",
            PowerProfile::Extreme => "Extreme",
            PowerProfile::Custom => "Custom",
        }
    }

    /// The colour Lenovo's power-button LED shows in this mode. The tray uses
    /// it to tint its status dot.
    pub fn led_color(self) -> LedColor {
        match self {
            PowerProfile::Quiet => LedColor::Blue,
            PowerProfile::Balanced => LedColor::White,
            PowerProfile::Performance => LedColor::Red,
            PowerProfile::Extreme | PowerProfile::Custom => LedColor::Purple,
        }
    }

    /// Parse a kernel profile string.
    pub fn from_sysfs(s: &str) -> Result<Self> {
        PowerProfile::ALL
            .into_iter()
            .find(|p| p.as_sysfs() == s)
            .ok_or_else(|| HwError::Parse(format!("unknown platform profile {s:?}")))
    }

    /// Parse a user-supplied name — accepts both the kernel spelling and the
    /// friendly label (case-insensitive), so `legion-ctl profile extreme` and
    /// `legion-ctl profile max-power` both work.
    pub fn from_user(s: &str) -> Result<Self> {
        let s = s.trim().to_ascii_lowercase();
        PowerProfile::ALL
            .into_iter()
            .find(|p| p.as_sysfs() == s || p.label().to_ascii_lowercase() == s)
            .ok_or_else(|| HwError::Parse(format!("unknown power profile {s:?}")))
    }
}

/// Power-button LED colour associated with a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LedColor {
    Blue,
    White,
    Red,
    Purple,
}

impl LedColor {
    /// `#rrggbb`, for the GUI and the tray icon renderer.
    pub fn hex(self) -> &'static str {
        match self {
            LedColor::Blue => "#3b82f6",
            LedColor::White => "#e5e7eb",
            LedColor::Red => "#ef4444",
            LedColor::Purple => "#a855f7",
        }
    }
}

/// Is the platform-profile interface present at all?
pub fn available(root: &SysRoot) -> bool {
    root.exists(PROFILE_FILE)
}

/// The driver backing the platform-profile device (`lenovo-wmi-gamezone`).
pub fn driver_name(root: &SysRoot) -> Option<String> {
    root.read_opt(&format!("{DIR}/name"))
}

/// Profiles this machine's firmware advertises, in kernel order.
pub fn choices(root: &SysRoot) -> Result<Vec<PowerProfile>> {
    let raw = root.read(&format!("{DIR}/choices"))?;
    raw.split_whitespace()
        .map(PowerProfile::from_sysfs)
        .collect()
}

/// The active profile.
pub fn get(root: &SysRoot) -> Result<PowerProfile> {
    PowerProfile::from_sysfs(&root.read(PROFILE_FILE)?)
}

/// Switch profile.
pub fn set(root: &SysRoot, profile: PowerProfile) -> Result<()> {
    root.write(PROFILE_FILE, profile.as_sysfs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_strings_round_trip() {
        for p in PowerProfile::ALL {
            assert_eq!(PowerProfile::from_sysfs(p.as_sysfs()).unwrap(), p);
        }
    }

    #[test]
    fn extreme_is_max_power() {
        assert_eq!(PowerProfile::Extreme.as_sysfs(), "max-power");
        assert_eq!(
            PowerProfile::from_sysfs("low-power").unwrap(),
            PowerProfile::Quiet
        );
    }

    #[test]
    fn user_input_accepts_labels_and_kernel_names() {
        assert_eq!(
            PowerProfile::from_user("Extreme").unwrap(),
            PowerProfile::Extreme
        );
        assert_eq!(
            PowerProfile::from_user("max-power").unwrap(),
            PowerProfile::Extreme
        );
        assert!(PowerProfile::from_user("turbo").is_err());
    }

    #[test]
    fn led_colors_match_lenovo_mapping() {
        assert_eq!(PowerProfile::Quiet.led_color(), LedColor::Blue);
        assert_eq!(PowerProfile::Balanced.led_color(), LedColor::White);
        assert_eq!(PowerProfile::Performance.led_color(), LedColor::Red);
        assert_eq!(PowerProfile::Extreme.led_color(), LedColor::Purple);
        assert_eq!(PowerProfile::Custom.led_color(), LedColor::Purple);
    }
}
