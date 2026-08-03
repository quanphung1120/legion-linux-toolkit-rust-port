//! Battery charge mode and statistics over the standard power_supply ABI.
//!
//! Charging behaviour is controlled through `charge_types` (kernel 6.19+),
//! which supersedes ideapad's deprecated `conservation_mode` and covers rapid
//! charging too. The file reads as a list of every supported type with the
//! active one bracketed, e.g. `Fast Standard [Long_Life]`; writing one of the
//! bare names selects it. The modes are mutually exclusive — the kernel
//! enforces that, so this module never has to.
//!
//! Note: this battery exposes no `temp` attribute, so [`BatteryStats`] has no
//! temperature field.

use serde::{Deserialize, Serialize};

use crate::{HwError, Result, SysRoot};

const BAT: &str = "sys/class/power_supply/BAT0";
const SUPPLIES: &str = "sys/class/power_supply";

/// How the firmware is allowed to charge the battery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChargeMode {
    /// Normal charging to 100%.
    Standard,
    /// Rapid charge.
    Fast,
    /// Conservation mode — caps around 60–80% to preserve battery health.
    LongLife,
}

impl ChargeMode {
    pub const ALL: [ChargeMode; 3] = [ChargeMode::Standard, ChargeMode::Fast, ChargeMode::LongLife];

    /// The kernel's spelling (note the underscore in `Long_Life`).
    pub fn as_sysfs(self) -> &'static str {
        match self {
            ChargeMode::Standard => "Standard",
            ChargeMode::Fast => "Fast",
            ChargeMode::LongLife => "Long_Life",
        }
    }

    /// Short key for the CLI and D-Bus.
    pub fn key(self) -> &'static str {
        match self {
            ChargeMode::Standard => "standard",
            ChargeMode::Fast => "fast",
            ChargeMode::LongLife => "long-life",
        }
    }

    /// Label for the GUI.
    pub fn label(self) -> &'static str {
        match self {
            ChargeMode::Standard => "Standard",
            ChargeMode::Fast => "Rapid charge",
            ChargeMode::LongLife => "Conservation",
        }
    }

    /// Parse a kernel token from `charge_types` (brackets already stripped).
    pub fn from_sysfs(s: &str) -> Result<Self> {
        let s = s.trim();
        ChargeMode::ALL
            .into_iter()
            .find(|m| m.as_sysfs().eq_ignore_ascii_case(s))
            .ok_or_else(|| HwError::Parse(format!("unknown charge type {s:?}")))
    }

    /// Parse a user-supplied name — accepts `long-life`, `long_life`,
    /// `longlife` and the kernel spelling.
    pub fn from_user(s: &str) -> Result<Self> {
        let norm = s.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        ChargeMode::ALL
            .into_iter()
            .find(|m| {
                m.key() == norm
                    || m.as_sysfs().to_ascii_lowercase().replace('_', "-") == norm
                    || m.key().replace('-', "") == norm
            })
            .ok_or_else(|| HwError::Parse(format!("unknown charge mode {s:?}")))
    }
}

/// Parsed contents of `charge_types`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargeTypes {
    /// Every mode the firmware supports, in the order the kernel listed them.
    pub available: Vec<ChargeMode>,
    /// The bracketed (active) mode, if the kernel marked one.
    pub active: Option<ChargeMode>,
}

/// Parse the bracketed-active list format, e.g. `Fast Standard [Long_Life]`.
pub fn parse_charge_types(raw: &str) -> Result<ChargeTypes> {
    let mut available = Vec::new();
    let mut active = None;
    for token in raw.split_whitespace() {
        let (name, is_active) = match token.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            Some(inner) => (inner, true),
            None => (token, false),
        };
        let mode = ChargeMode::from_sysfs(name)?;
        if is_active {
            active = Some(mode);
        }
        available.push(mode);
    }
    if available.is_empty() {
        return Err(HwError::Parse("empty charge_types".into()));
    }
    Ok(ChargeTypes { available, active })
}

/// Is the `charge_types` interface present?
pub fn available(root: &SysRoot) -> bool {
    root.exists(&format!("{BAT}/charge_types"))
}

/// Read the supported and active charge modes.
pub fn charge_types(root: &SysRoot) -> Result<ChargeTypes> {
    parse_charge_types(&root.read(&format!("{BAT}/charge_types"))?)
}

/// The active charge mode.
pub fn get_mode(root: &SysRoot) -> Result<ChargeMode> {
    charge_types(root)?
        .active
        .ok_or_else(|| HwError::Parse("charge_types has no active entry".into()))
}

