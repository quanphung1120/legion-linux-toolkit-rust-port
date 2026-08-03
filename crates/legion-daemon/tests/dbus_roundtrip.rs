//! End-to-end test of the daemon's D-Bus surface.
//!
//! The daemon is spawned as a real process with `--bus session
//! --sysfs-root <fake 82WM>`, so this exercises the same interface code that
//! runs in production — object server, property emission and error mapping —
//! without needing root, polkit or a real Legion laptop.
//!
//! Each test gets its *own* private `dbus-daemon`, so the tests stay
//! independent and can run in parallel — a single shared bus would let only
//! one daemon own `org.legiontoolkit.Daemon` at a time. Tests skip themselves
//! with a clear message if `dbus-daemon` is not installed.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use zbus::zvariant::OwnedValue;

const BUS_NAME: &str = "org.legiontoolkit.Daemon";
const OBJECT_PATH: &str = "/org/legiontoolkit/Daemon";
const IFACE: &str = "org.legiontoolkit.Daemon1";

const PROFILE_FILE: &str = "sys/class/platform-profile/platform-profile-0/profile";
const SPL_FILE: &str =
    "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes/ppt_pl1_spl/current_value";

// ── fake sysfs ──────────────────────────────────────────────────────────────

fn write(root: &Path, rel: &str, val: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, format!("{val}\n")).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel))
        .unwrap_or_else(|e| panic!("reading {rel}: {e}"))
        .trim()
        .to_string()
}

/// The live 82WM values, same tree the legion-hw tests use.
fn fake_82wm(root: &Path) {
    write(
        root,
        "sys/class/dmi/id/product_family",
        "Legion R9000P ARX8",
    );
    write(root, "sys/class/dmi/id/product_name", "82WM");

    let pp = "sys/class/platform-profile/platform-profile-0";
    write(root, &format!("{pp}/name"), "lenovo-wmi-gamezone");
    write(
        root,
        &format!("{pp}/choices"),
        "low-power balanced performance max-power custom",
    );
    write(root, &format!("{pp}/profile"), "balanced");

    let fa = "sys/class/firmware-attributes/lenovo-wmi-other-0/attributes";
    for (dir, cur, def, min, max) in [
        ("ppt_pl1_spl", 70, 70, 50, 115),
        ("ppt_pl2_sppt", 85, 85, 60, 130),
        ("ppt_pl3_fppt", 102, 102, 70, 150),
    ] {
        write(root, &format!("{fa}/{dir}/current_value"), &cur.to_string());
        write(root, &format!("{fa}/{dir}/default_value"), &def.to_string());
        write(root, &format!("{fa}/{dir}/min_value"), &min.to_string());
        write(root, &format!("{fa}/{dir}/max_value"), &max.to_string());
        write(root, &format!("{fa}/{dir}/scalar_increment"), "1");
        write(root, &format!("{fa}/{dir}/display_name"), dir);
    }

    let bat = "sys/class/power_supply/BAT0";
    write(
        root,
        &format!("{bat}/charge_types"),
        "Fast Standard [Long_Life]",
    );
    write(root, &format!("{bat}/capacity"), "78");
    write(root, &format!("{bat}/status"), "Discharging");
    write(root, "sys/class/power_supply/ADP0/type", "Mains");
    write(root, "sys/class/power_supply/ADP0/online", "1");

    let vpc = "sys/bus/platform/devices/VPC2004:00";
    write(root, &format!("{vpc}/fn_lock"), "0");
    write(root, &format!("{vpc}/camera_power"), "1");
    write(root, &format!("{vpc}/usb_charging"), "0");
    write(root, &format!("{vpc}/fan_mode"), "0");

    let led = "sys/class/leds/platform::kbd_backlight";
    write(root, &format!("{led}/brightness"), "1");
    write(root, &format!("{led}/max_brightness"), "2");

    write(root, "sys/devices/system/cpu/cpufreq/boost", "1");
}

// ── daemon harness ──────────────────────────────────────────────────────────

