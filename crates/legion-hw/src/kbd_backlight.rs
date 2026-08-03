//! Keyboard backlight over `/sys/class/leds/platform::kbd_backlight/`.
//!
//! The 82WM ships the **white-only** keyboard (USB `048d:c103`), which has
//! three brightness levels: 0 (off), 1 and 2 (`max_brightness` = 2). There is
//! no per-zone RGB on this machine, so this module deliberately exposes
//! nothing but a level.
//!
//! `brightness_hw_changed` is bumped by the firmware when the user presses
//! Fn+Space; it is exposed here so a caller can poll it if desired.

use crate::{HwError, Result, SysRoot};

const LED: &str = "sys/class/leds/platform::kbd_backlight";

/// Is a keyboard backlight present?
pub fn available(root: &SysRoot) -> bool {
    root.exists(&format!("{LED}/brightness"))
}

/// Highest level the LED accepts (2 on the 82WM).
pub fn max(root: &SysRoot) -> Result<u32> {
    root.read_parsed(&format!("{LED}/max_brightness"))
}

/// Current level.
pub fn get(root: &SysRoot) -> Result<u32> {
    root.read_parsed(&format!("{LED}/brightness"))
}

/// Set the level, rejecting anything above `max_brightness`.
pub fn set(root: &SysRoot, level: u32) -> Result<()> {
    let maximum = max(root)?;
    if level > maximum {
        return Err(HwError::Parse(format!(
            "keyboard backlight level {level} is out of range: 0–{maximum}"
        )));
    }
    root.write(&format!("{LED}/brightness"), &level.to_string())
}

/// The firmware's hardware-change counter, bumped on Fn+Space.
pub fn hw_changed(root: &SysRoot) -> Option<u32> {
    root.read_parsed_opt(&format!("{LED}/brightness_hw_changed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fake(tmp: &TempDir) -> SysRoot {
        let root = SysRoot::at(tmp.path());
        let dir = root.path(LED);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("brightness"), "1\n").unwrap();
        fs::write(dir.join("max_brightness"), "2\n").unwrap();
        root
    }

    #[test]
    fn reads_level_and_max() {
        let tmp = TempDir::new().unwrap();
        let root = fake(&tmp);
        assert_eq!(get(&root).unwrap(), 1);
        assert_eq!(max(&root).unwrap(), 2);
        assert!(available(&root));
    }

    #[test]
    fn set_above_max_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let root = fake(&tmp);
        let err = set(&root, 3).unwrap_err();
        assert!(matches!(err, HwError::Parse(_)));
        // The rejection is client-side: the file is untouched.
        assert_eq!(get(&root).unwrap(), 1);
    }

    #[test]
    fn set_within_range_writes() {
        let tmp = TempDir::new().unwrap();
        let root = fake(&tmp);
        set(&root, 2).unwrap();
        assert_eq!(get(&root).unwrap(), 2);
        set(&root, 0).unwrap();
        assert_eq!(get(&root).unwrap(), 0);
    }

    #[test]
    fn absent_backlight_is_not_supported() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        assert!(!available(&root));
        assert!(matches!(get(&root), Err(HwError::NotSupported)));
    }
}