/// Select a charge mode.
pub fn set_mode(root: &SysRoot, mode: ChargeMode) -> Result<()> {
    root.write(&format!("{BAT}/charge_types"), mode.as_sysfs())
}

/// Battery readings. Every field is optional: attributes absent on a given
/// machine are simply omitted rather than failing the whole read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BatteryStats {
    /// Charge percentage as the kernel reports it.
    pub capacity: Option<u32>,
    /// Percentage computed from energy_now/energy_full — finer-grained than
    /// `capacity`, and a sanity check on it.
    pub capacity_from_energy: Option<f64>,
    pub energy_now: Option<u64>,
    pub energy_full: Option<u64>,
    pub energy_full_design: Option<u64>,
    /// Remaining capacity as a fraction of the design capacity — battery wear.
    pub health_pct: Option<f64>,
    pub voltage_now: Option<u64>,
    pub power_now: Option<u64>,
    pub cycle_count: Option<u32>,
    pub status: Option<String>,
    /// True when an AC adapter reports `online`.
    pub ac_online: Option<bool>,
}

/// Read every battery attribute that exists, skipping the ones that do not.
pub fn stats(root: &SysRoot) -> BatteryStats {
    let g = |name: &str| root.read_parsed_opt::<u64>(&format!("{BAT}/{name}"));

    let energy_now = g("energy_now");
    let energy_full = g("energy_full");
    let energy_full_design = g("energy_full_design");

    let capacity_from_energy = match (energy_now, energy_full) {
        (Some(now), Some(full)) if full > 0 => Some(now as f64 / full as f64 * 100.0),
        _ => None,
    };
    let health_pct = match (energy_full, energy_full_design) {
        (Some(full), Some(design)) if design > 0 => Some(full as f64 / design as f64 * 100.0),
        _ => None,
    };

    BatteryStats {
        capacity: root.read_parsed_opt(&format!("{BAT}/capacity")),
        capacity_from_energy,
        energy_now,
        energy_full,
        energy_full_design,
        health_pct,
        voltage_now: g("voltage_now"),
        power_now: g("power_now"),
        cycle_count: root.read_parsed_opt(&format!("{BAT}/cycle_count")),
        status: root.read_opt(&format!("{BAT}/status")),
        ac_online: ac_online(root),
    }
}

/// Is an AC adapter plugged in?
///
/// Scans `power_supply` for a mains-type supply (`ADP*`/`AC*`) rather than
/// hardcoding `ADP0`, since the name varies between machines.
pub fn ac_online(root: &SysRoot) -> Option<bool> {
    for entry in root.list_dir(SUPPLIES) {
        let is_mains = root
            .read_opt(&format!("{SUPPLIES}/{entry}/type"))
            .is_some_and(|t| t.eq_ignore_ascii_case("Mains"))
            || entry.starts_with("ADP")
            || entry.starts_with("AC");
        if is_mains
            && let Some(v) = root.read_parsed_opt::<u32>(&format!("{SUPPLIES}/{entry}/online"))
        {
            return Some(v != 0);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_82wm_value() {
        let ct = parse_charge_types("Fast Standard [Long_Life]").unwrap();
        assert_eq!(ct.active, Some(ChargeMode::LongLife));
        assert_eq!(ct.available.len(), 3);
        assert!(ct.available.contains(&ChargeMode::Fast));
        assert!(ct.available.contains(&ChargeMode::Standard));
    }

    #[test]
    fn parses_single_bracketed_entry() {
        let ct = parse_charge_types("[Standard]").unwrap();
        assert_eq!(ct.active, Some(ChargeMode::Standard));
        assert_eq!(ct.available, vec![ChargeMode::Standard]);
    }

    #[test]
    fn parses_list_without_active_marker() {
        let ct = parse_charge_types("Fast Standard").unwrap();
        assert_eq!(ct.active, None);
        assert_eq!(ct.available.len(), 2);
    }

    #[test]
    fn rejects_unknown_and_empty() {
        assert!(parse_charge_types("Turbo").is_err());
        assert!(parse_charge_types("   ").is_err());
    }

    #[test]
    fn write_format_uses_underscore() {
        assert_eq!(ChargeMode::LongLife.as_sysfs(), "Long_Life");
        assert_eq!(ChargeMode::LongLife.key(), "long-life");
    }

    #[test]
    fn user_input_is_forgiving() {
        for s in ["long-life", "Long_Life", "longlife", "LONG-LIFE"] {
            assert_eq!(ChargeMode::from_user(s).unwrap(), ChargeMode::LongLife);
        }
        assert!(ChargeMode::from_user("conservation").is_err());
    }
}
