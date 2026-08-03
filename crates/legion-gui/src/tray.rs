//! StatusNotifierItem tray, via `ksni`.
//!
//! Works out of the box on Ubuntu GNOME (the `ubuntu-appindicators` extension
//! ships enabled) and on KDE Plasma, which supports SNI natively. Vanilla
//! GNOME needs the AppIndicator extension — the README says so, because it is
//! otherwise a support-question magnet.
//!
//! The menu is driven entirely by daemon state, so Fn+Q and `legion-ctl`
//! changes are reflected here too.

use std::sync::{Arc, Mutex};

use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem};
use legion_hw::battery::ChargeMode;
use legion_hw::profile::PowerProfile;
use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::bus::{Request, Snapshot};
use crate::state;

/// What the tray needs to render itself.
#[derive(Debug, Clone, Default)]
struct TrayState {
    profile: String,
    profile_choices: Vec<String>,
    battery_mode: String,
    battery_choices: Vec<String>,
    fn_lock: bool,
    camera: bool,
    has_fn_lock: bool,
    has_camera: bool,
}

impl TrayState {
    fn from_snapshot(s: &Snapshot) -> Self {
        Self {
            profile: s.profile.clone(),
            profile_choices: s.profile_choices.clone(),
            battery_mode: s.battery_mode.clone(),
            battery_choices: s.capabilities.charge_type_choices.clone(),
            fn_lock: s.fn_lock,
            camera: s.camera,
            has_fn_lock: s.capabilities.fn_lock,
            has_camera: s.capabilities.camera_power,
        }
    }

    fn tooltip(&self) -> String {
        if self.profile.is_empty() {
            "Legion Toolkit".to_string()
        } else {
            format!("Legion Toolkit — {}", state::profile_label(&self.profile))
        }
    }
}

struct Tray {
    state: TrayState,
    tx: mpsc::UnboundedSender<Request>,
    ui: slint::Weak<crate::AppWindow>,
}

