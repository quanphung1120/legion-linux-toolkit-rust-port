//! Persisted daemon state.
//!
//! Firmware forgets the battery charge mode and the PPT power limits across
//! reboots and suspend/resume, so the daemon records what the user last asked
//! for and reapplies it on start and on resume. The profile is recorded too —
//! reapplying it is cheap and harmless, and it keeps the three settings
//! consistent.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the daemon persists state when running as a system service.
/// `StateDirectory=legiond` in the unit file creates and owns this.
pub const DEFAULT_STATE_PATH: &str = "/var/lib/legiond/state.json";

/// The settings worth restoring.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Kernel profile string, e.g. `balanced`.
    pub profile: Option<String>,
    /// Kernel charge type, e.g. `Long_Life`.
    pub battery_mode: Option<String>,
    /// Power limits in watts, keyed by short name (`spl`, `sppt`, `fppt`).
    pub ppt: BTreeMap<String, u32>,
}

impl State {
    /// Load state from disk. A missing or corrupt file yields the default —
    /// bad persisted state must never stop the daemon from starting.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                log::warn!("ignoring unreadable state file {}: {e}", path.display());
                State::default()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => State::default(),
            Err(e) => {
                log::warn!("could not read state file {}: {e}", path.display());
                State::default()
            }
        }
    }

    /// Write state to disk, creating the parent directory if needed.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        // Write via a temporary file so a crash mid-write cannot leave a
        // truncated state.json behind.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)
    }

    /// Record a power limit.
    pub fn set_ppt(&mut self, key: &str, watts: u32) {
        self.ppt.insert(key.to_string(), watts);
    }

    /// Is there anything worth reapplying?
    pub fn is_empty(&self) -> bool {
        self.profile.is_none() && self.battery_mode.is_none() && self.ppt.is_empty()
    }
}

/// Resolve the state-file path, honouring an explicit override.
pub fn state_path(override_path: Option<PathBuf>) -> PathBuf {
    override_path.unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_PATH))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trips_through_disk() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested/state.json");

        let mut s = State {
            profile: Some("custom".into()),
            battery_mode: Some("Long_Life".into()),
            ..Default::default()
        };
        s.set_ppt("spl", 90);
        s.set_ppt("sppt", 100);

        s.save(&path).unwrap();
        assert_eq!(State::load(&path), s);
    }

    #[test]
    fn missing_file_loads_as_default() {
        let tmp = TempDir::new().unwrap();
        let s = State::load(&tmp.path().join("absent.json"));
        assert_eq!(s, State::default());
        assert!(s.is_empty());
    }

    #[test]
    fn corrupt_file_does_not_stop_the_daemon() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("state.json");
        fs::write(&path, "{ this is not json").unwrap();
        assert_eq!(State::load(&path), State::default());
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        // Forward compatibility: a newer daemon's state file must not break
        // an older one.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("state.json");
        fs::write(&path, r#"{"profile":"balanced","future_field":42}"#).unwrap();
        assert_eq!(State::load(&path).profile.as_deref(), Some("balanced"));
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("state.json");
        State::default().save(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn state_path_honours_the_override() {
        assert_eq!(
            state_path(Some(PathBuf::from("/tmp/x.json"))),
            PathBuf::from("/tmp/x.json")
        );
        assert_eq!(state_path(None), PathBuf::from(DEFAULT_STATE_PATH));
    }
}
