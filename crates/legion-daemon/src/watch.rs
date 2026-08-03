//! Fn+Q propagation.
//!
//! The firmware changes the platform profile behind the daemon's back when the
//! user presses Fn+Q. The only notification the kernel offers is `POLLPRI` on
//! the `profile` attribute, which is a blocking syscall — so it runs on a
//! dedicated OS thread and hands changes to the async side through a channel.
//! From there a `PropertiesChanged` signal reaches every client, and the GUI,
//! tray and `legion-ctl --watch` all update without polling.

use std::time::Duration;

use legion_hw::SysRoot;
use legion_hw::profile::PowerProfile;
use legion_hw::profile_watch::{ProfileWatcher, WatchEvent};
use tokio::sync::mpsc;

/// How long each `poll()` waits before looping. Bounded so the thread notices
/// a shutdown request promptly.
const POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn the watcher thread; returns the channel of observed profiles.
///
/// Returns `None` when this machine has no platform-profile attribute, in
/// which case there is simply nothing to watch.
pub fn spawn(root: &SysRoot) -> Option<mpsc::UnboundedReceiver<PowerProfile>> {
    let mut watcher = match ProfileWatcher::new(root) {
        Ok(w) => w,
        Err(e) => {
            log::info!("not watching for profile changes: {e}");
            return None;
        }
    };

    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::Builder::new()
        .name("profile-watch".into())
        .spawn(move || {
            let mut last: Option<PowerProfile> = None;
            loop {
                match watcher.wait_for_change(POLL_TIMEOUT) {
                    Ok(WatchEvent::Changed(p)) => {
                        // POLLPRI also fires for our own writes; only report
                        // transitions, so clients do not see duplicates.
                        if last != Some(p) {
                            last = Some(p);
                            if tx.send(p).is_err() {
                                break; // daemon is shutting down
                            }
                        }
                    }
                    Ok(WatchEvent::Timeout) => {}
                    Err(e) => {
                        log::warn!("profile watcher stopped: {e}");
                        break;
                    }
                }
            }
        })
        .ok()?;

    Some(rx)
}
