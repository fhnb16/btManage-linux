//! BlueZ, the Bluetooth stack of Linux, reached through its D-Bus interface.
//!
//! The system's own tools do the talking: `busctl` reads every Bluetooth
//! object BlueZ keeps (paired devices, their battery, the advertisements it
//! heard), and `bluetoothctl` keeps a discovery running so that
//! advertisements keep coming. That needs no D-Bus library in this
//! application, at the cost of a process per reading.
//!
//! The reading of `busctl --json` is plain Rust and is tested everywhere; the
//! processes run on Linux only.

// Elsewhere only the tests use the reading.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::HashMap;

use btmanage_core::hub::{Advertisement, Hub, SystemDevice, Transport};
use serde_json::Value;

/// Apple's company identifier, under which AirPods send their levels.
const APPLE: u16 = 0x004C;
/// The 16-bit UUID of Fast Pair, under which other earbuds send theirs.
const FAST_PAIR: u16 = 0xFE2C;
const BASE_UUID_TAIL: &str = "-0000-1000-8000-00805f9b34fb";

/// One device as BlueZ describes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Found {
    /// D-Bus object path, for connecting and removing.
    pub path: String,
    pub device: SystemDevice,
    pub paired: bool,
    /// Signal strength, while the device is heard in a discovery.
    pub rssi: Option<i16>,
    /// Apple manufacturer data, and Fast Pair service data, as last heard.
    pub apple: Option<Vec<u8>>,
    pub fast_pair: Option<Vec<u8>>,
    /// An Apple accessory: it offers the service of Apple's accessory
    /// protocol, or its maker is Apple.
    pub accessory: bool,
}

/// The service AirPods and Beats offer for Apple's accessory protocol (AAP).
const ACCESSORY_UUID: &str = "74ec2172-0bad-4d01-8f77-997b2be0722a";

/// One GATT characteristic of a device.
#[derive(Debug, Clone, PartialEq)]
pub struct Characteristic {
    /// D-Bus object path, for reading and writing it.
    pub path: String,
    /// The full UUID, in lower case.
    pub uuid: String,
    /// What it allows: "read", "write", "notify" and so on.
    pub flags: Vec<String>,
}