impl Tray {
    fn show_window(&self) {
        let ui = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                let _ = ui.show();
                ui.window().set_minimized(false);
            }
        });
    }

    fn toggle_window(&self) {
        let ui = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui.upgrade() {
                if ui.window().is_visible() {
                    let _ = ui.hide();
                } else {
                    let _ = ui.show();
                    ui.window().set_minimized(false);
                }
            }
        });
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "legion-toolkit".into()
    }

    fn title(&self) -> String {
        "Legion Toolkit".into()
    }

    /// One pre-rendered icon per profile, tinted with that mode's LED colour.
    fn icon_name(&self) -> String {
        state::tray_icon_name(&self.state.profile)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Legion Toolkit".into(),
            description: self.state.tooltip(),
            icon_name: self.icon_name(),
            icon_pixmap: Vec::new(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.toggle_window();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = Vec::new();

        // ── power profile ───────────────────────────────────────────────────
        if !self.state.profile_choices.is_empty() {
            items.push(
                StandardItem {
                    label: "Power Mode".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );

            let choices = self.state.profile_choices.clone();
            let selected = choices
                .iter()
                .position(|c| *c == self.state.profile)
                .unwrap_or(0);
            let for_activate = choices.clone();

            items.push(
                RadioGroup {
                    selected,
                    select: Box::new(move |this: &mut Self, idx| {
                        if let Some(choice) = for_activate.get(idx) {
                            let _ = this.tx.send(Request::SetProfile(choice.clone()));
                        }
                    }),
                    options: choices
                        .iter()
                        .map(|c| RadioItem {
                            label: PowerProfile::from_sysfs(c)
                                .map(|p| p.label().to_string())
                                .unwrap_or_else(|_| c.clone()),
                            ..Default::default()
                        })
                        .collect(),
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }

        // ── battery mode ────────────────────────────────────────────────────
        if !self.state.battery_choices.is_empty() {
            items.push(
                StandardItem {
                    label: "Battery".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );

            let choices = self.state.battery_choices.clone();
            let selected = choices
                .iter()
                .position(|c| *c == self.state.battery_mode)
                .unwrap_or(0);
            let for_activate = choices.clone();

            items.push(
                RadioGroup {
                    selected,
                    select: Box::new(move |this: &mut Self, idx| {
                        if let Some(choice) = for_activate.get(idx) {
                            let _ = this.tx.send(Request::SetBatteryMode(choice.clone()));
                        }
                    }),
                    options: choices
                        .iter()
                        .map(|c| RadioItem {
                            label: ChargeMode::from_sysfs(c)
                                .map(|m| m.label().to_string())
                                .unwrap_or_else(|_| c.clone()),
                            ..Default::default()
                        })
                        .collect(),
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }

        // ── quick switches ──────────────────────────────────────────────────
        if self.state.has_fn_lock {
            items.push(
                CheckmarkItem {
                    label: "Fn Lock".into(),
                    checked: self.state.fn_lock,
                    activate: Box::new(|this: &mut Self| {
                        let _ = this.tx.send(Request::SetFnLock(!this.state.fn_lock));
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        if self.state.has_camera {
            items.push(
                CheckmarkItem {
                    label: "Camera".into(),
                    checked: self.state.camera,
                    activate: Box::new(|this: &mut Self| {
                        let _ = this.tx.send(Request::SetCameraPower(!this.state.camera));
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        if self.state.has_fn_lock || self.state.has_camera {
            items.push(MenuItem::Separator);
        }

        items.push(
            StandardItem {
                label: "Open Dashboard".into(),
                activate: Box::new(|this: &mut Self| this.show_window()),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    let _ = slint::invoke_from_event_loop(|| {
                        let _ = slint::quit_event_loop();
                    });
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

/// Handle used to push new daemon state into the tray.
///
/// Every method here must be called from the tokio runtime: ksni talks to the
/// StatusNotifierItem host over zbus, which needs a live reactor.
pub struct TrayHandle {
    handle: ksni::Handle<Tray>,
    last: Mutex<TrayState>,
}

impl TrayHandle {
    /// Refresh the tray from a daemon snapshot, skipping no-op updates so the
    /// menu does not churn on every one-second tick.
    pub async fn update(&self, snap: &Snapshot) {
        let next = TrayState::from_snapshot(snap);
        {
            // Scoped so the std::sync guard is released before the await —
            // it must never be held across a suspension point.
            let mut last = self.last.lock().unwrap();
            if *last == next {
                return;
            }
            *last = next.clone();
        }
        self.handle
            .update(move |tray: &mut Tray| {
                tray.state = next;
            })
            .await;
    }
}

impl PartialEq for TrayState {
    fn eq(&self, other: &Self) -> bool {
        self.profile == other.profile
            && self.profile_choices == other.profile_choices
            && self.battery_mode == other.battery_mode
            && self.battery_choices == other.battery_choices
            && self.fn_lock == other.fn_lock
            && self.camera == other.camera
            && self.has_fn_lock == other.has_fn_lock
            && self.has_camera == other.has_camera
    }
}

/// Register the tray item. Fails when the session has no SNI host.
///
/// Must be awaited on the tokio runtime — ksni spawns its zbus service onto
/// whichever runtime is current.
pub async fn spawn(
    tx: mpsc::UnboundedSender<Request>,
    ui: slint::Weak<crate::AppWindow>,
) -> Result<Arc<TrayHandle>, ksni::Error> {
    use ksni::TrayMethods;

    let handle = Tray {
        state: TrayState::default(),
        tx,
        ui,
    }
    .spawn()
    .await?;

    Ok(Arc::new(TrayHandle {
        handle,
        last: Mutex::new(TrayState::default()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use legion_hw::Capabilities;

    fn snapshot() -> Snapshot {
        Snapshot {
            profile: "performance".into(),
            profile_choices: vec!["balanced".into(), "performance".into()],
            battery_mode: "Long_Life".into(),
            fn_lock: true,
            camera: false,
            capabilities: Capabilities {
                fn_lock: true,
                camera_power: true,
                charge_type_choices: vec!["Standard".into(), "Long_Life".into()],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn tray_state_mirrors_the_snapshot() {
        let s = TrayState::from_snapshot(&snapshot());
        assert_eq!(s.profile, "performance");
        assert_eq!(s.profile_choices.len(), 2);
        assert_eq!(s.battery_choices.len(), 2);
        assert!(s.has_fn_lock && s.has_camera);
        assert!(s.fn_lock);
        assert!(!s.camera);
    }

    #[test]
    fn tooltip_uses_the_friendly_profile_name() {
        let s = TrayState::from_snapshot(&snapshot());
        assert_eq!(s.tooltip(), "Legion Toolkit — Performance");
    }

    #[test]
    fn tooltip_without_a_profile_is_still_sensible() {
        assert_eq!(TrayState::default().tooltip(), "Legion Toolkit");
    }

    #[test]
    fn identical_snapshots_compare_equal_so_the_menu_does_not_churn() {
        let a = TrayState::from_snapshot(&snapshot());
        let b = TrayState::from_snapshot(&snapshot());
        assert_eq!(a, b);

        let mut changed = snapshot();
        changed.profile = "balanced".into();
        assert_ne!(a, TrayState::from_snapshot(&changed));
    }
}
