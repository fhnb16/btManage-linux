//! The developer console: raw traffic with one device, to find out what it
//! understands. Three ways:
//!
//! - GATT, through BlueZ: the characteristics of the device, read, write,
//!   subscribe. Reading and writing go through `busctl`; notifications
//!   through `bluetoothctl`, which stays on the bus while they last.
//! - RFCOMM by its channel number: serial ports, AT commands.
//! - L2CAP by its PSM: 0x1001 is AAP, Apple's accessory protocol.
//!
//! Everything that goes back and forth is listed in the window, newest first.
//! Outside Linux the window opens as a preview and every action says so.
//!
//! Not run on Linux yet.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use btmanage_core::apple::aap;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bluez::{Characteristic, characteristics};
use crate::btsocket::Socket;
use crate::{ConsoleWindow, Paths, SharedHub, lock, now_ms};

#[cfg(target_os = "linux")]
use crate::bluez::{Notify, connect, gatt_read, gatt_write, objects};
#[cfg(not(target_os = "linux"))]
use preview::{Notify, connect, gatt_read, gatt_write, objects};

/// The log keeps this many bytes, newest first.
const MAX_OUTPUT: usize = 64 * 1024;
const BASE_UUID_TAIL: &str = "-0000-1000-8000-00805f9b34fb";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    Gatt,
    Rfcomm,
    L2cap,
}

impl Way {
    fn from_index(index: i32) -> Way {
        match index {
            1 => Way::Rfcomm,
            2 => Way::L2cap,
            _ => Way::Gatt,
        }
    }
}

/// A ready command: it fills the fields, and nothing is sent until asked.
struct Template {
    label: &'static str,
    way: Way,
    target: &'static str,
    data: String,
    hex: bool,
}

fn templates() -> Vec<Template> {
    let gatt = |label, target| Template {
        label,
        way: Way::Gatt,
        target,
        data: String::new(),
        hex: false,
    };
    let serial = |label, data: &str| Template {
        label,
        way: Way::Rfcomm,
        target: "1",
        data: data.to_owned(),
        hex: false,
    };
    let accessory = |label, data: &[u8]| Template {
        label,
        way: Way::L2cap,
        target: "0x1001",
        data: hex(data),
        hex: true,
    };
    vec![
        gatt("Battery level, 2A19 · read", "2A19"),
        gatt("Battery level changes, 2A19 · subscribe", "2A19"),
        gatt("Device name, 2A00 · read", "2A00"),
        gatt("Manufacturer, 2A29 · read", "2A29"),
        gatt("Model, 2A24 · read", "2A24"),
        gatt("Firmware, 2A26 · read", "2A26"),
        gatt("Serial number, 2A25 · read", "2A25"),
        gatt("Heart rate, 2A37 · subscribe", "2A37"),
        gatt(
            "Nordic UART: listen · subscribe",
            "6e400003-b5a3-f393-e0a9-e50e24dcca9e",
        ),
        Template {
            label: "Nordic UART: Battery; (Lovense) · send",
            way: Way::Gatt,
            target: "6e400002-b5a3-f393-e0a9-e50e24dcca9e",
            data: "Battery;".to_owned(),
            hex: false,
        },
        serial("AT", "AT"),
        serial("AT+VERSION?", "AT+VERSION?"),
        serial("AT+NAME?", "AT+NAME?"),
        serial("AT+CBC, battery", "AT+CBC"),
        accessory("AAP: hello", &aap::HELLO),
        accessory("AAP: features", &aap::FEATURES),
        accessory("AAP: ask for notifications", &aap::NOTIFICATIONS),
        accessory(
            "AAP: noise cancellation",
            &aap::noise_command(aap::NoiseMode::NoiseCancellation),
        ),
        accessory(
            "AAP: transparency",
            &aap::noise_command(aap::NoiseMode::Transparency),
        ),
        accessory(
            "AAP: adaptive",
            &aap::noise_command(aap::NoiseMode::Adaptive),
        ),
        accessory("AAP: noise control off", &aap::noise_command(aap::NoiseMode::Off)),
    ]
}

/// What is open to the device.
#[derive(Default)]
struct Session {
    characteristics: Vec<Characteristic>,
    socket: Option<Arc<Socket>>,
    notify: Vec<Notify>,
}

/// A device on the list.
#[derive(Clone)]
struct Row {
    id: String,
    name: String,
    address: Option<u64>,
    /// BlueZ's object path, for GATT.
    path: Option<String>,
}