/// The GATT characteristics BlueZ lists under a device, from the same answer
/// as [`parse`], in the order of their paths.
pub fn characteristics(answer: &Value, device_path: &str) -> Vec<Characteristic> {
    let Some(objects) = answer
        .get("data")
        .and_then(|data| data.get(0))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let prefix = format!("{device_path}/");
    let mut found: Vec<Characteristic> = objects
        .iter()
        .filter(|(path, _)| path.starts_with(&prefix))
        .filter_map(|(path, interfaces)| {
            let properties = interfaces.get("org.bluez.GattCharacteristic1")?;
            let uuid = property(properties, "UUID")?.as_str()?.to_lowercase();
            let flags = property(properties, "Flags")
                .and_then(Value::as_array)
                .map(|flags| {
                    flags
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Some(Characteristic {
                path: path.clone(),
                uuid,
                flags,
            })
        })
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

/// The bytes of the answer of `ReadValue`: `{"type": "ay", "data": [[…]]}`.
pub fn read_answer(answer: &Value) -> Option<Vec<u8>> {
    answer
        .get("data")?
        .get(0)?
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

/// Reads the answer of `busctl --json=short call org.bluez /
/// org.freedesktop.DBus.ObjectManager GetManagedObjects`.
pub fn parse(answer: &Value) -> Vec<Found> {
    let Some(objects) = answer
        .get("data")
        .and_then(|data| data.get(0))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut found: Vec<Found> = objects
        .iter()
        .filter_map(|(path, interfaces)| device(path, interfaces))
        .collect();
    found.sort_by(|a, b| a.device.name.cmp(&b.device.name));
    found
}

/// The value of a property: `{"type": "s", "data": "…"}` gives the data.
fn property<'a>(properties: &'a Value, name: &str) -> Option<&'a Value> {
    properties.get(name)?.get("data")
}

fn device(path: &str, interfaces: &Value) -> Option<Found> {
    let properties = interfaces.get("org.bluez.Device1")?;
    let address_text = property(properties, "Address")?.as_str()?;
    let address = u64::from_str_radix(&address_text.replace(':', ""), 16).ok()?;
    let name = property(properties, "Alias")
        .or_else(|| property(properties, "Name"))
        .and_then(Value::as_str)
        .unwrap_or(address_text)
        .to_owned();
    let flag = |key: &str| property(properties, key).and_then(Value::as_bool) == Some(true);
    let class = property(properties, "Class")
        .and_then(Value::as_u64)
        .and_then(|class| u32::try_from(class).ok());
    let appearance = property(properties, "Appearance")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok());
    let transport = match (class.is_some(), appearance.is_some()) {
        (true, true) => Transport::Dual,
        (true, false) => Transport::Classic,
        _ => Transport::Le,
    };
    let uuids: Vec<&str> = property(properties, "UUIDs")
        .and_then(Value::as_array)
        .map(|uuids| uuids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let services = uuids.iter().copied().filter_map(short_uuid).collect();
    let (vendor_source, vendor_id, product_id) = property(properties, "Modalias")
        .and_then(Value::as_str)
        .and_then(modalias)
        .map_or((None, None, None), |(source, vendor, product)| {
            (Some(source), Some(vendor), Some(product))
        });
    let battery = interfaces
        .get("org.bluez.Battery1")
        .and_then(|battery| property(battery, "Percentage"))
        .and_then(Value::as_u64)
        .and_then(|percent| u8::try_from(percent).ok());
    let rssi = property(properties, "RSSI")
        .and_then(Value::as_i64)
        .and_then(|rssi| i16::try_from(rssi).ok());

    let apple = property(properties, "ManufacturerData")
        .and_then(|data| data.get(APPLE.to_string()))
        .and_then(bytes);
    let fast_pair = property(properties, "ServiceData").and_then(|data| {
        let key = format!("0000{FAST_PAIR:04x}{BASE_UUID_TAIL}");
        data.get(key).and_then(bytes)
    });
    let accessory = uuids
        .iter()
        .any(|uuid| uuid.eq_ignore_ascii_case(ACCESSORY_UUID))
        || (vendor_source == Some(1) && vendor_id == Some(APPLE));

    Some(Found {
        path: path.to_owned(),
        device: SystemDevice {
            id: address_text.to_owned(),
            name,
            address: Some(address),
            transport: Some(transport),
            connected: flag("Connected"),
            class_of_device: class,
            appearance,
            vendor_source,
            vendor_id,
            product_id,
            services,
            battery,
            last_connected_ms: None,
        },
        paired: flag("Paired"),
        rssi,
        apple,
        fast_pair,
        accessory,
    })
}

/// A variant holding bytes: `{"type": "ay", "data": [7, 25, …]}`.
fn bytes(variant: &Value) -> Option<Vec<u8>> {
    variant
        .get("data")?
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

/// The 16-bit form of a UUID on the Bluetooth base.
fn short_uuid(uuid: &str) -> Option<u16> {
    let uuid = uuid.to_lowercase();
    if !uuid.starts_with("0000") || !uuid.ends_with(BASE_UUID_TAIL) {
        return None;
    }
    u16::from_str_radix(uuid.get(4..8)?, 16).ok()
}

/// `bluetooth:v004Cp200Ed0100` gives source 1 (the Bluetooth SIG), vendor and
/// product; `usb:v…` gives source 2.
fn modalias(text: &str) -> Option<(u8, u16, u16)> {
    let (kind, rest) = text.split_once(':')?;
    let source = match kind {
        "bluetooth" => 1,
        "usb" => 2,
        _ => return None,
    };
    let vendor = u16::from_str_radix(rest.get(1..5)?, 16).ok()?;
    let product = u16::from_str_radix(rest.get(6..10)?, 16).ok()?;
    Some((source, vendor, product))
}

/// Feeds a reading to the hub: the paired devices, and the advertisements
/// that changed since the last reading. BlueZ keeps the last advertisement of
/// a device after it went quiet; feeding only changes keeps such a device from
/// looking nearby.
pub fn feed(hub: &mut Hub, found: &[Found], seen: &mut HashMap<String, (i16, Vec<u8>)>, now: u64) {
    let paired: Vec<SystemDevice> = found
        .iter()
        .filter(|item| item.paired)
        .map(|item| item.device.clone())
        .collect();
    hub.set_system_devices(paired, now);

    for item in found {
        let (Some(rssi), Some(address)) = (item.rssi, item.device.address) else {
            continue;
        };
        let adverts = [
            (APPLE, true, item.apple.as_ref()),
            (FAST_PAIR, false, item.fast_pair.as_ref()),
        ];
        for (id, manufacturer, data) in adverts {
            let Some(data) = data else {
                continue;
            };
            let key = format!("{}/{id}", item.device.id);
            if seen.get(&key) == Some(&(rssi, data.clone())) {
                continue;
            }
            seen.insert(key, (rssi, data.clone()));
            let advertisement = if manufacturer {
                Advertisement::Manufacturer { company_id: id, data }
            } else {
                Advertisement::ServiceData { uuid: id, data }
            };
            hub.advertisement(address, rssi, advertisement, now);
        }
    }
}

/// A line `bluetoothctl` printed, without its colours and prompt.
fn plain(line: &str) -> String {
    let mut text = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            // An escape sequence: ESC, '[', parameters, then a letter.
            '\u{1b}' => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            // The prompt marks: \x01 and \x02 around invisible parts.
            '\u{1}' | '\u{2}' | '\r' => {}
            _ => text.push(c),
        }
    }
    // "[My device]# " and "[bluetooth]# " before what was printed.
    let text = match text.find("]# ") {
        Some(end) if text.starts_with('[') => &text[end + 3..],
        _ => text.as_str(),
    };
    text.trim().to_owned()
}

#[cfg(target_os = "linux")]
pub use system::{Notify, Scan, connect, gatt_read, gatt_write, objects, read};

/// The processes: the discovery and the reading.
#[cfg(target_os = "linux")]
mod system {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, Command, Stdio};

    use serde_json::Value;

    use super::Found;

    /// A discovery kept running by `bluetoothctl`; it stops when this is dropped.
    pub struct Scan(Option<Child>);

    impl Scan {
        pub fn start() -> Scan {
            let child = Command::new("bluetoothctl")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()
                .and_then(|mut child| {
                    let stdin = child.stdin.as_mut()?;
                    stdin.write_all(b"scan on\n").ok()?;
                    Some(child)
                });
            if child.is_none() {
                eprintln!("btmanage: bluetoothctl did not start; no advertisements");
            }
            Scan(child)
        }
    }

    impl Drop for Scan {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(b"scan off\nquit\n");
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// Everything BlueZ knows now, or nothing when it does not answer.
    pub fn read() -> Vec<Found> {
        objects().map_or_else(Vec::new, |answer| super::parse(&answer))
    }

    /// The answer of `GetManagedObjects`: every object BlueZ keeps.
    pub fn objects() -> Option<Value> {
        let output = Command::new("busctl")
            .args(["--system", "--json=short", "call", "org.bluez", "/"])
            .args(["org.freedesktop.DBus.ObjectManager", "GetManagedObjects"])
            .output()
            .ok()?;
        serde_json::from_slice::<Value>(&output.stdout).ok()
    }

    /// Runs `busctl` with the arguments; its output, or what it said went wrong.
    fn busctl(arguments: &[String]) -> Result<Vec<u8>, String> {
        let output = Command::new("busctl")
            .args(["--system", "--json=short", "call", "org.bluez"])
            .args(arguments)
            .output()
            .map_err(|error| format!("busctl: {error}"))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
    }

    /// Reads a GATT characteristic.
    pub fn gatt_read(path: &str) -> Result<Vec<u8>, String> {
        let output = busctl(&[
            path.to_owned(),
            "org.bluez.GattCharacteristic1".to_owned(),
            "ReadValue".to_owned(),
            "a{sv}".to_owned(),
            "0".to_owned(),
        ])?;
        let answer: Value =
            serde_json::from_slice(&output).map_err(|error| format!("busctl: {error}"))?;
        super::read_answer(&answer).ok_or_else(|| "BlueZ gave no bytes".to_owned())
    }

    /// Writes a GATT characteristic; BlueZ picks the kind of write its flags allow.
    pub fn gatt_write(path: &str, data: &[u8]) -> Result<(), String> {
        let mut arguments = vec![
            path.to_owned(),
            "org.bluez.GattCharacteristic1".to_owned(),
            "WriteValue".to_owned(),
            "aya{sv}".to_owned(),
            data.len().to_string(),
        ];
        arguments.extend(data.iter().map(u8::to_string));
        arguments.push("0".to_owned());
        busctl(&arguments).map(|_| ())
    }

    /// Notifications of a GATT characteristic, through `bluetoothctl`: BlueZ
    /// sends them only while the program that asked stays on the bus, which
    /// `busctl` does not. They stop when this is dropped.
    pub struct Notify(Child);

    impl Notify {
        /// Starts the notifications; every line `bluetoothctl` prints goes to `line`.
        pub fn start(path: &str, line: impl Fn(String) + Send + 'static) -> Result<Notify, String> {
            let mut child = Command::new("bluetoothctl")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| format!("bluetoothctl: {error}"))?;
            let commands = format!("menu gatt\nselect-attribute {path}\nnotify on\n");
            child
                .stdin
                .as_mut()
                .ok_or_else(|| "bluetoothctl: no input".to_owned())?
                .write_all(commands.as_bytes())
                .map_err(|error| format!("bluetoothctl: {error}"))?;
            if let Some(stdout) = child.stdout.take() {
                std::thread::spawn(move || {
                    for text in BufReader::new(stdout).lines().map_while(Result::ok) {
                        let text = super::plain(&text);
                        if !text.is_empty() {
                            line(text);
                        }
                    }
                });
            }
            Ok(Notify(child))
        }
    }

    impl Drop for Notify {
        fn drop(&mut self) {
            if let Some(stdin) = self.0.stdin.as_mut() {
                let _ = stdin.write_all(b"notify off\nquit\n");
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Connects or disconnects a paired device. False when BlueZ refused.
    pub fn connect(path: &str, on: bool) -> bool {
        let method = if on { "Connect" } else { "Disconnect" };
        Command::new("busctl")
            .args(["--system", "call", "org.bluez", path, "org.bluez.Device1", method])
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `busctl --json=short` gives for a pair of AirPods and a keyboard,
    /// cut down to the properties read here. Written from BlueZ's interface
    /// description; not a capture.
    fn sample() -> Value {
        serde_json::json!({
            "type": "a{oa{sa{sv}}}",
            "data": [{
                "/org/bluez/hci0": {
                    "org.bluez.Adapter1": { "Address": { "type": "s", "data": "00:11:22:33:44:55" } }
                },
                "/org/bluez/hci0/dev_80_AF_19_19_B4_77": {
                    "org.bluez.Device1": {
                        "Address": { "type": "s", "data": "80:AF:19:19:B4:77" },
                        "Alias": { "type": "s", "data": "My AirPods" },
                        "Paired": { "type": "b", "data": true },
                        "Connected": { "type": "b", "data": true },
                        "Class": { "type": "u", "data": 2360344 },
                        "UUIDs": { "type": "as", "data": [
                            "0000110b-0000-1000-8000-00805f9b34fb",
                            "74ec2172-0bad-4d01-8f77-997b2be0722a"
                        ] },
                        "Modalias": { "type": "s", "data": "bluetooth:v004Cp201Bd0100" },
                        "RSSI": { "type": "n", "data": -52 },
                        "ManufacturerData": { "type": "a{qv}", "data": {
                            "76": { "type": "ay", "data": [7, 25, 1, 27, 32, 32, 121, 5,
                                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] }
                        } }
                    },
                    "org.bluez.Battery1": { "Percentage": { "type": "y", "data": 80 } }
                },
                "/org/bluez/hci0/dev_DC_2C_26_00_06_8D": {
                    "org.bluez.Device1": {
                        "Address": { "type": "s", "data": "DC:2C:26:00:06:8D" },
                        "Name": { "type": "s", "data": "Keyboard" },
                        "Paired": { "type": "b", "data": true },
                        "Connected": { "type": "b", "data": false },
                        "Appearance": { "type": "q", "data": 961 }
                    }
                }
            }]
        })
    }

    #[test]
    fn devices_are_read() {
        let found = parse(&sample());
        assert_eq!(found.len(), 2);
        let keyboard = &found[0];
        assert_eq!(keyboard.device.name, "Keyboard");
        assert_eq!(keyboard.device.transport, Some(Transport::Le));
        assert_eq!(keyboard.device.appearance, Some(961));

        let pods = &found[1];
        assert_eq!(pods.path, "/org/bluez/hci0/dev_80_AF_19_19_B4_77");
        assert_eq!(pods.device.id, "80:AF:19:19:B4:77");
        assert_eq!(pods.device.address, Some(0x80AF_1919_B477));
        assert!(pods.paired && pods.device.connected);
        assert_eq!(pods.device.services, [0x110B]);
        assert_eq!(pods.device.vendor_id, Some(0x004C));
        assert_eq!(pods.device.product_id, Some(0x201B));
        assert_eq!(pods.device.battery, Some(80));
        assert_eq!(pods.rssi, Some(-52));
        assert_eq!(pods.apple.as_ref().map(Vec::len), Some(27));
    }

    #[test]
    fn levels_reach_the_paired_airpods() {
        let found = parse(&sample());
        let mut hub = Hub::new();
        let mut seen = HashMap::new();
        feed(&mut hub, &found, &mut seen, 1_000);
        let snapshot = hub.snapshot(1_000);
        let pods = snapshot
            .devices
            .iter()
            .find(|device| device.id == "80:AF:19:19:B4:77")
            .unwrap();
        assert_eq!(pods.model, Some("AirPods 4 ANC"));
        assert!(pods.cells.iter().any(|cell| cell.percent == 90));

        // The same advertisement again is not fed again.
        feed(&mut hub, &found, &mut seen, 2_000);
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn apple_accessories_are_told_apart() {
        let found = parse(&sample());
        assert!(!found[0].accessory);
        assert!(found[1].accessory);
    }

    #[test]
    fn characteristics_of_a_device_are_listed() {
        let answer = serde_json::json!({
            "type": "a{oa{sa{sv}}}",
            "data": [{
                "/org/bluez/hci0/dev_DC_2C_26_00_06_8D/service0010/char0011": {
                    "org.bluez.GattCharacteristic1": {
                        "UUID": { "type": "s", "data": "00002A19-0000-1000-8000-00805F9B34FB" },
                        "Flags": { "type": "as", "data": ["read", "notify"] }
                    }
                },
                "/org/bluez/hci0/dev_DC_2C_26_00_06_8D/service0010": {
                    "org.bluez.GattService1": {
                        "UUID": { "type": "s", "data": "0000180f-0000-1000-8000-00805f9b34fb" }
                    }
                },
                "/org/bluez/hci0/dev_11_22_33_44_55_66/service0010/char0011": {
                    "org.bluez.GattCharacteristic1": {
                        "UUID": { "type": "s", "data": "00002a00-0000-1000-8000-00805f9b34fb" }
                    }
                }
            }]
        });
        let found = characteristics(&answer, "/org/bluez/hci0/dev_DC_2C_26_00_06_8D");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].uuid, "00002a19-0000-1000-8000-00805f9b34fb");
        assert_eq!(found[0].flags, ["read", "notify"]);
    }

    #[test]
    fn read_values_and_bluetoothctl_lines_are_read() {
        let answer = serde_json::json!({ "type": "ay", "data": [[100, 7]] });
        assert_eq!(read_answer(&answer), Some(vec![100, 7]));
        assert_eq!(
            plain("\u{1b}[0;94m[My mouse]\u{1b}[0m# [CHG] Attribute Value: 64"),
            "[CHG] Attribute Value: 64"
        );
        assert_eq!(plain("  64 07   d.  "), "64 07   d.");
    }

    #[test]
    fn modalias_and_uuids_are_read() {
        assert_eq!(modalias("usb:v05ACp024Fd0100"), Some((2, 0x05AC, 0x024F)));
        assert_eq!(modalias("nonsense"), None);
        assert_eq!(short_uuid("0000FE2C-0000-1000-8000-00805F9B34FB"), Some(0xFE2C));
        assert_eq!(short_uuid("74ec2172-0bad-4d01-8f77-997b2be0722a"), None);
    }
}
