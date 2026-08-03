//! Hardware backend for Lenovo Legion laptops.
//!
//! Every path this crate touches lives under a [`SysRoot`], which defaults to
//! `/` but can be pointed at a temporary directory in tests. Nothing here
//! hardcodes `/sys` at a call site, and every accessor feature-detects: absent
//! files yield [`HwError::NotSupported`] or `None`, never a panic.
//!
//! The ABI targeted is the upstream `lenovo-wmi-*` + `ideapad_laptop` driver
//! stack (kernel 6.17+, verified on 7.0). The out-of-tree LenovoLegionLinux
//! kernel module is explicitly *not* used, and none of its sysfs paths appear
//! anywhere in this crate. It may still be installed alongside, though: its
//! `legion_laptop` module registers a second platform-profile handler, so
//! [`profile::device_dir`] identifies the upstream one by `name` rather than
//! trusting the kernel's `platform-profile-N` numbering.
//!
//! Writes here go straight to sysfs, so they succeed for root (the daemon) and
//! fail with [`HwError::PermissionDenied`] for anyone else. Unprivileged
//! clients read directly but write through the daemon over D-Bus.

pub mod battery;
pub mod cpu;
pub mod detect;
pub mod error;
pub mod fan;
pub mod firmware_attrs;
pub mod ideapad;
pub mod kbd_backlight;
pub mod profile;
pub mod profile_watch;
pub mod sysfs;

pub use detect::Capabilities;
pub use error::HwError;
pub use sysfs::SysRoot;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, HwError>;
