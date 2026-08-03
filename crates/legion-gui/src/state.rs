//! Pure mapping between daemon data and what the UI displays.
//!
//! Everything here is a plain function over plain data — no Slint types, no
//! D-Bus — so it is unit-testable without a display or a bus. `main.rs` is
//! then a thin layer that pushes these results into Slint properties.

use legion_hw::Capabilities;
use legion_hw::battery::{BatteryStats, ChargeMode};
use legion_hw::firmware_attrs::AttrInfo;
use legion_hw::profile::PowerProfile;

/// Slider bounds and value for one power limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SliderBounds {
    pub value: i32,
    pub min: i32,
    pub max: i32,
}

impl SliderBounds {
    /// Derive the slider from a firmware attribute, clamping a nonsensical
    /// firmware range rather than letting the UI divide by zero.
    pub fn from_attr(a: &AttrInfo) -> Self {
        let min = a.min as i32;
        let max = if a.max > a.min {
            a.max as i32
        } else {
            min.saturating_add(1)
        };
        Self {
            value: (a.current as i32).clamp(min, max),
            min,
            max,
        }
    }

    /// Look up one attribute by short key in the daemon's `GetPpt` list.
    pub fn find(attrs: &[AttrInfo], key: &str) -> Self {
        attrs
            .iter()
            .find(|a| a.key == key)
            .map(Self::from_attr)
            .unwrap_or_default()
    }
}

/// The colour of the power-button LED for a profile, as `#rrggbb`.
pub fn profile_color(profile: &str) -> String {
    PowerProfile::from_sysfs(profile)
        .map(|p| p.led_color().hex().to_string())
        .unwrap_or_else(|_| "#99a0b0".to_string())
}

/// The same colour as RGB components, for Slint's `brush` properties.
pub fn profile_rgb(profile: &str) -> (u8, u8, u8) {
    parse_hex(&profile_color(profile)).unwrap_or((0x99, 0xa0, 0xb0))
}

/// Parse `#rrggbb`. Returns `None` for anything else rather than panicking on
/// a malformed colour constant.
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let s = hex.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ))
}

/// Human label for a kernel profile token.
pub fn profile_label(profile: &str) -> String {
    PowerProfile::from_sysfs(profile)
        .map(|p| p.label().to_string())
        .unwrap_or_else(|_| profile.to_string())
}

/// Labels for a whole choice list, preserving order.
pub fn profile_labels(choices: &[String]) -> Vec<String> {
    choices.iter().map(|c| profile_label(c)).collect()
}

/// The tray icon name for a profile — one pre-rendered PNG per LED colour.
pub fn tray_icon_name(profile: &str) -> String {
    match PowerProfile::from_sysfs(profile) {
        Ok(PowerProfile::Quiet) => "legion-toolkit-quiet".into(),
        Ok(PowerProfile::Balanced) => "legion-toolkit-balanced".into(),
        Ok(PowerProfile::Performance) => "legion-toolkit-performance".into(),
        Ok(PowerProfile::Extreme) => "legion-toolkit-extreme".into(),
        Ok(PowerProfile::Custom) => "legion-toolkit-custom".into(),
        Err(_) => "legion-toolkit".into(),
    }
}

/// Label for a kernel charge-type token.
pub fn battery_label(mode: &str) -> String {
    ChargeMode::from_sysfs(mode)
        .map(|m| m.label().to_string())
        .unwrap_or_else(|_| mode.to_string())
}

/// One-line explanation of what a charge mode does.
pub fn battery_hint(mode: &str) -> String {
    match ChargeMode::from_sysfs(mode) {
        Ok(ChargeMode::Standard) => "Charge to 100%".into(),
        Ok(ChargeMode::Fast) => "Charge faster, warmer battery".into(),
        Ok(ChargeMode::LongLife) => "Stop around 60–80% to extend battery life".into(),
        Err(_) => String::new(),
    }
}

/// Labels and hints for a charge-type choice list.
pub fn battery_labels(choices: &[String]) -> (Vec<String>, Vec<String>) {
    (
        choices.iter().map(|c| battery_label(c)).collect(),
        choices.iter().map(|c| battery_hint(c)).collect(),
    )
}

/// Format a temperature for display.
pub fn format_temp(celsius: Option<f64>) -> String {
    match celsius {
        Some(t) => format!("{t:.1} °C"),
        None => "—".into(),
    }
}

