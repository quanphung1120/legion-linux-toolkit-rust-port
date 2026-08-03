//! The `org.legiontoolkit.Daemon1` interface.
//!
//! This is the public API of the toolkit: the GUI, the tray and the CLI are
//! all clients of it, and nothing but this daemon writes to sysfs. Treat
//! renames and removals as breaking — add a `Daemon2` interface alongside
//! instead.

use std::path::PathBuf;

use legion_hw::battery::ChargeMode;
use legion_hw::firmware_attrs::AttrInfo;
use legion_hw::ideapad::{FanMode, Toggle};
use legion_hw::profile::PowerProfile;
use legion_hw::{
    SysRoot, battery, cpu, detect, fan, firmware_attrs, ideapad, kbd_backlight, profile,
};
use zbus::interface;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

use crate::auth::Authorizer;
use crate::error::{DaemonError, Result};
use crate::state::State;

/// Well-known bus name.
pub const BUS_NAME: &str = "org.legiontoolkit.Daemon";
/// Object path.
pub const OBJECT_PATH: &str = "/org/legiontoolkit/Daemon";
/// Interface name.
pub const IFACE_NAME: &str = "org.legiontoolkit.Daemon1";

/// The daemon's single interface object.
pub struct Daemon {
    root: SysRoot,
    state: State,
    state_path: PathBuf,
    authorizer: Authorizer,
}

impl Daemon {
    pub fn new(root: SysRoot, state: State, state_path: PathBuf, authorizer: Authorizer) -> Self {
        Self {
            root,
            state,
            state_path,
            authorizer,
        }
    }

    /// Persist state, logging rather than failing the client's call — the
    /// hardware change already succeeded, so a state-file problem must not be
    /// reported as if the setting did not apply.
    fn persist(&self) {
        if let Err(e) = self.state.save(&self.state_path) {
            log::warn!(
                "could not persist state to {}: {e}",
                self.state_path.display()
            );
        }
    }

    /// Check the caller against polkit action `org.legiontoolkit.control`.
    async fn authorize(&self, hdr: &Header<'_>) -> Result<()> {
        self.authorizer.check(hdr).await
    }

    /// Reapply everything in the persisted state.
    ///
    /// Called at startup and after resume from sleep, because the firmware
    /// forgets the battery mode and power limits. Failures are logged, not
    /// fatal: a machine may legitimately lack an interface it had before.
    pub fn reapply_state(&self) {
        if let Some(p) = &self.state.profile
            && let Ok(parsed) = PowerProfile::from_sysfs(p)
            && let Err(e) = profile::set(&self.root, parsed)
        {
            log::warn!("could not restore profile {p}: {e}");
        }

        if let Some(m) = &self.state.battery_mode
            && let Ok(parsed) = ChargeMode::from_sysfs(m)
            && let Err(e) = battery::set_mode(&self.root, parsed)
        {
            log::warn!("could not restore battery mode {m}: {e}");
        }

        for (key, watts) in &self.state.ppt {
            let Ok(attr) = firmware_attrs::PptAttr::from_key(key) else {
                continue;
            };
            if let Err(e) = firmware_attrs::write(&self.root, attr, *watts) {
                // EBUSY here is expected and benign: the limits only apply in
                // Custom mode, and we may not be in it.
                log::debug!("could not restore {key}={watts}W: {e}");
            }
        }
    }

    /// Re-read the profile and emit `PropertiesChanged`. The POLLPRI watcher
    /// calls this when the firmware changes the profile behind our back
    /// (Fn+Q).
    pub async fn notify_profile_changed(&self, emitter: &SignalEmitter<'_>) -> zbus::Result<()> {
        self.profile_changed(emitter).await
    }
}

#[interface(name = "org.legiontoolkit.Daemon1")]
impl Daemon {
    // ── Properties ─────────────────────────────────────────────────────────

    /// Active power profile, as the kernel string (`balanced`, `max-power`…).
    #[zbus(property)]
    async fn profile(&self) -> String {
        profile::get(&self.root)
            .map(|p| p.as_sysfs().to_string())
            .unwrap_or_default()
    }