/// Writes to the window from any thread.
#[derive(Clone)]
struct Log {
    window: slint::Weak<ConsoleWindow>,
    started: Instant,
}

impl Log {
    fn line(&self, text: impl Into<String>) {
        let text: String = text.into();
        let line = format!("{:>9.3}  {text}", self.started.elapsed().as_secs_f64());
        let _ = self.window.upgrade_in_event_loop(move |window| {
            let mut output = format!("{line}\n{}", window.get_output());
            if output.len() > MAX_OUTPUT {
                let mut cut = MAX_OUTPUT;
                while !output.is_char_boundary(cut) {
                    cut -= 1;
                }
                output.truncate(cut);
            }
            window.set_output(output.into());
        });
    }

    fn status(&self, text: impl Into<String>) {
        let text: String = text.into();
        let text = SharedString::from(text);
        let _ = self
            .window
            .upgrade_in_event_loop(move |window| window.set_status(text));
    }

    fn characteristics(&self, list: &[Characteristic]) {
        let labels: Vec<SharedString> = list
            .iter()
            .map(|item| format!("{} · {}", short(&item.uuid), item.flags.join(", ")).into())
            .collect();
        let _ = self.window.upgrade_in_event_loop(move |window| {
            window.set_characteristics(ModelRc::new(VecModel::from(labels)));
        });
    }
}

struct Console {
    window: slint::Weak<ConsoleWindow>,
    log: Log,
    session: Arc<Mutex<Session>>,
    devices: RefCell<Vec<Row>>,
    templates: Vec<Template>,
    hub: SharedHub,
    paths: Paths,
}

/// Opens the console, or brings the one open to the front.
pub fn open(holder: &Rc<RefCell<Option<ConsoleWindow>>>, hub: &SharedHub, paths: &Paths) {
    if let Some(window) = holder.borrow().as_ref() {
        let _ = window.show();
        return;
    }
    let window = match ConsoleWindow::new() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("btmanage: no console window: {error}");
            return;
        }
    };
    let console = Rc::new(Console {
        window: window.as_weak(),
        log: Log {
            window: window.as_weak(),
            started: Instant::now(),
        },
        session: Arc::default(),
        devices: RefCell::default(),
        templates: templates(),
        hub: Arc::clone(hub),
        paths: Arc::clone(paths),
    });
    console.fill_devices(&window);
    console.show_way(&window);
    window.set_status("Not connected".into());

    let wire = |handler: fn(&Console, &ConsoleWindow)| {
        let console = Rc::clone(&console);
        move || {
            if let Some(window) = console.window.upgrade() {
                handler(&console, &window);
            }
        }
    };
    window.on_connect(wire(Console::connect));
    window.on_disconnect(wire(Console::disconnect));
    window.on_send(wire(Console::send));
    window.on_read(wire(|console, window| console.gatt(window, false)));
    window.on_subscribe(wire(|console, window| console.gatt(window, true)));
    window.on_clear(wire(|_, window| window.set_output(SharedString::new())));
    window.on_refresh_devices(wire(Console::fill_devices));
    window.on_way_changed(wire_index(&console, |console, window, _| console.show_way(window)));
    window.on_template_picked(wire_index(&console, Console::apply_template));
    window.on_characteristic_picked(wire_index(&console, |console, window, index| {
        let session = lock(&console.session);
        if let Some(item) = usize::try_from(index)
            .ok()
            .and_then(|index| session.characteristics.get(index))
        {
            window.set_target(short(&item.uuid).into());
        }
    }));

    let closing = Rc::clone(&console);
    window.window().on_close_requested(move || {
        closing.close_session();
        slint::CloseRequestResponse::HideWindow
    });

    let _ = window.show();
    *holder.borrow_mut() = Some(window);
}

fn wire_index(
    console: &Rc<Console>,
    handler: fn(&Console, &ConsoleWindow, i32),
) -> impl Fn(i32) + 'static {
    let console = Rc::clone(console);
    move |index| {
        if let Some(window) = console.window.upgrade() {
            handler(&console, &window, index);
        }
    }
}