/// Format a CPU frequency given in kHz.
pub fn format_freq(khz: Option<u64>) -> String {
    match khz {
        Some(k) => format!("{:.2} GHz", k as f64 / 1_000_000.0),
        None => "—".into(),
    }
}

/// Format fan speed.
///
/// `None` is the *expected* state on this machine — kernel 7.0 exposes no fan
/// hwmon for it, and upstream's `lenovo-wmi-other` fan sensors are newer than
/// that. So the text names the reason rather than showing a dash or "0 RPM",
/// both of which read as a fault. It resolves itself on a newer kernel:
/// `fan::readings()` scans rather than hardcoding.
pub fn format_fan(rpm: Option<u32>) -> String {
    match rpm {
        Some(r) => format!("{r} RPM"),
        None => "No fan sensor (kernel too old)".into(),
    }
}

/// Format a microwatt/microvolt power_supply reading as W or V.
pub fn format_micro(value: Option<u64>, unit: &str) -> String {
    match value {
        Some(v) => format!("{:.1} {unit}", v as f64 / 1_000_000.0),
        None => "—".into(),
    }
}

/// Format a percentage.
pub fn format_pct(pct: Option<f64>) -> String {
    match pct {
        Some(p) => format!("{p:.0}%"),
        None => "—".into(),
    }
}

/// The battery summary shown on Home and Battery.
pub fn battery_capacity_text(stats: &BatteryStats) -> String {
    match stats.capacity {
        Some(c) => format!("{c}%"),
        None => format_pct(stats.capacity_from_energy),
    }
}

/// "Plugged in" / "On battery".
pub fn ac_text(stats: &BatteryStats) -> String {
    match stats.ac_online {
        Some(true) => "Plugged in".into(),
        Some(false) => "On battery".into(),
        None => "—".into(),
    }
}

/// A note explaining the cooling section, tailored to what the kernel exposes.
pub fn kernel_note(caps: &Capabilities) -> String {
    if caps.fan_rpm {
        "Fan speed comes from the kernel's hwmon sensors.".into()
    } else {
        "This kernel exposes no fan sensor for this model, so fan speed cannot \
         be shown. Fan curves have no upstream interface and are not offered."
            .into()
    }
}