    /// Profiles this firmware advertises.
    #[zbus(property)]
    async fn profile_choices(&self) -> Vec<String> {
        profile::choices(&self.root)
            .map(|v| v.into_iter().map(|p| p.as_sysfs().to_string()).collect())
            .unwrap_or_default()
    }

    /// Active battery charge type (`Standard`, `Fast`, `Long_Life`).
    #[zbus(property)]
    async fn battery_mode(&self) -> String {
        battery::get_mode(&self.root)
            .map(|m| m.as_sysfs().to_string())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn fn_lock(&self) -> bool {
        ideapad::get(&self.root, Toggle::FnLock).unwrap_or(false)
    }

    #[zbus(property)]
    async fn camera_power(&self) -> bool {
        ideapad::get(&self.root, Toggle::CameraPower).unwrap_or(false)
    }

    #[zbus(property)]
    async fn usb_charging(&self) -> bool {
        ideapad::get(&self.root, Toggle::UsbCharging).unwrap_or(false)
    }

    #[zbus(property)]
    async fn kbd_backlight(&self) -> u32 {
        kbd_backlight::get(&self.root).unwrap_or(0)
    }

    #[zbus(property)]
    async fn cpu_boost(&self) -> bool {
        cpu::get_boost(&self.root).unwrap_or(false)
    }

    /// JSON of `legion_hw::detect::Capabilities` — clients hide what this
    /// machine cannot do.
    #[zbus(property)]
    async fn capabilities(&self) -> String {
        detect::probe(&self.root).to_json().unwrap_or_default()
    }

    // ── Methods ────────────────────────────────────────────────────────────

    async fn set_profile(
        &mut self,
        value: &str,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;
        let p = PowerProfile::from_user(value).map_err(DaemonError::from)?;
        profile::set(&self.root, p).map_err(DaemonError::from)?;

        self.state.profile = Some(p.as_sysfs().to_string());
        self.persist();
        self.profile_changed(&emitter).await?;
        log::info!("profile set to {}", p.as_sysfs());
        Ok(())
    }

    async fn set_battery_mode(
        &mut self,
        value: &str,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;
        let m = ChargeMode::from_user(value).map_err(DaemonError::from)?;
        battery::set_mode(&self.root, m).map_err(DaemonError::from)?;

        self.state.battery_mode = Some(m.as_sysfs().to_string());
        self.persist();
        self.battery_mode_changed(&emitter).await?;
        log::info!("battery mode set to {}", m.as_sysfs());
        Ok(())
    }

    async fn set_fn_lock(
        &mut self,
        value: bool,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;
        ideapad::set(&self.root, Toggle::FnLock, value).map_err(DaemonError::from)?;
        self.fn_lock_changed(&emitter).await?;
        Ok(())
    }

    async fn set_camera_power(
        &mut self,
        value: bool,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;
        ideapad::set(&self.root, Toggle::CameraPower, value).map_err(DaemonError::from)?;
        self.camera_power_changed(&emitter).await?;
        Ok(())
    }

    async fn set_usb_charging(
        &mut self,
        value: bool,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;
        ideapad::set(&self.root, Toggle::UsbCharging, value).map_err(DaemonError::from)?;
        self.usb_charging_changed(&emitter).await?;
        Ok(())
    }

    async fn set_kbd_backlight(
        &mut self,
        value: u32,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;
        kbd_backlight::set(&self.root, value).map_err(DaemonError::from)?;
        self.kbd_backlight_changed(&emitter).await?;
        Ok(())
    }

    async fn set_cpu_boost(
        &mut self,
        value: bool,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;
        cpu::set_boost(&self.root, value).map_err(DaemonError::from)?;
        self.cpu_boost_changed(&emitter).await?;
        Ok(())
    }

    /// Every advertised firmware attribute, as a JSON array of `AttrInfo`.
    ///
    /// This scans the attributes directory, so a kernel that starts exposing
    /// more of them needs no daemon change.
    async fn get_ppt(&self) -> Result<String> {
        let all: Vec<AttrInfo> = firmware_attrs::list_all(&self.root);
        serde_json::to_string(&all).map_err(|e| DaemonError::Failed(e.to_string()))
    }

    /// Set a power limit, in watts. Fails with
    /// `org.legiontoolkit.Error.CustomModeRequired` outside Custom mode.
    async fn set_ppt(
        &mut self,
        attr: &str,
        watts: u32,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<()> {
        self.authorize(&hdr).await?;

        // Fail fast with the actionable error rather than relying on the
        // kernel's EBUSY, so the message is right even if a driver stops
        // enforcing the gate.
        let current = profile::get(&self.root).map_err(DaemonError::from)?;
        if current != PowerProfile::Custom {
            return Err(DaemonError::CustomModeRequired(format!(
                "power limits require the Custom profile; current profile is {}",
                current.as_sysfs()
            )));
        }

        let key = firmware_attrs::PptAttr::from_key(attr)
            .map(|a| a.key().to_string())
            .unwrap_or_else(|_| attr.to_string());
        let dir_name = firmware_attrs::PptAttr::from_key(attr)
            .map(|a| a.dir_name().to_string())
            .unwrap_or_else(|_| attr.to_string());

        firmware_attrs::write_by_name(&self.root, &dir_name, watts).map_err(DaemonError::from)?;

        self.state.set_ppt(&key, watts);
        self.persist();
        log::info!("{key} set to {watts} W");
        Ok(())
    }

    /// A JSON dump of everything the CLI's `status` shows.
    async fn status(&self) -> Result<String> {
        let caps = detect::probe(&self.root);
        let status = serde_json::json!({
            "capabilities": caps,
            "profile": profile::get(&self.root).ok().map(|p| p.as_sysfs()),
            "profile_label": profile::get(&self.root).ok().map(|p| p.label()),
            "profile_choices": profile::choices(&self.root)
                .map(|v| v.into_iter().map(|p| p.as_sysfs()).collect::<Vec<_>>())
                .unwrap_or_default(),
            "battery_mode": battery::get_mode(&self.root).ok().map(|m| m.as_sysfs()),
            "battery": battery::stats(&self.root),
            "ppt": firmware_attrs::list_all(&self.root),
            "fn_lock": ideapad::get(&self.root, Toggle::FnLock),
            "camera_power": ideapad::get(&self.root, Toggle::CameraPower),
            "usb_charging": ideapad::get(&self.root, Toggle::UsbCharging),
            "fan_mode": ideapad::get_fan_mode(&self.root).ok().map(|m| m.label()),
            "fan_rpm": fan::rpm(&self.root),
            "kbd_backlight": kbd_backlight::get(&self.root).ok(),
            "kbd_backlight_max": kbd_backlight::max(&self.root).ok(),
            "cpu": cpu::info(&self.root),
        });
        serde_json::to_string(&status).map_err(|e| DaemonError::Failed(e.to_string()))
    }

    /// Set the ideapad fan mode (Super Silent / Standard / Dust Cleaning /
    /// Efficient Thermal Dissipation).
    async fn set_fan_mode(&mut self, value: u32, #[zbus(header)] hdr: Header<'_>) -> Result<()> {
        self.authorize(&hdr).await?;
        let m = FanMode::from_value(value).map_err(DaemonError::from)?;
        ideapad::set_fan_mode(&self.root, m).map_err(DaemonError::from)?;
        Ok(())
    }

    /// Daemon version, so clients can detect a stale daemon after an upgrade.
    #[zbus(property)]
    async fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_documented_api() {
        assert_eq!(BUS_NAME, "org.legiontoolkit.Daemon");
        assert_eq!(OBJECT_PATH, "/org/legiontoolkit/Daemon");
        assert_eq!(IFACE_NAME, "org.legiontoolkit.Daemon1");
    }
}
