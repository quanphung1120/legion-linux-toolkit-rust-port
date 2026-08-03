//! One connection policy, used by every subcommand.
//!
//! The CLI prefers the daemon: it is the process with the rights to write, and
//! going through it keeps the GUI and tray in sync. When `legiond` is not on
//! the bus the CLI falls back to talking to sysfs directly — reads always
//! work, writes work when run as root — and says so once, on stderr, so a
//! script's stdout stays clean.

use std::path::PathBuf;

use legion_hw::{HwError, SysRoot};
use zbus::Connection;

pub const BUS_NAME: &str = "org.legiontoolkit.Daemon";
pub const OBJECT_PATH: &str = "/org/legiontoolkit/Daemon";
pub const IFACE: &str = "org.legiontoolkit.Daemon1";

/// Printed once when falling back, so the user knows why a write may fail.
pub const FALLBACK_HINT: &str =
    "legiond not running — using direct sysfs (systemctl enable --now legiond)";

/// Which bus to look for the daemon on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BusKind {
    System,
    Session,
}

/// How this invocation talks to the hardware.
pub enum Backend {
    /// Through the daemon.
    Daemon {
        proxy: Box<zbus::Proxy<'static>>,
        root: SysRoot,
    },
    /// Straight to sysfs.
    Direct { root: SysRoot },
}

impl Backend {
    /// Try the daemon; fall back to sysfs, printing the hint once.
    pub async fn connect(bus: BusKind, sysfs_root: Option<PathBuf>) -> Self {
        let root = match sysfs_root {
            Some(p) => SysRoot::at(p),
            None => SysRoot::system(),
        };

        match Self::try_daemon(bus).await {
            Some(proxy) => Backend::Daemon {
                proxy: Box::new(proxy),
                root,
            },
            None => {
                eprintln!("{FALLBACK_HINT}");
                Backend::Direct { root }
            }
        }
    }

    async fn try_daemon(bus: BusKind) -> Option<zbus::Proxy<'static>> {
        let conn = match bus {
            BusKind::System => Connection::system().await.ok()?,
            BusKind::Session => Connection::session().await.ok()?,
        };

        // Only use the daemon if the name is actually owned; building a proxy
        // for an absent service would otherwise fail later, mid-command.
        let dbus = zbus::fdo::DBusProxy::new(&conn).await.ok()?;
        if !dbus
            .name_has_owner(BUS_NAME.try_into().ok()?)
            .await
            .unwrap_or(false)
        {
            return None;
        }

        zbus::Proxy::new(&conn, BUS_NAME, OBJECT_PATH, IFACE)
            .await
            .ok()
    }

    /// The sysfs root, for the read-only paths that never need the daemon.
    pub fn root(&self) -> &SysRoot {
        match self {
            Backend::Daemon { root, .. } => root,
            Backend::Direct { root } => root,
        }
    }

    /// The daemon proxy, when there is one.
    pub fn proxy(&self) -> Option<&zbus::Proxy<'static>> {
        match self {
            Backend::Daemon { proxy, .. } => Some(proxy),
            Backend::Direct { .. } => None,
        }
    }
}

/// Turn any failure into the message the user sees, with the actionable cases
/// spelled out rather than surfaced as raw D-Bus error names.
pub fn describe_error(e: &zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, detail, _) => {
            let name = name.as_str();
            let detail = detail.clone().unwrap_or_default();
            match name {
                "org.legiontoolkit.Error.CustomModeRequired" => {
                    "switch to the Custom profile first: legion-ctl profile custom".to_string()
                }
                "org.legiontoolkit.Error.NotAuthorized" => {
                    format!("not authorized: {detail}")
                }
                "org.legiontoolkit.Error.NotSupported" => {
                    format!("not supported on this machine: {detail}")
                }
                _ => {
                    if detail.is_empty() {
                        name.to_string()
                    } else {
                        detail
                    }
                }
            }
        }
        other => other.to_string(),
    }
}

/// Same, for direct-sysfs failures.
pub fn describe_hw_error(e: &HwError) -> String {
    match e {
        HwError::PermissionDenied => {
            "permission denied — start the daemon (systemctl enable --now legiond) or run as root"
                .to_string()
        }
        HwError::Busy => {
            "switch to the Custom profile first: legion-ctl profile custom".to_string()
        }
        HwError::NotSupported => "not supported on this machine".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_mode_error_becomes_actionable_advice() {
        let e = zbus::Error::MethodError(
            "org.legiontoolkit.Error.CustomModeRequired"
                .try_into()
                .unwrap(),
            Some("nope".into()),
            zbus::message::Message::method_call("/x", "Y")
                .unwrap()
                .build(&())
                .unwrap(),
        );
        assert!(describe_error(&e).contains("profile custom"));
    }

    #[test]
    fn permission_denied_names_the_fix() {
        let msg = describe_hw_error(&HwError::PermissionDenied);
        assert!(msg.contains("legiond") || msg.contains("root"), "{msg}");
    }

    #[test]
    fn busy_maps_to_the_same_advice_in_both_paths() {
        assert!(describe_hw_error(&HwError::Busy).contains("Custom"));
    }
}
