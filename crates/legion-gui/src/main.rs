//! `legion-gui` — Slint dashboard and tray for the Legion Toolkit.
//!
//! A pure D-Bus client: every hardware write goes through `legiond`. Read-only
//! stats that need no privileges (CPU temperature, frequency, battery gauge)
//! are read directly from sysfs on the same tick, which keeps the daemon out
//! of a once-a-second polling loop.
//!
//! Threading: Slint owns the main thread, tokio runs the bus work on its own
//! threads, and results cross back through `slint::invoke_from_event_loop` —
//! Slint's UI types are not `Send`, and blocking the UI thread on a D-Bus call
//! would freeze the window.

mod bus;
mod single_instance;
mod state;
mod tray;

use std::rc::Rc;
use std::time::Duration;

use clap::Parser;
use legion_hw::{SysRoot, battery, cpu, fan, ideapad};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tokio::sync::mpsc;

use crate::bus::{Daemon, Event, Request, Snapshot};

slint::include_modules!();

/// Where the icon lives once installed; the repo copy is the dev fallback.
const INSTALLED_LOGO: &str = "/usr/share/icons/hicolor/512x512/apps/legion-toolkit.png";
const DEV_LOGO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.png");

#[derive(Debug, Parser)]
#[command(
    name = "legion-gui",
    about = "Legion Toolkit dashboard and tray",
    version
)]
struct Args {
    /// Start hidden with only the tray icon.
    #[arg(long)]
    tray: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = Args::parse();

    // A second invocation raises the running window rather than starting a
    // rival process. This replaces the old tray's pkill-and-respawn hack.
    let listener = match single_instance::acquire() {
        single_instance::Instance::Primary(listener) => listener,
        single_instance::Instance::AlreadyRunning => {
            single_instance::signal_show();
            log::info!("another instance is already running — asked it to show its window");
            return Ok(());
        }
    };

    let ui = AppWindow::new()?;
    single_instance::spawn_listener(listener, ui.as_weak());
    ui.set_app_version(SharedString::from(env!("CARGO_PKG_VERSION")));
    for path in [INSTALLED_LOGO, DEV_LOGO] {
        if let Ok(logo) = slint::Image::load_from_path(std::path::Path::new(path)) {
            ui.set_logo(logo);
            break;
        }
    }

    let (req_tx, req_rx) = mpsc::unbounded_channel::<Request>();
    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<Event>();

    // ── tokio side: owns the bus connection ─────────────────────────────────
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();
    handle.spawn(bus_task(req_rx, evt_tx.clone()));

    wire_callbacks(&ui, req_tx.clone());

    // ── tray ────────────────────────────────────────────────────────────────
    let tray_handle = tray::spawn(req_tx.clone(), ui.as_weak()).ok();
    if tray_handle.is_none() {
        log::warn!("could not register a tray icon (no StatusNotifierItem host?)");
    }

    if args.tray {
        // Start hidden: in tray mode the icon is the entry point.
        log::info!("starting in tray mode");
    } else {
        ui.show()?;
    }

    // ── pump daemon events into Slint properties ────────────────────────────
    let weak = ui.as_weak();
    let tray_for_events = tray_handle.clone();
    handle.spawn(async move {
        while let Some(event) = evt_rx.recv().await {
            let weak = weak.clone();
            let tray = tray_for_events.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                match event {
                    Event::Snapshot(snap) => {
                        apply_snapshot(&ui, &snap);
                        if let Some(t) = &tray {
                            t.update(&snap);
                        }
                    }
                    Event::Toast(msg) => ui.set_toast(SharedString::from(msg)),
                    Event::Disconnected(why) => {
                        ui.set_daemon_connected(false);
                        ui.set_daemon_error(SharedString::from(why));
                    }
                }
            });
        }
    });

    // ── unprivileged local stats, once a second ─────────────────────────────
    let weak = ui.as_weak();
    let stats_timer = slint::Timer::default();
    stats_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            if let Some(ui) = weak.upgrade() {
                apply_local_stats(&ui);
            }
        },
    );

    // Ask for the first snapshot straight away.
    let _ = req_tx.send(Request::Refresh);

    slint::run_event_loop()?;
    Ok(())
}