impl Console {
    /// The devices the hub knows with an address, the connected first.
    fn fill_devices(&self, window: &ConsoleWindow) {
        let snapshot = lock(&self.hub).snapshot(now_ms());
        let paths = lock(&self.paths).clone();
        let mut rows: Vec<(bool, Row)> = snapshot
            .devices
            .iter()
            .filter_map(|device| {
                let text = device.address.as_deref()?;
                let address = u64::from_str_radix(&text.replace(':', ""), 16).ok();
                let name = if device.name.is_empty() {
                    device.model.unwrap_or("Unnamed").to_owned()
                } else {
                    device.name.clone()
                };
                Some((
                    device.connected,
                    Row {
                        id: device.id.clone(),
                        name: format!("{name}  {text}"),
                        address,
                        path: paths.get(&device.id).cloned(),
                    },
                ))
            })
            .collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        let rows: Vec<Row> = rows.into_iter().map(|(_, row)| row).collect();

        let selected = self.selected(window).map(|row| row.id);
        let index = selected
            .and_then(|id| rows.iter().position(|row| row.id == id))
            .unwrap_or(0);
        let labels: Vec<SharedString> = rows.iter().map(|row| row.name.clone().into()).collect();
        window.set_devices(ModelRc::new(VecModel::from(labels)));
        window.set_device_index(i32::try_from(index).unwrap_or(0));
        *self.devices.borrow_mut() = rows;
    }

    fn selected(&self, window: &ConsoleWindow) -> Option<Row> {
        let index = usize::try_from(window.get_device_index()).ok()?;
        self.devices.borrow().get(index).cloned()
    }

    fn way_templates(&self, way: Way) -> Vec<&Template> {
        self.templates.iter().filter(|item| item.way == way).collect()
    }

    /// The fields for the way chosen: its ready commands, and where it goes.
    fn show_way(&self, window: &ConsoleWindow) {
        let way = Way::from_index(window.get_way_index());
        let mut labels: Vec<SharedString> = vec!["None".into()];
        labels.extend(self.way_templates(way).iter().map(|item| SharedString::from(item.label)));
        window.set_templates(ModelRc::new(VecModel::from(labels)));
        window.set_template_index(0);
        window.set_target(
            match way {
                Way::Gatt => "",
                Way::Rfcomm => "1",
                Way::L2cap => "0x1001",
            }
            .into(),
        );
        window.set_hex(way == Way::L2cap);
        window.set_ending_index(if way == Way::Rfcomm { 2 } else { 0 });
    }

    fn apply_template(&self, window: &ConsoleWindow, index: i32) {
        let way = Way::from_index(window.get_way_index());
        let templates = self.way_templates(way);
        let Some(chosen) = usize::try_from(index - 1)
            .ok()
            .and_then(|index| templates.get(index))
        else {
            return;
        };
        window.set_target(chosen.target.into());
        window.set_data(chosen.data.as_str().into());
        window.set_hex(chosen.hex);
    }

