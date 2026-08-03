//! The sync primitive behind Fn+Q detection.
//!
//! `Documentation/ABI/testing/sysfs-class-platform-profile` specifies
//! `poll()`/`POLLPRI` on the `profile` file as *the* notification mechanism:
//! open the file, read once to clear the pending state, then wait for
//! `POLLPRI` and re-read. sysfs attributes never generate inotify events, so
//! no filesystem-watching crate can substitute for this.
//!
//! This module deliberately stops at the file handle. It does no waiting of
//! its own — the daemon registers [`ProfileWatchFd`]'s descriptor on tokio's
//! reactor with `Interest::PRIORITY` (`EPOLLPRI`) and awaits it as an ordinary
//! async task, so there is no dedicated blocking thread and no channel bridge.
//!
//! Testing note: a regular file in a fake sysfs tree can never produce
//! `POLLPRI`, so only [`ProfileWatchFd::consume`]'s parse path is unit-tested;
//! the wakeup itself is verified on hardware.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};

use crate::profile::{self, PowerProfile};
use crate::{HwError, Result, SysRoot};

/// An open handle to the `profile` attribute, armed for `POLLPRI`.
///
/// Register it with an async reactor and call [`consume`](Self::consume) after
/// each wakeup.
#[derive(Debug)]
pub struct ProfileWatchFd {
    file: File,
}

impl ProfileWatchFd {
    /// Open the profile attribute and clear its pending state.
    ///
    /// The attribute is the one belonging to the device
    /// [`profile::device_dir`] discovers by name, so the watcher never ends up
    /// on a second handler's file (see [`crate::profile`]).
    ///
    /// The initial read is what arms `POLLPRI`; without it the descriptor
    /// reports ready immediately and forever.
    pub fn open(root: &SysRoot) -> Result<Self> {
        let rel = profile::profile_file(root).ok_or(HwError::NotSupported)?;
        let file = File::open(root.path(&rel)).map_err(HwError::from_io)?;
        let mut watcher = Self { file };
        let _ = watcher.consume()?;
        Ok(watcher)
    }

    /// Seek to the start and re-read the current profile.
    ///
    /// Call this once per wakeup: re-reading is also what re-arms the
    /// notification for the next change.
    pub fn consume(&mut self) -> Result<PowerProfile> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(HwError::from_io)?;
        let mut buf = String::new();
        self.file
            .read_to_string(&mut buf)
            .map_err(HwError::from_io)?;
        PowerProfile::from_sysfs(buf.trim())
    }

    /// The raw descriptor, for registering with an async reactor.
    pub fn raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl AsFd for ProfileWatchFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

// Required by `tokio::io::unix::AsyncFd`, which is how the daemon awaits
// EPOLLPRI on this descriptor.
impl AsRawFd for ProfileWatchFd {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
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
    fn open_performs_the_arming_read() {
        let tmp = TempDir::new().unwrap();
        let root = fake_profile_file(&tmp, "balanced");
        let mut w = ProfileWatchFd::open(&root).unwrap();
        assert_eq!(w.consume().unwrap(), PowerProfile::Balanced);
    }

    #[test]
    fn consume_rereads_after_the_value_changes() {
        let tmp = TempDir::new().unwrap();
        let root = fake_profile_file(&tmp, "balanced");
        let mut w = ProfileWatchFd::open(&root).unwrap();

        // Simulate the firmware changing the profile behind our back.
        fs::write(
            root.path("sys/class/platform-profile/platform-profile-0/profile"),
            "max-power\n",
        )
        .unwrap();

        assert_eq!(w.consume().unwrap(), PowerProfile::Extreme);
    }

    #[test]
    fn consume_reports_an_unparseable_value() {
        let tmp = TempDir::new().unwrap();
        let root = fake_profile_file(&tmp, "balanced");
        let mut w = ProfileWatchFd::open(&root).unwrap();

        fs::write(
            root.path("sys/class/platform-profile/platform-profile-0/profile"),
            "ludicrous\n",
        )
        .unwrap();

        assert!(matches!(w.consume(), Err(HwError::Parse(_))));
    }

    #[test]
    fn open_on_a_missing_file_is_not_supported() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        assert!(matches!(
            ProfileWatchFd::open(&root),
            Err(HwError::NotSupported)
        ));
    }

    #[test]
    fn exposes_a_usable_descriptor() {
        let tmp = TempDir::new().unwrap();
        let root = fake_profile_file(&tmp, "balanced");
        let w = ProfileWatchFd::open(&root).unwrap();
        assert!(w.raw_fd() >= 0);
        // AsFd is what lets the daemon hand this to tokio's AsyncFd.
        let _borrowed = w.as_fd();
    }
}
