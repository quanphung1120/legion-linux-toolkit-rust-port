//! D-Bus error mapping.
//!
//! The interesting case is [`HwError::Busy`], which the kernel returns for a
//! PPT write outside Custom mode. It gets its own error name,
//! `org.legiontoolkit.Error.CustomModeRequired`, so a client can show
//! "Switch to Custom mode first" rather than a generic failure.

use legion_hw::HwError;

/// Errors this daemon returns over D-Bus.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.legiontoolkit.Error")]
pub enum DaemonError {
    /// Pass-through for zbus's own failures.
    #[zbus(error)]
    ZBus(zbus::Error),

    /// A power-limit write was attempted while the profile is not `custom`.
    CustomModeRequired(String),

    /// This machine's kernel does not expose the requested interface.
    NotSupported(String),

    /// Polkit denied the caller.
    NotAuthorized(String),

    /// The daemon could not write the value (it runs as root, so this is
    /// unusual — a read-only sysfs or a driver refusing the value).
    PermissionDenied(String),

    /// Bad argument from the client.
    InvalidArgument(String),

    /// Anything else.
    Failed(String),
}

impl From<HwError> for DaemonError {
    fn from(e: HwError) -> Self {
        let msg = e.to_string();
        match e {
            HwError::Busy => DaemonError::CustomModeRequired(
                "power limits can only be changed while the Custom profile is active".into(),
            ),
            HwError::NotSupported => DaemonError::NotSupported(msg),
            HwError::PermissionDenied => DaemonError::PermissionDenied(msg),
            HwError::Parse(_) => DaemonError::InvalidArgument(msg),
            HwError::Io(_) => DaemonError::Failed(msg),
        }
    }
}

/// Result type for interface methods.
pub type Result<T> = std::result::Result<T, DaemonError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_becomes_custom_mode_required() {
        assert!(matches!(
            DaemonError::from(HwError::Busy),
            DaemonError::CustomModeRequired(_)
        ));
    }

    #[test]
    fn parse_becomes_invalid_argument() {
        assert!(matches!(
            DaemonError::from(HwError::Parse("bad".into())),
            DaemonError::InvalidArgument(_)
        ));
    }

    #[test]
    fn not_supported_is_preserved() {
        assert!(matches!(
            DaemonError::from(HwError::NotSupported),
            DaemonError::NotSupported(_)
        ));
    }
}