    fn connect(&self, window: &ConsoleWindow) {
        let Some(row) = self.selected(window) else {
            self.log.line("Pick a device first");
            return;
        };
        self.close_session();
        let way = Way::from_index(window.get_way_index());
        let target = window.get_target().to_string();
        let log = self.log.clone();
        let session = Arc::clone(&self.session);

        if way == Way::Gatt {
            let Some(path) = row.path else {
                log.line("BlueZ has no record of this device");
                return;
            };
            std::thread::spawn(move || {
                log.line(format!("Connecting to {}", row.name));
                if !connect(&path, true) {
                    log.line("BlueZ did not connect it; looking for its characteristics anyway");
                }
                // The characteristics appear once BlueZ has resolved the services.
                for _ in 0..10 {
                    let list = objects()
                        .map(|answer| characteristics(&answer, &path))
                        .unwrap_or_default();
                    if !list.is_empty() {
                        log.line(format!("{} characteristics", list.len()));
                        log.characteristics(&list);
                        log.status(format!("GATT · {}", row.name));
                        lock(&session).characteristics = list;
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                log.line("No characteristics: the device offers none, or BlueZ has not resolved them");
            });
            return;
        }

        let Some(address) = row.address else {
            log.line("This device has no address");
            return;
        };
        let Some(number) = number(&target) else {
            log.line(if way == Way::Rfcomm {
                "Channel: a number from 1 to 30"
            } else {
                "PSM: a number such as 0x1001"
            });
            return;
        };
        std::thread::spawn(move || {
            log.line(format!("Opening {target} on {}", row.name));
            let opened = match (way, u8::try_from(number), u16::try_from(number)) {
                (Way::Rfcomm, Ok(channel), _) => Socket::rfcomm(address, channel),
                (Way::L2cap, _, Ok(psm)) => Socket::l2cap(address, psm),
                _ => {
                    log.line("The number is out of range");
                    return;
                }
            };
            let socket = match opened {
                Ok(socket) => Arc::new(socket),
                Err(error) => {
                    log.line(format!("Not opened: {error}"));
                    return;
                }
            };
            lock(&session).socket = Some(Arc::clone(&socket));
            log.line("Open");
            log.status(format!("{target} · {}", row.name));
            let mut buffer = [0u8; 4096];
            while let Ok(length) = socket.recv(&mut buffer) {
                if length == 0 {
                    break;
                }
                log.line(format!("<- {}", describe(&buffer[..length])));
            }
            let mut session = lock(&session);
            if session
                .socket
                .as_ref()
                .is_some_and(|open| Arc::ptr_eq(open, &socket))
            {
                session.socket = None;
                log.line("Closed by the device");
                log.status("Not connected");
            }
        });
    }

    fn disconnect(&self, _window: &ConsoleWindow) {
        self.close_session();
        self.log.line("Closed");
    }

    /// Closes the channel and the notifications. The device itself stays connected.
    fn close_session(&self) {
        let mut session = lock(&self.session);
        if let Some(socket) = session.socket.take() {
            socket.shutdown();
        }
        session.notify.clear();
        session.characteristics.clear();
        drop(session);
        self.log.characteristics(&[]);
        self.log.status("Not connected");
    }

    /// The characteristic named in the field, among those of the device.
    fn characteristic(&self, window: &ConsoleWindow) -> Option<String> {
        let Some(uuid) = full_uuid(&window.get_target()) else {
            self.log.line("Characteristic: four hex digits such as 2A19, or a full UUID");
            return None;
        };
        let path = lock(&self.session)
            .characteristics
            .iter()
            .find(|item| item.uuid == uuid)
            .map(|item| item.path.clone());
        if path.is_none() {
            self.log.line(format!(
                "The device has no characteristic {}; connect over GATT first",
                short(&uuid)
            ));
        }
        path
    }

    fn send(&self, window: &ConsoleWindow) {
        let data = match payload(
            &window.get_data(),
            window.get_hex(),
            window.get_ending_index(),
        ) {
            Ok(data) => data,
            Err(problem) => {
                self.log.line(problem);
                return;
            }
        };
        let log = self.log.clone();
        if Way::from_index(window.get_way_index()) == Way::Gatt {
            let Some(path) = self.characteristic(window) else {
                return;
            };
            std::thread::spawn(move || match gatt_write(&path, &data) {
                Ok(()) => log.line(format!("-> {}", hex(&data))),
                Err(error) => log.line(format!("-> {}: {error}", hex(&data))),
            });
            return;
        }
        let Some(socket) = lock(&self.session).socket.clone() else {
            log.line("Not open: connect first");
            return;
        };
        std::thread::spawn(move || match socket.send(&data) {
            Ok(()) => log.line(format!("-> {}", hex(&data))),
            Err(error) => log.line(format!("-> {}: {error}", hex(&data))),
        });
    }

    /// Reads a characteristic, or subscribes to its changes.
    fn gatt(&self, window: &ConsoleWindow, subscribe: bool) {
        let Some(path) = self.characteristic(window) else {
            return;
        };
        let name = short(&full_uuid(&window.get_target()).unwrap_or_default());
        let log = self.log.clone();
        if subscribe {
            let lines = log.clone();
            match Notify::start(&path, move |line| lines.line(format!("bluetoothctl: {line}"))) {
                Ok(notify) => {
                    lock(&self.session).notify.push(notify);
                    log.line(format!("Subscribed to {name}"));
                }
                Err(error) => log.line(format!("Not subscribed: {error}")),
            }
            return;
        }
        std::thread::spawn(move || match gatt_read(&path) {
            Ok(data) => log.line(format!("{name}: {}", describe(&data))),
            Err(error) => log.line(format!("{name}: {error}")),
        });
    }
}

/// The full UUID in lower case, from "2A19" or a whole UUID.
fn full_uuid(text: &str) -> Option<String> {
    let text = text.trim().to_lowercase();
    if text.len() == 4 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("0000{text}{BASE_UUID_TAIL}"));
    }
    let digits: String = text.chars().filter(|c| *c != '-').collect();
    (text.len() == 36 && digits.len() == 32 && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(text)
}

/// "2A19" for a UUID on the Bluetooth base, the whole UUID for any other.
fn short(uuid: &str) -> String {
    match uuid.strip_prefix("0000").and_then(|rest| rest.strip_suffix(BASE_UUID_TAIL)) {
        Some(short) if short.len() == 4 => short.to_uppercase(),
        _ => uuid.to_owned(),
    }
}

/// "0x1001" or "4097"; a channel or a PSM.
fn number(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => u32::from_str_radix(digits, 16).ok(),
        None => text.parse().ok(),
    }
}

