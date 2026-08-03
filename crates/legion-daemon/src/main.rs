//! `legiond` — the Legion Toolkit hardware control daemon.
//!
//! Runs as root under systemd (`Type=dbus`) and owns
//! `org.legiontoolkit.Daemon` on the system bus. It is the only process that
//! writes to sysfs; the GUI, tray and CLI are all D-Bus clients, and each
//! mutating call is checked against polkit.
//!
//! Two hidden flags exist for tests: `--bus session` and `--sysfs-root`, which
//! together run the identical interface on a private session bus against a
//! fake sysfs tree, needing no root.

mod auth;
mod error;
mod iface;
mod state;
mod watch;

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use legion_hw::{SysRoot, detect};
use zbus::connection;

use crate::auth::Authorizer;
use crate::iface::{BUS_NAME, Daemon, IFACE_NAME, OBJECT_PATH};
use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BusKind {
    System,
    Session,
}

#[derive(Debug, Parser)]
#[command(
    name = "legiond",
    about = "Legion Toolkit hardware control daemon",
    version
)]
struct Args {
    /// Log to stderr and stay in the foreground.
    #[arg(long)]
    foreground: bool,

    /// Alternate sysfs root (testing only).
    #[arg(long, hide = true)]
    sysfs_root: Option<PathBuf>,

    /// Which bus to own the name on (testing only).
    #[arg(long, hide = true, value_enum, default_value = "system")]
    bus: BusKind,

    /// Alternate state file (testing only).
    #[arg(long, hide = true)]
    state_file: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(if args.foreground { "info" } else { "warn" }),
    )
    .init();

    let root = match &args.sysfs_root {
        Some(p) => SysRoot::at(p.clone()),
        None => SysRoot::system(),
    };

    // Report what we found, but never refuse to run: any machine exposing the
    // same upstream ABI is fine, and an unsupported one just shows less.
    let caps = detect::probe(&root);
    if !caps.is_legion {
        log::warn!(
            "DMI does not identify a Legion machine (product_family={:?}); continuing anyway",
            caps.product_family
        );
    }
    if caps.is_empty() {
        log::warn!(
            "no supported control interfaces found — is the lenovo-wmi-gamezone driver loaded?"
        );
    } else {
        log::info!(
            "detected: profile={} ppt={} charge_types={} kbd_backlight={} fan_rpm={}",
            caps.platform_profile,
            caps.ppt,
            caps.charge_types,
            caps.kbd_backlight,
            caps.fan_rpm
        );
    }

    let state_path = state::state_path(args.state_file.clone());
    let persisted = State::load(&state_path);
    if persisted.is_empty() {
        log::debug!("no persisted state to reapply");
    } else {
        log::info!(
            "reapplying persisted settings from {}",
            state_path.display()
        );
    }

    let authorizer = match args.bus {
        // On a private session bus there is no polkit and no privileged sysfs
        // — this path exists only for integration tests.
        BusKind::Session => Authorizer::AllowAll,
        BusKind::System => {
            let probe_conn = zbus::Connection::system().await?;
            match Authorizer::polkit(&probe_conn).await {
                Ok(a) => a,
                Err(e) => {
                    log::error!("cannot reach polkit: {e}");
                    return Err(e.into());
                }
            }
        }
    };

    let daemon = Daemon::new(root.clone(), persisted, state_path, authorizer);
    daemon.reapply_state();

    let conn = match args.bus {
        BusKind::System => connection::Builder::system()?,
        BusKind::Session => connection::Builder::session()?,
    }
    .name(BUS_NAME)?
    .serve_at(OBJECT_PATH, daemon)?
    .build()
    .await?;

    log::info!(
        "listening on {} bus as {BUS_NAME}, serving {IFACE_NAME} at {OBJECT_PATH}",
        match args.bus {
            BusKind::System => "system",
            BusKind::Session => "session",
        }
    );

    // Fn+Q propagation: an EPOLLPRI task on the reactor, not a thread.
    let mut profile_watcher = watch::ProfileWatcher::new(&root);

    // Reapply after resume: the firmware forgets the battery mode and power
    // limits across a suspend cycle.
    let mut sleep_rx = sleep_watch(&conn).await;

    let iface_ref = conn
        .object_server()
        .interface::<_, Daemon>(OBJECT_PATH)
        .await?;

    loop {
        tokio::select! {
            p = watch::next_change(&mut profile_watcher) => {
                log::info!("firmware changed profile to {}", p.as_sysfs());
                let iface = iface_ref.get().await;
                if let Err(e) = iface.notify_profile_changed(iface_ref.signal_emitter()).await {
                    log::warn!("could not emit profile change: {e}");
                }
            }

            Some(resuming) = async {
                match sleep_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if resuming {
                    log::info!("resumed from sleep — reapplying settings");
                    let iface = iface_ref.get().await;
                    iface.reapply_state();
                    if let Err(e) = iface.notify_profile_changed(iface_ref.signal_emitter()).await {
                        log::warn!("could not emit profile change: {e}");
                    }
                }
            }

            _ = tokio::signal::ctrl_c() => {
                log::info!("shutting down");
                break;
            }
        }
    }

    Ok(())
}

/// Subscribe to logind's `PrepareForSleep`. Yields `true` on resume.
///
/// Returns `None` when logind is unavailable — the daemon still works, it just
/// does not reapply settings after suspend.
async fn sleep_watch(
    conn: &zbus::Connection,
) -> Option<tokio::sync::mpsc::UnboundedReceiver<bool>> {
    use futures_util::StreamExt;
    use logind_zbus::manager::ManagerProxy;

    let proxy = match ManagerProxy::new(conn).await {
        Ok(p) => p,
        Err(e) => {
            log::info!("logind unavailable, not reapplying after resume: {e}");
            return None;
        }
    };
    let mut stream = match proxy.receive_prepare_for_sleep().await {
        Ok(s) => s,
        Err(e) => {
            log::info!("cannot subscribe to PrepareForSleep: {e}");
            return None;
        }
    };

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(signal) = stream.next().await {
            let Ok(args) = signal.args() else { continue };
            // `true` means "about to sleep", `false` means "just resumed".
            if tx.send(!args.start).is_err() {
                break;
            }
        }
    });
    Some(rx)
}
