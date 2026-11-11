//! Synthetic advertisements for looking at the interface without earbuds at
//! hand, and for running the application where there is no BlueZ.

use btmanage_core::Tracker;
use btmanage_core::apple::COMPANY_ID;

/// Seconds the left earbud stays on one level before it drains a step.
const SECONDS_PER_STEP: u64 = 6;

/// Feeds the tracker what one second of a nearby pair of AirPods Pro and a
/// pair of AirPods Max would send. `tick` counts the seconds since start.
pub fn feed(tracker: &mut Tracker, tick: u64, now_ms: u64) {
    let left = 9 - (tick / SECONDS_PER_STEP % 9) as u8;
    // AirPods Pro: left draining, right at 70 %, case at 50 % and charging.
    let pro = message([0x0E, 0x20], 0x22, 0x70 | left, 0x45);
    // AirPods Max further away, at 60 %.
    let max = message([0x0A, 0x20], 0x20, 0xF6, 0x0F);
    tracker.ingest(0xA1, -48, COMPANY_ID, &pro, now_ms);
    tracker.ingest(0xB2, -62, COMPANY_ID, &max, now_ms);
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

    #[test]
    fn demo_yields_two_devices_nearest_first() {
        let mut tracker = Tracker::new();
        feed(&mut tracker, 0, 0);
        let snapshot = tracker.snapshot(0);
        let models: Vec<_> = snapshot.devices.iter().map(|d| d.model).collect();
        assert_eq!(models, [Some("AirPods Pro"), Some("AirPods Max")]);
        assert_eq!(snapshot.devices[0].left.unwrap().percent, 90);
    }

    #[test]
    fn left_earbud_drains_and_wraps() {
        let mut tracker = Tracker::new();
        feed(&mut tracker, 8 * SECONDS_PER_STEP, 0);
        assert_eq!(tracker.snapshot(0).devices[0].left.unwrap().percent, 10);
        feed(&mut tracker, 9 * SECONDS_PER_STEP, 1_000);
        assert_eq!(tracker.snapshot(1_000).devices[0].left.unwrap().percent, 90);
    }
}
