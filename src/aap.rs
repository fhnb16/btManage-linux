//! AAP, Apple's accessory protocol, to connected AirPods and Beats: an L2CAP
//! channel of Bluetooth Classic over which they report their levels in steps
//! of 1 % and take the noise mode. The core builds and reads the packets
//! (`btmanage_core::apple::aap`); this keeps the channels open and carries
//! them, as the Android application does. Linux opens such a channel for any
//! program, without the workarounds Android needs.
//!
//! Not run on Linux yet.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use btmanage_core::apple::aap;

use crate::bluez::Found;
use crate::btsocket::Socket;
use crate::{SharedHub, lock, now_ms};

/// A channel that failed is tried again after this long, a few times, and
/// then not until the device connects anew.
const RETRY_MS: u64 = 10 * 60 * 1000;
const MAX_TRIES: u32 = 3;
/// Between the packets of the opening.
const PACKET_GAP: Duration = Duration::from_millis(200);

#[derive(Default)]
struct State {
    open: HashMap<String, Arc<Socket>>,
    opening: HashSet<String>,
    /// Failed attempts and when the last was, by device.
    failures: HashMap<String, (u32, u64)>,
}

/// The channels to the connected Apple accessories, by device identifier.
#[derive(Clone, Default)]
pub struct Links(Arc<Mutex<State>>);

/// A device the channel is opened to: an Apple accessory, paired and connected.
fn target(found: &Found) -> Option<(String, u64)> {
    if !(found.accessory && found.paired && found.device.connected) {
        return None;
    }
    Some((found.device.id.clone(), found.device.address?))
}

impl Links {
    /// Opens a channel to every connected Apple accessory that has none, in
    /// the background, and closes those of devices that went away.
    pub fn keep(&self, hub: &SharedHub, found: &[Found]) {
        let wanted: HashMap<String, u64> = found.iter().filter_map(target).collect();
        let now = now_ms();
        let mut state = lock(&self.0);
        state.open.retain(|id, socket| {
            let keep = wanted.contains_key(id);
            if !keep {
                socket.shutdown();
            }
            keep
        });
        // A device that went away starts afresh when it comes back.
        state.failures.retain(|id, _| wanted.contains_key(id));

        for (id, address) in wanted {
            if state.open.contains_key(&id) || state.opening.contains(&id) {
                continue;
            }
            let resting = state.failures.get(&id).is_some_and(|&(tries, at)| {
                tries >= MAX_TRIES || now.saturating_sub(at) < RETRY_MS
            });
            if resting {
                continue;
            }
            state.opening.insert(id.clone());
            let links = self.clone();
            let hub = Arc::clone(hub);
            std::thread::spawn(move || links.open(&hub, &id, address));
        }
    }

    /// Opens the channel, starts reading it, then says hello and asks for
    /// every notification, twice over, as LibrePods does: AirPods do not
    /// always take the opening the first time.
    fn open(&self, hub: &SharedHub, id: &str, address: u64) {
        let socket = match Socket::l2cap(address, aap::PSM) {
            Ok(socket) => Arc::new(socket),
            Err(error) => {
                eprintln!("btmanage: AAP to {id}: {error}");
                self.failed(id);
                return;
            }
        };
        {
            let mut state = lock(&self.0);
            state.opening.remove(id);
            state.failures.remove(id);
            state.open.insert(id.to_owned(), Arc::clone(&socket));
        }

        {
            let links = self.clone();
            let hub = Arc::clone(hub);
            let id = id.to_owned();
            let socket = Arc::clone(&socket);
            std::thread::spawn(move || links.read(&hub, &id, &socket));
        }

        let opening: [&[u8]; 3] = [&aap::HELLO, &aap::FEATURES, &aap::NOTIFICATIONS];
        for packet in opening.iter().chain(opening.iter()) {
            if let Err(error) = socket.send(packet) {
                eprintln!("btmanage: AAP to {id}: {error}");
                socket.shutdown();
                break;
            }
            std::thread::sleep(PACKET_GAP);
        }
    }

    fn failed(&self, id: &str) {
        let mut state = lock(&self.0);
        state.opening.remove(id);
        let entry = state.failures.entry(id.to_owned()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = now_ms();
    }

    /// Hands every packet to the hub until the channel closes.
    fn read(&self, hub: &SharedHub, id: &str, socket: &Arc<Socket>) {
        let mut buffer = [0u8; 1024];
        while let Ok(length) = socket.recv(&mut buffer) {
            if length == 0 {
                break;
            }
            lock(hub).aap_packet(id, &buffer[..length], now_ms());
        }
        let mut state = lock(&self.0);
        if state
            .open
            .get(id)
            .is_some_and(|open| Arc::ptr_eq(open, socket))
        {
            state.open.remove(id);
        }
    }

    /// Sends a packet over the open channel of a device. False when there is
    /// none or it failed.
    pub fn send(&self, id: &str, packet: &[u8]) -> bool {
        let socket = lock(&self.0).open.get(id).cloned();
        socket.is_some_and(|socket| socket.send(packet).is_ok())
    }

    /// Switches the noise mode of a device.
    pub fn set_noise(&self, id: &str, mode: aap::NoiseMode) -> bool {
        self.send(id, &aap::noise_command(mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use btmanage_core::SystemDevice;

    #[test]
    fn only_connected_apple_accessories_are_wanted() {
        let mut found = Found {
            accessory: true,
            paired: true,
            device: SystemDevice {
                id: "80:AF:19:19:B4:77".into(),
                address: Some(0x80AF_1919_B477),
                connected: true,
                ..SystemDevice::default()
            },
            ..Found::default()
        };
        assert_eq!(
            target(&found),
            Some(("80:AF:19:19:B4:77".to_owned(), 0x80AF_1919_B477))
        );
        found.device.connected = false;
        assert_eq!(target(&found), None);
        found.device.connected = true;
        found.accessory = false;
        assert_eq!(target(&found), None);
    }
}