fn daemon_bin() -> PathBuf {
    // The test binary lives in target/<profile>/deps/; legiond is two levels up.
    let mut p = std::env::current_exe().expect("test exe path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("legiond")
}

fn dbus_daemon_available() -> bool {
    Command::new("dbus-daemon")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A private session bus, torn down on drop.
struct PrivateBus {
    child: Child,
    address: String,
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl PrivateBus {
    fn start() -> Self {
        use std::io::{BufRead, BufReader};
        use std::process::Stdio;

        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawning dbus-daemon");

        let stdout = child.stdout.take().expect("dbus-daemon stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("reading the bus address");
        let address = line.trim().to_string();
        assert!(!address.is_empty(), "dbus-daemon printed no address");

        Self { child, address }
    }
}

/// A running daemon on its own private bus, all killed on drop.
struct Daemon {
    child: Child,
    bus: PrivateBus,
    _tmp: TempDir,
    root: PathBuf,
    state_file: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Daemon {
    /// Spawn against a fresh fake 82WM tree on a fresh bus.
    fn spawn() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("sysroot");
        fs::create_dir_all(&root).unwrap();
        fake_82wm(&root);
        let state_file = tmp.path().join("state.json");
        let bus = PrivateBus::start();
        let child = Self::spawn_process(&bus, &root, &state_file);
        Self {
            child,
            bus,
            _tmp: tmp,
            root,
            state_file,
        }
    }

    fn spawn_process(bus: &PrivateBus, root: &Path, state_file: &Path) -> Child {
        Command::new(daemon_bin())
            .arg("--bus")
            .arg("session")
            .arg("--sysfs-root")
            .arg(root)
            .arg("--state-file")
            .arg(state_file)
            .arg("--foreground")
            .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
            .env("RUST_LOG", "warn")
            .spawn()
            .expect("spawning legiond")
    }

    /// Kill and restart the daemon against the same tree, bus and state file.
    fn restart(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Give the bus a moment to release the well-known name.
        std::thread::sleep(Duration::from_millis(200));
        self.child = Self::spawn_process(&self.bus, &self.root, &self.state_file);
    }

    /// Connect to this daemon's bus, waiting for it to claim its name.
    async fn connect(&self) -> zbus::Connection {
        let conn = zbus::connection::Builder::address(self.bus.address.as_str())
            .expect("parsing the private bus address")
            .build()
            .await
            .expect("connecting to the private bus");

        let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if dbus
                .name_has_owner(BUS_NAME.try_into().unwrap())
                .await
                .unwrap_or(false)
            {
                return conn;
            }
            assert!(
                Instant::now() < deadline,
                "legiond did not claim {BUS_NAME} within 10s"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn proxy(conn: &zbus::Connection) -> zbus::Proxy<'static> {
    zbus::Proxy::new(conn, BUS_NAME, OBJECT_PATH, IFACE)
        .await
        .expect("building proxy")
}

// ── tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn set_profile_writes_sysfs_and_emits_properties_changed() {
    if !dbus_daemon_available() {
        eprintln!("skipping: dbus-daemon is not installed");
        return;
    }
    let daemon = Daemon::spawn();
    let conn = daemon.connect().await;
    let p = proxy(&conn).await;

    // Baseline, read back through the interface.
    let current: String = p.get_property("Profile").await.unwrap();
    assert_eq!(current, "balanced");

    // Subscribe before mutating so the signal cannot be missed.
    use futures_util::StreamExt;
    let mut changes = p.receive_property_changed::<String>("Profile").await;

    p.call_method("SetProfile", &("performance")).await.unwrap();

    // The fake sysfs file really was written.
    assert_eq!(read(&daemon.root, PROFILE_FILE), "performance");

    // ...and a PropertiesChanged arrived.
    let change = tokio::time::timeout(Duration::from_secs(5), changes.next())
        .await
        .expect("PropertiesChanged did not arrive within 5s")
        .expect("signal stream ended");
    assert_eq!(change.get().await.unwrap(), "performance");

    assert_eq!(
        p.get_property::<String>("Profile").await.unwrap(),
        "performance"
    );
}

#[tokio::test]
async fn set_ppt_outside_custom_returns_custom_mode_required() {
    if !dbus_daemon_available() {
        eprintln!("skipping: dbus-daemon is not installed");
        return;
    }
    let daemon = Daemon::spawn();
    let conn = daemon.connect().await;
    let p = proxy(&conn).await;

    // The fake starts in `balanced`, so a power-limit write must be refused
    // with the actionable, named error.
    let err = p
        .call_method("SetPpt", &("spl", 90u32))
        .await
        .expect_err("SetPpt should fail outside Custom mode");

    let name = match &err {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        other => panic!("expected a D-Bus method error, got {other:?}"),
    };
    assert_eq!(name, "org.legiontoolkit.Error.CustomModeRequired");

    // The value was not written.
    assert_eq!(read(&daemon.root, SPL_FILE), "70");

    // Switching to Custom makes the same call succeed.
    p.call_method("SetProfile", &("custom")).await.unwrap();
    p.call_method("SetPpt", &("spl", 90u32)).await.unwrap();
    assert_eq!(read(&daemon.root, SPL_FILE), "90");
}

#[tokio::test]
async fn state_is_persisted_and_reapplied_across_a_restart() {
    if !dbus_daemon_available() {
        eprintln!("skipping: dbus-daemon is not installed");
        return;
    }
    let mut daemon = Daemon::spawn();
    {
        let conn = daemon.connect().await;
        let p = proxy(&conn).await;
        p.call_method("SetProfile", &("custom")).await.unwrap();
        p.call_method("SetPpt", &("spl", 100u32)).await.unwrap();
        p.call_method("SetBatteryMode", &("standard"))
            .await
            .unwrap();
    }

    // The state file records what to restore.
    let state = fs::read_to_string(&daemon.state_file).expect("state.json should exist");
    assert!(state.contains("custom"), "state.json: {state}");
    assert!(state.contains("100"), "state.json: {state}");
    assert!(state.contains("Standard"), "state.json: {state}");

    // Simulate the firmware forgetting everything across a reboot.
    write(&daemon.root, PROFILE_FILE, "balanced");
    write(&daemon.root, SPL_FILE, "70");
    write(
        &daemon.root,
        "sys/class/power_supply/BAT0/charge_types",
        "Fast Standard [Long_Life]",
    );

    daemon.restart();
    let conn = daemon.connect().await;
    let p = proxy(&conn).await;
    assert_eq!(p.get_property::<String>("Profile").await.unwrap(), "custom");

    // Profile, power limit and battery mode were all reapplied.
    assert_eq!(read(&daemon.root, PROFILE_FILE), "custom");
    assert_eq!(read(&daemon.root, SPL_FILE), "100");
    assert_eq!(
        read(&daemon.root, "sys/class/power_supply/BAT0/charge_types"),
        "Standard"
    );
}

#[tokio::test]
async fn toggles_and_capabilities_round_trip() {
    if !dbus_daemon_available() {
        eprintln!("skipping: dbus-daemon is not installed");
        return;
    }
    let daemon = Daemon::spawn();
    let conn = daemon.connect().await;
    let p = proxy(&conn).await;

    p.call_method("SetFnLock", &(true)).await.unwrap();
    assert!(p.get_property::<bool>("FnLock").await.unwrap());
    assert_eq!(
        read(&daemon.root, "sys/bus/platform/devices/VPC2004:00/fn_lock"),
        "1"
    );

    p.call_method("SetKbdBacklight", &(2u32)).await.unwrap();
    assert_eq!(p.get_property::<u32>("KbdBacklight").await.unwrap(), 2);

    // Above max_brightness must be rejected as an invalid argument.
    let err = p
        .call_method("SetKbdBacklight", &(5u32))
        .await
        .expect_err("level 5 exceeds max_brightness 2");
    match &err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.to_string(), "org.legiontoolkit.Error.InvalidArgument");
        }
        other => panic!("expected a D-Bus method error, got {other:?}"),
    }

    // Capabilities reflect the fake machine: everything but fan RPM.
    let caps_json: String = p.get_property("Capabilities").await.unwrap();
    let caps: HashMap<String, serde_json::Value> = serde_json::from_str(&caps_json).unwrap();
    assert_eq!(caps["platform_profile"], serde_json::json!(true));
    assert_eq!(caps["ppt"], serde_json::json!(true));
    assert_eq!(caps["charge_types"], serde_json::json!(true));
    assert_eq!(caps["fan_rpm"], serde_json::json!(false));
    assert_eq!(caps["is_legion"], serde_json::json!(true));
}

#[tokio::test]
async fn status_and_get_ppt_return_usable_json() {
    if !dbus_daemon_available() {
        eprintln!("skipping: dbus-daemon is not installed");
        return;
    }
    let daemon = Daemon::spawn();
    let conn = daemon.connect().await;
    let p = proxy(&conn).await;

    let ppt_json: String = p
        .call_method("GetPpt", &())
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let ppt: Vec<serde_json::Value> = serde_json::from_str(&ppt_json).unwrap();
    assert_eq!(ppt.len(), 3);
    let spl = ppt.iter().find(|a| a["key"] == "spl").unwrap();
    assert_eq!(spl["current"], serde_json::json!(70));
    assert_eq!(spl["min"], serde_json::json!(50));
    assert_eq!(spl["max"], serde_json::json!(115));

    let status_json: String = p
        .call_method("Status", &())
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let status: serde_json::Value = serde_json::from_str(&status_json).unwrap();
    assert_eq!(status["profile"], serde_json::json!("balanced"));
    assert_eq!(status["battery_mode"], serde_json::json!("Long_Life"));
    assert_eq!(status["fan_rpm"], serde_json::Value::Null);

    // Unknown properties must not appear as empty strings by accident.
    let choices: Vec<String> = p.get_property("ProfileChoices").await.unwrap();
    assert_eq!(choices.len(), 5);
    assert!(choices.contains(&"max-power".to_string()));
}

#[tokio::test]
async fn unknown_profile_is_an_invalid_argument() {
    if !dbus_daemon_available() {
        eprintln!("skipping: dbus-daemon is not installed");
        return;
    }
    let daemon = Daemon::spawn();
    let conn = daemon.connect().await;
    let p = proxy(&conn).await;

    let err = p
        .call_method("SetProfile", &("ludicrous"))
        .await
        .expect_err("unknown profile should be rejected");
    match &err {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(name.to_string(), "org.legiontoolkit.Error.InvalidArgument");
        }
        other => panic!("expected a D-Bus method error, got {other:?}"),
    }
}

/// Guard against accidental interface renames: the published surface is the
/// project's public API.
#[tokio::test]
async fn interface_exposes_the_documented_members() {
    if !dbus_daemon_available() {
        eprintln!("skipping: dbus-daemon is not installed");
        return;
    }
    let daemon = Daemon::spawn();
    let conn = daemon.connect().await;

    let introspectable = zbus::fdo::IntrospectableProxy::builder(&conn)
        .destination(BUS_NAME)
        .unwrap()
        .path(OBJECT_PATH)
        .unwrap()
        .build()
        .await
        .unwrap();
    let xml = introspectable.introspect().await.unwrap();

    for member in [
        "SetProfile",
        "SetBatteryMode",
        "SetFnLock",
        "SetCameraPower",
        "SetUsbCharging",
        "SetKbdBacklight",
        "SetCpuBoost",
        "SetPpt",
        "GetPpt",
        "Status",
    ] {
        assert!(xml.contains(member), "introspection is missing {member}");
    }
    for prop in [
        "Profile",
        "ProfileChoices",
        "BatteryMode",
        "FnLock",
        "CameraPower",
        "UsbCharging",
        "KbdBacklight",
        "CpuBoost",
        "Capabilities",
    ] {
        assert!(xml.contains(prop), "introspection is missing {prop}");
    }
    assert!(xml.contains(IFACE));

    // Silence the unused-import warning when this is the only test compiled.
    let _ = std::mem::size_of::<OwnedValue>();
}
