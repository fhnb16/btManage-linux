//! btManage for Linux.
//!
//! This is the skeleton: the window, the tray icon and the link to the core
//! are in place, scanning through BlueZ is not. Until it is, the application
//! shows live-looking data only in demo mode (`--demo`, and always on systems
//! other than Linux, where it serves as a preview of the interface).

// No console window next to the preview on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod demo;
mod share;
#[cfg(target_os = "linux")]
mod tray;
mod update;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use btmanage_core::{Cell, Device, FormFactor, Snapshot, Tracker};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// A reading older than this is shown with its age.
const FRESH_MS: u64 = 30_000;
/// At or below this level, and not charging, a battery counts as low.
const LOW_PERCENT: u8 = 20;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let demo = cfg!(not(target_os = "linux")) || std::env::args().any(|arg| arg == "--demo");

    let window = AppWindow::new()?;
    let tracker = Rc::new(RefCell::new(Tracker::new()));
    let started = Instant::now();
    let now_ms = move || started.elapsed().as_millis() as u64;

    // Once a second: feed the demo data if asked to, then redraw. The redraw
    // also moves the "updated … ago" text on.
    let timer = slint::Timer::default();
    let mut tick = {
        let window = window.as_weak();
        let tracker = Rc::clone(&tracker);
        let mut seconds = 0u64;
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let mut tracker = tracker.borrow_mut();
            if demo {
                demo::feed(&mut tracker, seconds, now_ms());
                seconds += 1;
            }
            render(&window, &tracker.snapshot(now_ms()));
        }
    };
    tick();
    timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), tick);

    check_for_update(&window);
    // Files and text with other btManage applications on the network.
    let _sharing = share::start(&window);

    #[cfg(target_os = "linux")]
    let _tray = spawn_tray(&window);

    window.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}

/// Looks for a newer release in the background and, if there is one, shows a
/// line under the devices that opens the release page.
fn check_for_update(window: &AppWindow) {
    let weak = window.as_weak();
    std::thread::spawn(move || {
        let Some(found) = update::check() else {
            return;
        };
        let _ = weak.upgrade_in_event_loop(move |window| {
            window.set_update_text(format!("Version {} is available", found.version).into());
            window.on_open_update(move || {
                let _ = webbrowser::open(&found.url);
            });
        });
    });
}

/// Starts the tray icon. Without a tray host the application still runs, as
/// a plain window.
#[cfg(target_os = "linux")]
fn spawn_tray(window: &AppWindow) -> Option<ksni::blocking::Handle<tray::AppTray>> {
    let weak = window.as_weak();
    let app_tray = tray::AppTray {
        on_activate: Box::new(move || toggle(&weak)),
        on_quit: Box::new(|| {
            let _ = slint::quit_event_loop();
        }),
    };

    match tray::spawn(app_tray) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("btmanage: no tray icon: {error}");
            quit_on_close(window);
            None
        }
    }
}

/// Shows the window if it is hidden and hides it if it is shown. Callable
/// from any thread.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn toggle(window: &slint::Weak<AppWindow>) {
    let _ = window.upgrade_in_event_loop(|window| {
        let _ = if window.window().is_visible() {
            window.hide()
        } else {
            window.show()
        };
    });
}

/// Makes closing the window quit the application. Used when there is no tray
/// icon, since nothing would bring a hidden window back.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn quit_on_close(window: &AppWindow) {
    window.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
}

fn render(window: &AppWindow, snapshot: &Snapshot) {
    let Some(nearest) = snapshot.devices.first() else {
        window.set_has_device(false);
        window.set_empty_title("No headphones nearby".into());
        window.set_empty_hint("Open the case or put the earbuds in next to this computer".into());
        return;
    };

    window.set_has_device(true);
    window.set_device_name(name(nearest).into());
    window.set_device_age(age(nearest).into());
    window.set_cells(ModelRc::new(VecModel::from(cells(nearest))));
    window.set_others(ModelRc::new(VecModel::from(
        snapshot.devices[1..]
            .iter()
            .map(|device| OtherDevice {
                name: name(device).into(),
                levels: levels(device).into(),
            })
            .collect::<Vec<_>>(),
    )));
}

fn name(device: &Device) -> &'static str {
    device.model.unwrap_or("Apple headphones")
}

fn age(device: &Device) -> String {
    match device.age_ms {
        age if age < FRESH_MS => String::new(),
        age if age < 60_000 => "Updated less than a minute ago".into(),
        age => format!("Updated {} min ago", age / 60_000),
    }
}

fn cells(device: &Device) -> Vec<CellData> {
    match device.form {
        FormFactor::Headphones => vec![cell("main", "Battery", device.main)],
        FormFactor::Earbuds => vec![
            cell("left", "Left", device.left),
            cell("right", "Right", device.right),
            cell("case", "Case", device.case),
        ],
    }
}

fn cell(kind: &str, label: &str, cell: Option<Cell>) -> CellData {
    let label = match cell {
        Some(Cell {
            in_ear: Some(true), ..
        }) => format!("{label} · in ear"),
        _ => label.to_owned(),
    };
    CellData {
        kind: SharedString::from(kind),
        label: label.into(),
        known: cell.is_some(),
        percent: cell.map_or(0, |cell| i32::from(cell.percent)),
        charging: cell.is_some_and(|cell| cell.charging),
        low: cell.is_some_and(|cell| cell.percent <= LOW_PERCENT && !cell.charging),
    }
}

/// "Left 90%, Right 70%, Case 50%", or the single level of headphones.
fn levels(device: &Device) -> String {
    if let Some(main) = device.main {
        return format!("{}%", main.percent);
    }
    [
        ("Left", device.left),
        ("Right", device.right),
        ("Case", device.case),
    ]
    .iter()
    .filter_map(|(label, cell)| cell.map(|cell| format!("{label} {}%", cell.percent)))
    .collect::<Vec<_>>()
    .join(", ")
}