/// Blank fields read as broken; an em dash reads as "nothing here".
pub fn or_dash(s: Option<&str>) -> String {
    match s {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(key: &str, current: u32, min: u32, max: u32) -> AttrInfo {
        AttrInfo {
            key: key.into(),
            display_name: key.into(),
            current,
            default: current,
            min,
            max,
            step: 1,
        }
    }

    #[test]
    fn slider_bounds_come_from_the_firmware() {
        let b = SliderBounds::from_attr(&attr("spl", 70, 50, 115));
        assert_eq!(
            b,
            SliderBounds {
                value: 70,
                min: 50,
                max: 115
            }
        );
    }

    #[test]
    fn slider_clamps_a_value_outside_the_reported_range() {
        let b = SliderBounds::from_attr(&attr("spl", 200, 50, 115));
        assert_eq!(b.value, 115);
    }

    #[test]
    fn slider_survives_a_degenerate_range() {
        // A firmware reporting min == max must not make the UI divide by zero.
        let b = SliderBounds::from_attr(&attr("spl", 50, 50, 50));
        assert!(b.max > b.min);
    }

    #[test]
    fn slider_lookup_by_key() {
        let attrs = vec![attr("spl", 70, 50, 115), attr("fppt", 102, 70, 150)];
        assert_eq!(SliderBounds::find(&attrs, "fppt").value, 102);
        // An attribute this machine lacks yields a zeroed slider, not a panic.
        assert_eq!(SliderBounds::find(&attrs, "sppt"), SliderBounds::default());
    }

    #[test]
    fn profile_colors_follow_the_led_mapping() {
        assert_eq!(profile_color("low-power"), "#3b82f6");
        assert_eq!(profile_color("balanced"), "#e5e7eb");
        assert_eq!(profile_color("performance"), "#ef4444");
        assert_eq!(profile_color("max-power"), "#a855f7");
        assert_eq!(profile_color("custom"), "#a855f7");
    }

    #[test]
    fn unknown_profile_gets_a_neutral_color_not_a_panic() {
        assert_eq!(profile_color("quantum"), "#99a0b0");
        assert_eq!(profile_rgb("quantum"), (0x99, 0xa0, 0xb0));
        assert_eq!(profile_label("quantum"), "quantum");
    }

    #[test]
    fn profile_rgb_decodes_the_hex_constants() {
        assert_eq!(profile_rgb("performance"), (0xef, 0x44, 0x44));
        assert_eq!(profile_rgb("low-power"), (0x3b, 0x82, 0xf6));
    }

    #[test]
    fn hex_parsing_rejects_malformed_input() {
        assert_eq!(parse_hex("#abc"), None);
        assert_eq!(parse_hex("ef4444"), None);
        assert_eq!(parse_hex("#gggggg"), None);
        assert_eq!(parse_hex("#ef4444"), Some((0xef, 0x44, 0x44)));
    }

    #[test]
    fn profile_labels_are_friendly() {
        let choices = vec![
            "low-power".to_string(),
            "max-power".to_string(),
            "custom".to_string(),
        ];
        assert_eq!(profile_labels(&choices), vec!["Quiet", "Extreme", "Custom"]);
    }

    #[test]
    fn tray_icon_names_are_distinct_per_profile() {
        let names: Vec<String> = [
            "low-power",
            "balanced",
            "performance",
            "max-power",
            "custom",
        ]
        .iter()
        .map(|p| tray_icon_name(p))
        .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "icon names must be unique");
        assert_eq!(tray_icon_name("nonsense"), "legion-toolkit");
    }

    #[test]
    fn battery_labels_and_hints_line_up() {
        let choices = vec![
            "Fast".to_string(),
            "Standard".to_string(),
            "Long_Life".to_string(),
        ];
        let (labels, hints) = battery_labels(&choices);
        assert_eq!(labels, vec!["Rapid charge", "Standard", "Conservation"]);
        assert_eq!(hints.len(), 3);
        assert!(hints[2].contains("60"));
    }

    #[test]
    fn formatters_render_absent_values_as_a_dash() {
        assert_eq!(format_temp(None), "—");
        assert_eq!(format_freq(None), "—");
        assert_eq!(format_micro(None, "W"), "—");
        assert_eq!(format_pct(None), "—");
        assert_eq!(or_dash(None), "—");
        assert_eq!(or_dash(Some("")), "—");
        assert_eq!(or_dash(Some("82WM")), "82WM");
    }

    #[test]
    fn formatters_render_present_values() {
        assert_eq!(format_temp(Some(52.125)), "52.1 °C");
        assert_eq!(format_freq(Some(3_593_000)), "3.59 GHz");
        assert_eq!(format_micro(Some(12_000_000), "W"), "12.0 W");
        assert_eq!(format_pct(Some(77.5)), "78%");
    }

    #[test]
    fn absent_fan_explains_itself_rather_than_showing_zero() {
        // "0 RPM" would imply the fan is stopped and a bare dash reads as a
        // fault; on this machine the sensor simply does not exist yet.
        let text = format_fan(None);
        assert_eq!(text, "No fan sensor (kernel too old)");
        assert!(!text.contains("0 RPM"));
        assert_ne!(text, "—");

        assert_eq!(format_fan(Some(2400)), "2400 RPM");
    }

    #[test]
    fn capacity_falls_back_to_the_energy_ratio() {
        let stats = BatteryStats {
            capacity: None,
            capacity_from_energy: Some(77.5),
            ..Default::default()
        };
        assert_eq!(battery_capacity_text(&stats), "78%");

        let stats = BatteryStats {
            capacity: Some(80),
            capacity_from_energy: Some(77.5),
            ..Default::default()
        };
        assert_eq!(battery_capacity_text(&stats), "80%");
    }

    #[test]
    fn ac_state_is_worded_for_humans() {
        let mut stats = BatteryStats::default();
        assert_eq!(ac_text(&stats), "—");
        stats.ac_online = Some(true);
        assert_eq!(ac_text(&stats), "Plugged in");
        stats.ac_online = Some(false);
        assert_eq!(ac_text(&stats), "On battery");
    }

    #[test]
    fn kernel_note_explains_the_missing_fan_sensor() {
        let caps = Capabilities::default();
        assert!(kernel_note(&caps).contains("no fan sensor"));

        let caps = Capabilities {
            fan_rpm: true,
            ..Default::default()
        };
        assert!(kernel_note(&caps).contains("hwmon"));
    }
}
