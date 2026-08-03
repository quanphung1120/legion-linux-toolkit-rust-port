//! `legion-ctl` — command-line control for Legion laptops.
//!
//! A thin D-Bus client over `legiond`, with a direct-sysfs fallback so that
//! reads (and root-run writes) still work when the daemon is not installed.
//! `status` prints plain `key: value` lines so it can be parsed by scripts.

mod backend;
mod commands;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::backend::{Backend, BusKind};

#[derive(Debug, Parser)]
#[command(
    name = "legion-ctl",
    about = "Control Lenovo Legion laptop hardware",
    version
)]
struct Cli {
    /// Which bus to find the daemon on (testing only).
    #[arg(long, hide = true, value_enum, default_value = "system", global = true)]
    bus: BusKind,

    /// Alternate sysfs root (testing only).
    #[arg(long, hide = true, global = true)]
    sysfs_root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show a capability-aware summary of the machine.
    Status,

    /// Get or set the power profile.
    Profile {
        /// quiet | balanced | performance | extreme | custom
        value: Option<String>,

        /// Print profile changes as they happen (including Fn+Q).
        #[arg(long)]
        watch: bool,
    },

    /// Get or set a CPU power limit, in watts.
    Ppt {
        /// spl | sppt | fppt (omit to list all)
        attr: Option<String>,
        /// Watts (omit to read)
        watts: Option<u32>,
    },

    /// Get or set the battery charge mode.
    Battery {
        /// standard | fast | long-life
        value: Option<String>,
    },

    /// Get or set Fn Lock.
    Fnlock {
        /// on | off
        value: Option<String>,
    },

    /// Get or set camera power.
    Camera {
        /// on | off
        value: Option<String>,
    },

    /// Get or set always-on USB charging.
    UsbCharging {
        /// on | off
        value: Option<String>,
    },

    /// Get or set the keyboard backlight level.
    Backlight {
        /// 0 | 1 | 2
        value: Option<u32>,
    },

    /// Get or set CPU boost.
    Boost {
        /// on | off
        value: Option<String>,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let backend = Backend::connect(cli.bus, cli.sysfs_root.clone()).await;

    let result = match cli.command {
        Command::Status => commands::status(&backend).await,
        Command::Profile { value, watch } => {
            if watch {
                commands::profile_watch(&backend).await
            } else {
                commands::profile(&backend, value.as_deref()).await
            }
        }
        Command::Ppt { attr, watts } => commands::ppt(&backend, attr.as_deref(), watts).await,
        Command::Battery { value } => commands::battery(&backend, value.as_deref()).await,
        Command::Fnlock { value } => {
            commands::toggle(&backend, commands::ToggleKind::FnLock, value.as_deref()).await
        }
        Command::Camera { value } => {
            commands::toggle(&backend, commands::ToggleKind::Camera, value.as_deref()).await
        }
        Command::UsbCharging { value } => {
            commands::toggle(
                &backend,
                commands::ToggleKind::UsbCharging,
                value.as_deref(),
            )
            .await
        }
        Command::Backlight { value } => commands::backlight(&backend, value).await,
        Command::Boost { value } => {
            commands::toggle(&backend, commands::ToggleKind::Boost, value.as_deref()).await
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("legion-ctl: {msg}");
            ExitCode::FAILURE
        }
    }
}