/// The bus task: connect, subscribe, and serve UI requests.
async fn bus_task(
    mut req_rx: mpsc::UnboundedReceiver<Request>,
    evt_tx: mpsc::UnboundedSender<Event>,
) {
    use futures_util::StreamExt;

    let daemon = match Daemon::connect().await {
        Ok(d) => d,
        Err(e) => {
            let _ = evt_tx.send(Event::Disconnected(e));
            return;
        }
    };

    async fn refresh(daemon: &Daemon, tx: &mpsc::UnboundedSender<Event>) {
        match daemon.snapshot().await {
            Ok(s) => {
                let _ = tx.send(Event::Snapshot(Box::new(s)));
            }
            Err(e) => {
                let _ = tx.send(Event::Disconnected(e));
            }
        }
    }

    refresh(&daemon, &evt_tx).await;

    // PropertiesChanged keeps the GUI in step with Fn+Q, the CLI and the tray,
    // so no client ever needs its own POLLPRI watcher.
    let mut changes = daemon
        .proxy()
        .receive_signal("org.freedesktop.DBus.Properties.PropertiesChanged")
        .await
        .ok();

    loop {
        tokio::select! {
            maybe_req = req_rx.recv() => {
                let Some(req) = maybe_req else { break };
                match daemon.apply(req).await {
                    Ok(Some(msg)) => { let _ = evt_tx.send(Event::Toast(msg)); }
                    Ok(None) => {}
                    Err(e) => { let _ = evt_tx.send(Event::Disconnected(e)); }
                }
                refresh(&daemon, &evt_tx).await;
            }

            Some(_) = async {
                match changes.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                refresh(&daemon, &evt_tx).await;
            }
        }
    }
}

fn strings(v: &[String]) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = v.iter().map(SharedString::from).collect();
    ModelRc::from(Rc::new(VecModel::from(items)))
}

/// Push a daemon snapshot into the window's properties.
fn apply_snapshot(ui: &AppWindow, snap: &Snapshot) {
    let caps = &snap.capabilities;

    ui.set_daemon_connected(true);
    ui.set_daemon_error(SharedString::default());
    ui.set_daemon_version(SharedString::from(state::or_dash(Some(
        &snap.daemon_version,
    ))));
    ui.set_machine(SharedString::from(state::or_dash(
        caps.product_name.as_deref(),
    )));
    ui.set_family(SharedString::from(state::or_dash(
        caps.product_family.as_deref(),
    )));
    ui.set_profile_driver(SharedString::from(state::or_dash(
        caps.platform_profile_driver.as_deref(),
    )));

    // Profile: selection compares kernel tokens, display uses labels.
    ui.set_profile_supported(caps.platform_profile);
    ui.set_profile_choices(strings(&snap.profile_choices));
    ui.set_profile_labels(strings(&state::profile_labels(&snap.profile_choices)));
    ui.set_current_profile(SharedString::from(snap.profile.clone()));
    ui.set_current_profile_label(SharedString::from(state::profile_label(&snap.profile)));
    let (r, g, b) = state::profile_rgb(&snap.profile);
    ui.set_profile_color(slint::Brush::SolidColor(slint::Color::from_rgb_u8(r, g, b)));
    ui.set_custom_active(snap.profile == "custom");

    // Battery mode
    ui.set_battery_supported(caps.charge_types);
    ui.set_battery_mode(SharedString::from(snap.battery_mode.clone()));
    ui.set_battery_choices(strings(&caps.charge_type_choices));
    let (labels, hints) = state::battery_labels(&caps.charge_type_choices);
    ui.set_battery_labels(strings(&labels));
    ui.set_battery_hints(strings(&hints));

    // Power limits
    ui.set_ppt_supported(caps.ppt);
    let spl = state::SliderBounds::find(&snap.ppt, "spl");
    let sppt = state::SliderBounds::find(&snap.ppt, "sppt");
    let fppt = state::SliderBounds::find(&snap.ppt, "fppt");
    ui.set_spl(spl.value);
    ui.set_spl_min(spl.min);
    ui.set_spl_max(spl.max);
    ui.set_sppt(sppt.value);
    ui.set_sppt_min(sppt.min);
    ui.set_sppt_max(sppt.max);
    ui.set_fppt(fppt.value);
    ui.set_fppt_min(fppt.min);
    ui.set_fppt_max(fppt.max);

    // System switches — each hidden when the machine lacks the attribute.
    ui.set_has_fn_lock(caps.fn_lock);
    ui.set_has_camera(caps.camera_power);
    ui.set_has_usb_charging(caps.usb_charging);
    ui.set_has_backlight(caps.kbd_backlight);
    ui.set_has_boost(caps.cpu_boost);
    ui.set_fn_lock(snap.fn_lock);
    ui.set_camera(snap.camera);
    ui.set_usb_charging(snap.usb_charging);
    ui.set_boost(snap.boost);
    ui.set_backlight(snap.backlight as i32);
    ui.set_backlight_max(caps.kbd_backlight_max.unwrap_or(2) as i32);
    ui.set_kernel_note(SharedString::from(state::kernel_note(caps)));
}

