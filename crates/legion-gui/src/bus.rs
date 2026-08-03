//! The D-Bus client side of the GUI.
//!
//! All hardware state comes from `legiond`; the GUI never writes sysfs. D-Bus
//! work happens on a tokio task and results reach the UI through a channel —
//! Slint's types are not `Send`, and blocking the UI thread on a bus call
//! would freeze the window.

use legion_hw::Capabilities;
use legion_hw::firmware_attrs::AttrInfo;
use zbus::Connection;

pub const BUS_NAME: &str = "org.legiontoolkit.Daemon";
pub const OBJECT_PATH: &str = "/org/legiontoolkit/Daemon";
pub const IFACE: &str = "org.legiontoolkit.Daemon1";

/// A snapshot of everything the UI displays, fetched in one pass.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub profile: String,
    pub profile_choices: Vec<String>,
    pub battery_mode: String,
    pub fn_lock: bool,
    pub camera: bool,
    pub usb_charging: bool,
    pub backlight: u32,
    pub boost: bool,
    pub capabilities: Capabilities,
    pub ppt: Vec<AttrInfo>,
    pub daemon_version: String,
}

/// A request from the UI to the daemon.
#[derive(Debug, Clone)]
pub enum Request {
    SetProfile(String),
    SetBatteryMode(String),
    SetPpt(String, u32),
    SetFnLock(bool),
    SetCameraPower(bool),
    SetUsbCharging(bool),
    SetKbdBacklight(u32),
    SetCpuBoost(bool),
    Refresh,
}

/// Something the UI needs to react to.
#[derive(Debug, Clone)]
pub enum Event {
    /// Fresh state.
    Snapshot(Box<Snapshot>),
    /// A user-facing message for the toast.
    Toast(String),
    /// The daemon vanished (or never appeared).
    Disconnected(String),
}

/// Connected proxy to the daemon.
pub struct Daemon {
    proxy: zbus::Proxy<'static>,
}

impl Daemon {
    /// Connect, failing when the daemon is not on the bus so the GUI can show
    /// its "legiond is not running" notice instead of a dead dashboard.
    pub async fn connect() -> Result<Self, String> {
        let conn = Connection::system()
            .await
            .map_err(|e| format!("cannot reach the system bus: {e}"))?;

        let dbus = zbus::fdo::DBusProxy::new(&conn)
            .await
            .map_err(|e| e.to_string())?;
        let owned = dbus
            .name_has_owner(
                BUS_NAME
                    .try_into()
                    .map_err(|_| "bad bus name".to_string())?,
            )
            .await
            .unwrap_or(false);
        if !owned {
            return Err(format!("{BUS_NAME} is not owned by any process"));
        }

        let proxy = zbus::Proxy::new(&conn, BUS_NAME, OBJECT_PATH, IFACE)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self { proxy })
    }

    pub fn proxy(&self) -> &zbus::Proxy<'static> {
        &self.proxy
    }

    /// Read every property the UI needs.
    pub async fn snapshot(&self) -> Result<Snapshot, String> {
        let caps_json: String = self
            .proxy
            .get_property("Capabilities")
            .await
            .map_err(|e| e.to_string())?;
        let capabilities = Capabilities::from_json(&caps_json).unwrap_or_default();

        let ppt_json: String = self
            .proxy
            .call_method("GetPpt", &())
            .await
            .map_err(|e| e.to_string())?
            .body()
            .deserialize()
            .map_err(|e| e.to_string())?;
        let ppt: Vec<AttrInfo> = serde_json::from_str(&ppt_json).unwrap_or_default();

        Ok(Snapshot {
            profile: self.proxy.get_property("Profile").await.unwrap_or_default(),
            profile_choices: self
                .proxy
                .get_property("ProfileChoices")
                .await
                .unwrap_or_default(),
            battery_mode: self
                .proxy
                .get_property("BatteryMode")
                .await
                .unwrap_or_default(),
            fn_lock: self.proxy.get_property("FnLock").await.unwrap_or(false),
            camera: self
                .proxy
                .get_property("CameraPower")
                .await
                .unwrap_or(false),
            usb_charging: self
                .proxy
                .get_property("UsbCharging")
                .await
                .unwrap_or(false),
            backlight: self.proxy.get_property("KbdBacklight").await.unwrap_or(0),
            boost: self.proxy.get_property("CpuBoost").await.unwrap_or(false),
            daemon_version: self.proxy.get_property("Version").await.unwrap_or_default(),
            capabilities,
            ppt,
        })
    }

    /// Dispatch a UI request. Returns the message to toast, if any.
    pub async fn apply(&self, req: Request) -> Result<Option<String>, String> {
        let call = match req {
            Request::Refresh => return Ok(None),
            Request::SetProfile(v) => self.proxy.call_method("SetProfile", &(v.as_str())).await,
            Request::SetBatteryMode(v) => {
                self.proxy
                    .call_method("SetBatteryMode", &(v.as_str()))
                    .await
            }
            Request::SetPpt(attr, watts) => {
                self.proxy
                    .call_method("SetPpt", &(attr.as_str(), watts))
                    .await
            }
            Request::SetFnLock(v) => self.proxy.call_method("SetFnLock", &(v)).await,
            Request::SetCameraPower(v) => self.proxy.call_method("SetCameraPower", &(v)).await,
            Request::SetUsbCharging(v) => self.proxy.call_method("SetUsbCharging", &(v)).await,
            Request::SetKbdBacklight(v) => self.proxy.call_method("SetKbdBacklight", &(v)).await,
            Request::SetCpuBoost(v) => self.proxy.call_method("SetCpuBoost", &(v)).await,
        };

        match call {
            Ok(_) => Ok(None),
            Err(e) => Ok(Some(describe(&e))),
        }
    }
}

/// Turn a D-Bus error into the sentence shown in the toast.
pub fn describe(e: &zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, detail, _) => match name.as_str() {
            "org.legiontoolkit.Error.CustomModeRequired" => {
                "Switch to Custom mode first".to_string()
            }
            "org.legiontoolkit.Error.NotAuthorized" => {
                "Not authorized — an active local session is required".to_string()
            }
            "org.legiontoolkit.Error.NotSupported" => "Not supported on this machine".to_string(),
            _ => detail.clone().unwrap_or_else(|| name.as_str().to_string()),
        },
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method_error(name: &str) -> zbus::Error {
        zbus::Error::MethodError(
            name.try_into().unwrap(),
            Some("raw detail".into()),
            zbus::message::Message::method_call("/x", "Y")
                .unwrap()
                .build(&())
                .unwrap(),
        )
    }

    #[test]
    fn custom_mode_error_is_the_documented_toast() {
        assert_eq!(
            describe(&method_error("org.legiontoolkit.Error.CustomModeRequired")),
            "Switch to Custom mode first"
        );
    }

    #[test]
    fn authorization_failure_is_explained() {
        let msg = describe(&method_error("org.legiontoolkit.Error.NotAuthorized"));
        assert!(msg.contains("active local session"), "{msg}");
    }

    #[test]
    fn unknown_errors_fall_back_to_the_detail_text() {
        assert_eq!(
            describe(&method_error("org.example.Whatever")),
            "raw detail"
        );
    }

    #[test]
    fn bus_constants_match_the_daemon() {
        assert_eq!(BUS_NAME, "org.legiontoolkit.Daemon");
        assert_eq!(OBJECT_PATH, "/org/legiontoolkit/Daemon");
        assert_eq!(IFACE, "org.legiontoolkit.Daemon1");
    }
}
