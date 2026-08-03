use std::io;

/// Every failure mode the backend can produce.
#[derive(Debug, thiserror::Error)]
pub enum HwError {
    /// The kernel does not expose this feature on this machine (path absent).
    #[error("not supported on this machine")]
    NotSupported,

    /// The caller lacks the rights to write this sysfs file (i.e. is not root).
    #[error("permission denied (only the daemon, running as root, may write here)")]
    PermissionDenied,

    /// The kernel refused the write because a precondition is unmet — for the
    /// `ppt_*` firmware attributes this means the platform profile is not
    /// `custom`.
    #[error("device busy: switch the power profile to Custom first")]
    Busy,

    #[error("i/o error: {0}")]
    Io(#[from] io::Error),

    #[error("could not parse sysfs value: {0}")]
    Parse(String),
}

impl HwError {
    /// Classify a raw [`io::Error`] coming out of a sysfs read/write.
    pub fn from_io(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => HwError::NotSupported,
            io::ErrorKind::PermissionDenied => HwError::PermissionDenied,
            _ if e.raw_os_error() == Some(rustix::io::Errno::BUSY.raw_os_error()) => HwError::Busy,
            // ENODEV/EOPNOTSUPP show up when a driver advertises an attribute
            // its firmware does not actually implement.
            _ if matches!(
                e.raw_os_error(),
                Some(x) if x == rustix::io::Errno::NODEV.raw_os_error()
                    || x == rustix::io::Errno::OPNOTSUPP.raw_os_error()
            ) =>
            {
                HwError::NotSupported
            }
            _ => HwError::Io(e),
        }
    }

    /// Stable machine-readable tag, used by the daemon to pick a D-Bus error
    /// name and by the CLI to pick an exit code.
    pub fn kind_str(&self) -> &'static str {
        match self {
            HwError::NotSupported => "not-supported",
            HwError::PermissionDenied => "permission-denied",
            HwError::Busy => "custom-mode-required",
            HwError::Io(_) => "io",
            HwError::Parse(_) => "parse",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ebusy_maps_to_busy() {
        let e = io::Error::from_raw_os_error(rustix::io::Errno::BUSY.raw_os_error());
        assert!(matches!(HwError::from_io(e), HwError::Busy));
    }

    #[test]
    fn enoent_maps_to_not_supported() {
        let e = io::Error::from_raw_os_error(rustix::io::Errno::NOENT.raw_os_error());
        assert!(matches!(HwError::from_io(e), HwError::NotSupported));
    }

    #[test]
    fn eacces_maps_to_permission_denied() {
        let e = io::Error::from_raw_os_error(rustix::io::Errno::ACCESS.raw_os_error());
        assert!(matches!(HwError::from_io(e), HwError::PermissionDenied));
    }

    #[test]
    fn kind_strings_are_stable() {
        assert_eq!(HwError::Busy.kind_str(), "custom-mode-required");
        assert_eq!(HwError::NotSupported.kind_str(), "not-supported");
    }
}