/// Read the unprivileged sysfs values the dashboard refreshes every second.
fn apply_local_stats(ui: &AppWindow) {
    let root = SysRoot::system();

    let info = cpu::info(&root);
    ui.set_cpu_temp(SharedString::from(state::format_temp(info.temp_celsius)));
    ui.set_cpu_freq(SharedString::from(state::format_freq(info.cur_freq_khz)));
    ui.set_cpu_governor(SharedString::from(state::or_dash(info.governor.as_deref())));
    ui.set_fan_rpm(SharedString::from(state::format_fan(fan::rpm(&root))));

    let stats = battery::stats(&root);
    ui.set_battery_capacity(SharedString::from(state::battery_capacity_text(&stats)));
    ui.set_battery_status(SharedString::from(state::or_dash(stats.status.as_deref())));
    ui.set_ac_state(SharedString::from(state::ac_text(&stats)));
    ui.set_battery_health(SharedString::from(state::format_pct(stats.health_pct)));
    ui.set_battery_cycles(SharedString::from(match stats.cycle_count {
        Some(c) => c.to_string(),
        None => "—".to_string(),
    }));
    ui.set_battery_power(SharedString::from(state::format_micro(
        stats.power_now,
        "W",
    )));
    ui.set_battery_voltage(SharedString::from(state::format_micro(
        stats.voltage_now,
        "V",
    )));

    ui.set_fan_mode(SharedString::from(match ideapad::get_fan_mode(&root) {
        Ok(m) => m.label().to_string(),
        Err(_) => "—".to_string(),
    }));
}

/// Connect every UI callback to a daemon request.
fn wire_callbacks(ui: &AppWindow, tx: mpsc::UnboundedSender<Request>) {
    {
        let tx = tx.clone();
        ui.on_select_profile(move |p| {
            let _ = tx.send(Request::SetProfile(p.to_string()));
        });
    }
    {
        let tx = tx.clone();
        ui.on_select_battery_mode(move |m| {
            let _ = tx.send(Request::SetBatteryMode(m.to_string()));
        });
    }
    {
        let tx = tx.clone();
        ui.on_set_fn_lock(move |v| {
            let _ = tx.send(Request::SetFnLock(v));
        });
    }
    {
        let tx = tx.clone();
        ui.on_set_camera(move |v| {
            let _ = tx.send(Request::SetCameraPower(v));
        });
    }
    {
        let tx = tx.clone();
        ui.on_set_usb_charging(move |v| {
            let _ = tx.send(Request::SetUsbCharging(v));
        });
    }
    {
        let tx = tx.clone();
        ui.on_set_boost(move |v| {
            let _ = tx.send(Request::SetCpuBoost(v));
        });
    }
    {
        let tx = tx.clone();
        ui.on_set_backlight(move |level| {
            let _ = tx.send(Request::SetKbdBacklight(level.max(0) as u32));
        });
    }
    {
        let tx = tx.clone();
        ui.on_set_ppt(move |attr, watts| {
            let _ = tx.send(Request::SetPpt(attr.to_string(), watts.max(0) as u32));
        });
    }

    let weak = ui.as_weak();
    ui.on_dismiss_toast(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_toast(SharedString::default());
        }
    });
}
