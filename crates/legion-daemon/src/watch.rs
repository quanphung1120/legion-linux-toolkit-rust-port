//! Fn+Q propagation, as an async task.
//!
//! The firmware changes the platform profile behind the daemon's back when the
//! user presses Fn+Q. The kernel's only notification for that is `POLLPRI` on
//! the `profile` attribute, which epoll exposes as `EPOLLPRI` — so the file
//! descriptor is registered on tokio's reactor with `Interest::PRIORITY` and
//! awaited like any other async event source. No dedicated thread, no channel
//! bridge; the caller composes this future directly into its `select!` loop
//! alongside shutdown.
//!
//! (inotify is not an option here: sysfs attributes never generate inotify
//! events.)

use legion_hw::SysRoot;
use legion_hw::profile::PowerProfile;
use legion_hw::profile_watch::ProfileWatchFd;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

/// Awaits firmware-initiated profile changes.
pub struct ProfileWatcher {
    fd: AsyncFd<ProfileWatchFd>,
    last: Option<PowerProfile>,
}

impl ProfileWatcher {
    /// Register the profile attribute on the reactor.
    ///
    /// Returns `None` when this machine has no platform-profile device — there
    /// is then simply nothing to watch, and the daemon runs on without it.
    pub fn new(root: &SysRoot) -> Option<Self> {
        let watch_fd = match ProfileWatchFd::open(root) {
            Ok(w) => w,
            Err(e) => {
                log::info!("not watching for profile changes: {e}");
                return None;
            }
        };

        match AsyncFd::with_interest(watch_fd, Interest::PRIORITY) {
            Ok(fd) => Some(Self { fd, last: None }),
            Err(e) => {
                log::warn!("could not register the profile watcher with the reactor: {e}");
                None
            }
        }
    }

    /// Wait for the next *change* to the profile.
    ///
    /// `EPOLLPRI` also fires for the daemon's own writes, so identical
    /// consecutive values are swallowed here rather than echoed to clients as
    /// spurious `PropertiesChanged` signals.
    pub async fn next_change(&mut self) -> PowerProfile {
        loop {
            let mut guard = match self.fd.ready_mut(Interest::PRIORITY).await {
                Ok(g) => g,
                Err(e) => {
                    log::warn!("profile watcher failed: {e}");
                    // Never spin on a broken descriptor.
                    std::future::pending::<()>().await;
                    unreachable!()
                }
            };

            // Re-reading is what re-arms the notification.
            let value = guard.get_inner_mut().consume();
            guard.clear_ready();

            match value {
                Ok(p) => {
                    if self.last != Some(p) {
                        self.last = Some(p);
                        return p;
                    }
                }
                Err(e) => log::warn!("could not re-read the profile: {e}"),
            }
        }
    }
}

/// Await a change if watching is possible, otherwise never resolve — lets the
/// caller write one `select!` arm without special-casing the absent watcher.
pub async fn next_change(watcher: &mut Option<ProfileWatcher>) -> PowerProfile {
    match watcher.as_mut() {
        Some(w) => w.next_change().await,
        None => std::future::pending().await,
    }
}
