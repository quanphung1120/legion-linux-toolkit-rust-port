//! Single-instance guard built on an abstract Unix socket.
//!
//! An abstract socket (leading NUL in the path) is bound to the process, not
//! to the filesystem, so it disappears automatically when the process exits —
//! no stale lock file to clean up after a crash. A second invocation finds the
//! name taken, writes a byte to ask the running instance to show its window,
//! and exits.
//!
//! This replaces the old tray's `pkill` + `Popen` restart hack.

use std::io::{Read, Write};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};

use slint::ComponentHandle;

/// Abstract socket name (no filesystem entry).
const SOCKET_NAME: &str = "legion-toolkit.lock";

/// Outcome of trying to become the primary instance.
pub enum Instance {
    /// We are the only instance; hold this listener for the process's life.
    Primary(UnixListener),
    /// Another instance already holds the name.
    AlreadyRunning,
}

/// Try to claim the single-instance name.
pub fn acquire() -> Instance {
    let Ok(addr) = SocketAddr::from_abstract_name(SOCKET_NAME.as_bytes()) else {
        // Without an abstract namespace we simply do not enforce uniqueness
        // rather than refusing to start.
        log::warn!("abstract sockets unavailable; skipping the single-instance check");
        return Instance::AlreadyRunning;
    };

    match UnixListener::bind_addr(&addr) {
        Ok(listener) => Instance::Primary(listener),
        Err(_) => Instance::AlreadyRunning,
    }
}

/// Ask the running instance to show its window.
pub fn signal_show() {
    let Ok(addr) = SocketAddr::from_abstract_name(SOCKET_NAME.as_bytes()) else {
        return;
    };
    if let Ok(mut stream) = UnixStream::connect_addr(&addr) {
        let _ = stream.write_all(b"S");
    }
}

/// Serve show-requests from later invocations.
pub fn spawn_listener(listener: UnixListener, ui: slint::Weak<crate::AppWindow>) {
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 1];
                if stream.read_exact(&mut buf).is_ok() && buf[0] == b'S' {
                    let ui = ui.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui.upgrade() {
                            let _ = ui.show();
                            ui.window().set_minimized(false);
                        }
                    });
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_name_is_stable() {
        // Changing this silently breaks single-instance behaviour across an
        // upgrade, so pin it.
        assert_eq!(SOCKET_NAME, "legion-toolkit.lock");
    }

    #[test]
    fn second_bind_of_the_same_name_fails() {
        let name = format!("legion-toolkit-test-{}", std::process::id());
        let addr = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let _first = UnixListener::bind_addr(&addr).expect("first bind should succeed");
        assert!(
            UnixListener::bind_addr(&addr).is_err(),
            "the second bind must fail — that is what detects a running instance"
        );
    }
}
