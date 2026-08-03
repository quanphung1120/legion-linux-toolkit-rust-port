//! Blocking watcher for Fn+Q (and any other firmware-side) profile changes.
//!
//! `Documentation/ABI/testing/sysfs-class-platform-profile` specifies
//! `poll()`/`POLLPRI` on the `profile` file as *the* notification mechanism —
//! there is no uevent or evdev event for this on the 82WM. The sequence is:
//! open, read once to arm the notification, then `poll` for `POLLPRI`, then
//! re-read.
//!
//! Only the daemon runs this; every other process learns about changes from
//! the daemon's `PropertiesChanged` signal.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::profile::{self, PROFILE_FILE, PowerProfile};
use crate::{HwError, Result, SysRoot};

/// A file handle armed for `POLLPRI` notifications on the profile attribute.
pub struct ProfileWatcher {
    file: File,
}

/// Outcome of one [`ProfileWatcher::wait_for_change`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEvent {
    /// The firmware changed the profile; here is the new value.
    Changed(PowerProfile),
    /// Nothing happened before the timeout elapsed.
    Timeout,
}

impl ProfileWatcher {
    /// Open the profile attribute and arm it.
    pub fn new(root: &SysRoot) -> Result<Self> {
        let file = File::open(root.path(PROFILE_FILE)).map_err(HwError::from_io)?;
        let mut watcher = Self { file };
        // The initial read is what arms POLLPRI; without it poll() returns
        // immediately, forever.
        let _ = watcher.reread()?;
        Ok(watcher)
    }

    /// Rewind and read the current value.
    fn reread(&mut self) -> Result<PowerProfile> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(HwError::from_io)?;
        let mut buf = String::new();
        self.file
            .read_to_string(&mut buf)
            .map_err(HwError::from_io)?;
        PowerProfile::from_sysfs(buf.trim())
    }

    /// Block until the firmware changes the profile, or `timeout` elapses.
    pub fn wait_for_change(&mut self, timeout: Duration) -> Result<WatchEvent> {
        let mut fds = [PollFd::new(&self.file, PollFlags::PRI | PollFlags::ERR)];
        let ts = Timespec {
            tv_sec: timeout.as_secs() as _,
            tv_nsec: timeout.subsec_nanos() as _,
        };
        let ready = poll(&mut fds, Some(&ts))
            .map_err(|e| HwError::Io(std::io::Error::from_raw_os_error(e.raw_os_error())))?;
        if ready == 0 {
            return Ok(WatchEvent::Timeout);
        }
        Ok(WatchEvent::Changed(self.reread()?))
    }
}

/// Read the current profile without holding a watcher open.
pub fn current(root: &SysRoot) -> Result<PowerProfile> {
    profile::get(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fake_profile_file(tmp: &TempDir, value: &str) -> SysRoot {
        let root = SysRoot::at(tmp.path());
        let dir = root.path("sys/class/platform-profile/platform-profile-0");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("profile"), format!("{value}\n")).unwrap();
        root
    }

    #[test]
    fn watcher_opens_and_reads_initial_value() {
        let tmp = TempDir::new().unwrap();
        let root = fake_profile_file(&tmp, "balanced");
        let mut w = ProfileWatcher::new(&root).unwrap();
        assert_eq!(w.reread().unwrap(), PowerProfile::Balanced);
    }

    #[test]
    fn watcher_on_missing_file_is_not_supported() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        assert!(matches!(
            ProfileWatcher::new(&root),
            Err(HwError::NotSupported)
        ));
    }

    #[test]
    fn regular_file_poll_times_out_or_reports() {
        // A tmpfs file is always "ready" for some poll flags, so this test only
        // asserts the call returns rather than hangs or errors.
        let tmp = TempDir::new().unwrap();
        let root = fake_profile_file(&tmp, "performance");
        let mut w = ProfileWatcher::new(&root).unwrap();
        let r = w.wait_for_change(Duration::from_millis(20)).unwrap();
        match r {
            WatchEvent::Timeout => {}
            WatchEvent::Changed(p) => assert_eq!(p, PowerProfile::Performance),
        }
    }
}