/// "04 00 1A".
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bytes from hexadecimal: "04 00 0f", "04000F", "0x04,0x00".
fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let digits: String = text
        .replace("0x", "")
        .replace("0X", "")
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != ':')
        .collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok())
        .collect()
}

/// What the data field holds as bytes: hexadecimal, or text with the line
/// ending chosen (0 none, 1 CR, 2 CR LF, 3 LF).
fn payload(text: &str, hex: bool, ending: i32) -> Result<Vec<u8>, &'static str> {
    if hex {
        return parse_hex(text).ok_or("Not hexadecimal: pairs of digits 0-9 and A-F");
    }
    let ending = match ending {
        1 => "\r",
        2 => "\r\n",
        3 => "\n",
        _ => "",
    };
    let bytes = format!("{text}{ending}").into_bytes();
    if bytes.is_empty() {
        Err("Nothing to send")
    } else {
        Ok(bytes)
    }
}

/// Bytes in hexadecimal and, where they read as text, as text.
fn describe(bytes: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if text.trim().is_empty() || text.contains('\u{FFFD}') {
        hex(bytes)
    } else {
        format!("{}  \"{text}\"", hex(bytes))
    }
}

/// Stand-ins for the BlueZ tools elsewhere than on Linux.
#[cfg(not(target_os = "linux"))]
mod preview {
    use serde_json::Value;

    const ONLY: &str = "Only on Linux; this is a preview of the window";

    pub struct Notify;

    impl Notify {
        pub fn start(_path: &str, _line: impl Fn(String) + Send + 'static) -> Result<Notify, String> {
            Err(ONLY.to_owned())
        }
    }

    pub fn connect(_path: &str, _on: bool) -> bool {
        false
    }

    pub fn objects() -> Option<Value> {
        None
    }

    pub fn gatt_read(_path: &str) -> Result<Vec<u8>, String> {
        Err(ONLY.to_owned())
    }

    pub fn gatt_write(_path: &str, _data: &[u8]) -> Result<(), String> {
        Err(ONLY.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_read_and_shortened() {
        assert_eq!(
            full_uuid(" 2a19 ").as_deref(),
            Some("00002a19-0000-1000-8000-00805f9b34fb")
        );
        assert_eq!(
            full_uuid("6E400002-B5A3-F393-E0A9-E50E24DCCA9E").as_deref(),
            Some("6e400002-b5a3-f393-e0a9-e50e24dcca9e")
        );
        assert_eq!(full_uuid("battery"), None);
        assert_eq!(short("00002a19-0000-1000-8000-00805f9b34fb"), "2A19");
        assert_eq!(
            short("6e400002-b5a3-f393-e0a9-e50e24dcca9e"),
            "6e400002-b5a3-f393-e0a9-e50e24dcca9e"
        );
    }

    #[test]
    fn data_is_read() {
        assert_eq!(parse_hex("04 00 0f"), Some(vec![4, 0, 15]));
        assert_eq!(parse_hex("0x04,0x00"), Some(vec![4, 0]));
        assert_eq!(parse_hex("040"), None);
        assert_eq!(parse_hex("zz"), None);
        assert_eq!(payload("AT", false, 2), Ok(b"AT\r\n".to_vec()));
        assert_eq!(payload("", false, 0), Err("Nothing to send"));
        assert_eq!(number("0x1001"), Some(0x1001));
        assert_eq!(number("3"), Some(3));
        assert_eq!(number("x"), None);
    }

    #[test]
    fn bytes_are_described() {
        assert_eq!(hex(&[4, 0, 0x1A]), "04 00 1A");
        assert_eq!(describe(b"OK\r\n"), "4F 4B 0D 0A  \"OK\"");
        assert_eq!(describe(&[0, 1]), "00 01");
    }

    #[test]
    fn every_way_has_templates() {
        let all = templates();
        for way in [Way::Gatt, Way::Rfcomm, Way::L2cap] {
            assert!(all.iter().any(|item| item.way == way));
        }
        let hello = all.iter().find(|item| item.label == "AAP: hello").unwrap();
        assert_eq!(parse_hex(&hello.data), Some(aap::HELLO.to_vec()));
    }
}
