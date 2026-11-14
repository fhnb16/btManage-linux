//! Synthetic advertisements for looking at the interface without earbuds at
//! hand, and for running the application where there is no BlueZ.

use btmanage_core::Hub;
use btmanage_core::apple::COMPANY_ID;
use btmanage_core::hub::Advertisement;

/// Seconds the left earbud stays on one level before it drains a step.
const SECONDS_PER_STEP: u64 = 6;

/// Feeds the hub what one second of a nearby pair of AirPods Pro and a pair of
/// AirPods Max would send. `tick` counts the seconds since start.
pub fn feed(hub: &mut Hub, tick: u64, now_ms: u64) {
    let left = 9 - (tick / SECONDS_PER_STEP % 9) as u8;
    // AirPods Pro: left draining, right at 70 %, case at 50 % and charging.
    let pro = message([0x0E, 0x20], 0x22, 0x70 | left, 0x45);
    // AirPods Max further away, at 60 %.
    let max = message([0x0A, 0x20], 0x20, 0xF6, 0x0F);
    for (address, rssi, data) in [(0xA1, -48, &pro), (0xB2, -62, &max)] {
        let advertisement = Advertisement::Manufacturer {
            company_id: COMPANY_ID,
            data,
        };
        hub.advertisement(address, rssi, advertisement, now_ms);
    }
}

/// A proximity pairing message; `btmanage_core::apple::proximity` documents
/// the layout.
fn message(model: [u8; 2], status: u8, levels: u8, charge_and_case: u8) -> [u8; 27] {
    let mut data = [0u8; 27];
    data[..8].copy_from_slice(&[
        0x07,
        0x19,
        0x01,
        model[0],
        model[1],
        status,
        levels,
        charge_and_case,
    ]);
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;

    fn left_of_pro(hub: &mut Hub, now: u64) -> u8 {
        let snapshot = hub.snapshot(now);
        let pro = snapshot
            .devices
            .iter()
            .find(|device| device.model == Some("AirPods Pro"))
            .unwrap();
        pro.cells[0].percent
    }

    #[test]
    fn demo_yields_two_devices() {
        let mut hub = Hub::new();
        feed(&mut hub, 0, T0);
        let snapshot = hub.snapshot(T0);
        let mut models: Vec<_> = snapshot.devices.iter().map(|d| d.model).collect();
        models.sort();
        assert_eq!(models, [Some("AirPods Max"), Some("AirPods Pro")]);
        assert_eq!(left_of_pro(&mut hub, T0), 90);
    }

    #[test]
    fn left_earbud_drains_and_wraps() {
        let mut hub = Hub::new();
        feed(&mut hub, 8 * SECONDS_PER_STEP, T0);
        assert_eq!(left_of_pro(&mut hub, T0), 10);
        feed(&mut hub, 9 * SECONDS_PER_STEP, T0 + 1_000);
        assert_eq!(left_of_pro(&mut hub, T0 + 1_000), 90);
    }
}
