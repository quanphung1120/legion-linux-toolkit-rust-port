//! Polkit authorization for every mutating method.
//!
//! The action is `org.legiontoolkit.control`, whose policy defaults are
//! `allow_active=yes` / `allow_inactive=auth_admin` / `allow_any=no`: the
//! person sitting at the laptop controls their own hardware with no prompt,
//! while an SSH or otherwise inactive session must authenticate. Read-only
//! properties are not checked at all.

use zbus::message::Header;
use zbus_polkit::policykit1::{AuthorityProxy, CheckAuthorizationFlags, Subject};

use crate::error::{DaemonError, Result};

/// The polkit action gating every `Set*` method.
pub const ACTION_ID: &str = "org.legiontoolkit.control";

/// How a given daemon instance authorizes callers.
pub enum Authorizer {
    /// Ask polkit (the system-bus production path).
    Polkit(Box<AuthorityProxy<'static>>),
    /// Allow everything — used only when the daemon runs on a private session
    /// bus for tests, where there is no polkit and no privileged sysfs.
    AllowAll,
}

impl Authorizer {
    /// Connect to polkit on the system bus.
    pub async fn polkit(connection: &zbus::Connection) -> zbus::Result<Self> {
        let proxy = AuthorityProxy::new(connection).await?;
        Ok(Authorizer::Polkit(Box::new(proxy)))
    }

    /// Authorize the sender of `hdr`, or fail with `NotAuthorized`.
    pub async fn check(&self, hdr: &Header<'_>) -> Result<()> {
        let proxy = match self {
            Authorizer::AllowAll => return Ok(()),
            Authorizer::Polkit(p) => p,
        };

        let subject = Subject::new_for_message_header(hdr)
            .map_err(|e| DaemonError::Failed(format!("identifying caller: {e}")))?;

        let result = proxy
            .check_authorization(
                &subject,
                ACTION_ID,
                &std::collections::HashMap::new(),
                // AllowUserInteraction lets a session with a polkit agent
                // satisfy allow_inactive=auth_admin; the active desktop user
                // never sees a prompt (allow_active=yes), and agent-less
                // sessions (plain SSH) get a clean denial.
                CheckAuthorizationFlags::AllowUserInteraction.into(),
                "",
            )
            .await
            .map_err(|e| DaemonError::Failed(format!("polkit check failed: {e}")))?;

        if result.is_authorized {
            Ok(())
        } else {
            Err(DaemonError::NotAuthorized(
                "not authorized to change Legion hardware settings \
                 (active local session required)"
                    .into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_id_matches_the_polkit_policy_file() {
        assert_eq!(ACTION_ID, "org.legiontoolkit.control");
    }
}
