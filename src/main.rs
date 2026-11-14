//! btManage for Linux.
//!
//! The devices come from BlueZ (`bluez`): the paired ones with the battery
//! BlueZ knows, and the levels earbuds broadcast, all merged by the core's
//! hub, as on Windows and Android. In demo mode (`--demo`, and always on
//! systems other than Linux, where the application serves as a preview of
//! the interface) made-up advertisements stand in.

// No console window next to the preview on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bluez;
mod demo;
mod share;
#[cfg(target_os = "linux")]
mod tray;
mod update;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use btmanage_core::Hub;
use btmanage_core::hub::{CellView, DeviceView, HubSnapshot, Origin, Role};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// A reading older than this is shown with its age.
const FRESH_MS: u64 = 30_000;
/// At or below this level, and not charging, a battery counts as low.
const LOW_PERCENT: u8 = 20;
/// How many devices the list under the featured one shows.
const MAX_OTHERS: usize = 12;

/// The hub, shared with the thread that reads BlueZ.
type SharedHub = Arc<Mutex<Hub>>;
/// D-Bus object paths of the devices, by device identifier, for the buttons.
type Paths = Arc<Mutex<HashMap<String, String>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| u64::try_from(time.as_millis()).unwrap_or(u64::MAX))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let demo = cfg!(not(target_os = "linux")) || std::env::args().any(|arg| arg == "--demo");

    let window = AppWindow::new()?;
    let hub: SharedHub = Arc::default();
    let paths: Paths = Arc::default();
    if !demo {
        read_bluez(&hub, &paths);
    }

    // The devices on the list under the featured one, for its buttons: identifier and
    // whether connected, in the order shown.
    let rows: Rc<RefCell<Vec<(String, bool)>>> = Rc::default();
    let acting = Rc::clone(&rows);
    let acting_paths = Arc::clone(&paths);
    window.on_device_action(move |index| {
        let row = usize::try_from(index)
            .ok()
            .and_then(|index| acting.borrow().get(index).cloned());
        if let Some((id, connected)) = row {
            toggle_connection(&acting_paths, &id, !connected);
        }
    });

    // Once a second: feed the demo data if asked to, then redraw. The redraw
    // also moves the "updated … ago" text on.
    let timer = slint::Timer::default();
    let mut tick = {
        let window = window.as_weak();
        let hub = Arc::clone(&hub);
        let mut seconds = 0u64;
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let now = now_ms();
            let snapshot = {
                let mut hub = lock(&hub);
                if demo {
                    demo::feed(&mut hub, seconds, now);
                    seconds += 1;
                }
                hub.snapshot(now)
            };
            render(&window, &snapshot, &rows);
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

/// Reads BlueZ every two seconds in the background, with a discovery running,
/// and feeds what it finds to the hub.
#[cfg(target_os = "linux")]
fn read_bluez(hub: &SharedHub, paths: &Paths) {
    let hub = Arc::clone(hub);
    let paths = Arc::clone(paths);
    std::thread::spawn(move || {
        let _scan = bluez::Scan::start();
        let mut seen = HashMap::new();
        loop {
            let found = bluez::read();
            *lock(&paths) = found
                .iter()
                .map(|item| (item.device.id.clone(), item.path.clone()))
                .collect();
            bluez::feed(&mut lock(&hub), &found, &mut seen, now_ms());
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn read_bluez(_hub: &SharedHub, _paths: &Paths) {}

/// Connects or disconnects a paired device, away from the window's thread.
#[cfg(target_os = "linux")]
fn toggle_connection(paths: &Paths, id: &str, on: bool) {
    let Some(path) = lock(paths).get(id).cloned() else {
        return;
    };
    std::thread::spawn(move || {
        if !bluez::connect(&path, on) {
            eprintln!("btmanage: BlueZ refused to {} {path}", if on { "connect" } else { "disconnect" });
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn toggle_connection(_paths: &Paths, _id: &str, _on: bool) {}

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

/// Shows the hub's list: the device in use with each of its batteries at the
/// top, the others under it, each with a button where it can be connected.
fn render(window: &AppWindow, snapshot: &HubSnapshot, rows: &RefCell<Vec<(String, bool)>>) {
    let featured = snapshot
        .devices
        .iter()
        .find(|device| (device.connected || device.nearby) && !device.cells.is_empty());

    match featured {
        Some(device) => {
            window.set_has_device(true);
            window.set_device_name(name(device).into());
            window.set_device_age(age(device).into());
            window.set_cells(ModelRc::new(VecModel::from(cells(device))));
        }
        None => {
            window.set_has_device(false);
            window.set_empty_title("No headphones in use".into());
            window.set_empty_hint(
                "Connect your headphones, or open the case next to this computer".into(),
            );
        }
    }

    let others: Vec<&DeviceView> = snapshot
        .devices
        .iter()
        .filter(|device| featured.is_none_or(|shown| shown.id != device.id))
        .take(MAX_OTHERS)
        .collect();
    *rows.borrow_mut() = others
        .iter()
        .map(|device| (device.id.clone(), device.connected))
        .collect();
    let list: Vec<OtherDevice> = others
        .iter()
        .map(|device| OtherDevice {
            name: name(device).into(),
            levels: summary(device).into(),
            action: action(device).into(),
        })
        .collect();
    window.set_others(ModelRc::new(VecModel::from(list)));
}

/// The name of a device; for earbuds that are not paired, what they are.
fn name(device: &DeviceView) -> String {
    if !device.name.is_empty() {
        return device.name.clone();
    }
    match (device.model, device.origin) {
        (Some(model), _) => model.to_owned(),
        (None, Origin::FastPair) => "Fast Pair earbuds".to_owned(),
        _ => "Apple headphones".to_owned(),
    }
}

/// How old the newest level is, when it is not current.
fn age(device: &DeviceView) -> String {
    match device.cells.iter().map(|cell| cell.age_ms).min() {
        Some(age) if age >= 60_000 => format!("Updated {} min ago", age / 60_000),
        Some(age) if age >= FRESH_MS => "Updated less than a minute ago".into(),
        _ => String::new(),
    }
}

fn cells(device: &DeviceView) -> Vec<CellData> {
    let cell = |role: Role| device.cells.iter().find(|cell| cell.role == role);
    if cell(Role::Left).is_some() || cell(Role::Right).is_some() {
        vec![
            cell_data("left", "Left", cell(Role::Left)),
            cell_data("right", "Right", cell(Role::Right)),
            cell_data("case", "Case", cell(Role::Case)),
        ]
    } else {
        vec![cell_data("main", "Battery", cell(Role::Main))]
    }
}

fn cell_data(kind: &str, label: &str, cell: Option<&CellView>) -> CellData {
    let label = match cell {
        Some(CellView {
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

/// "Connected · Left 90%, Right 70%", or the state alone for a device without levels.
fn summary(device: &DeviceView) -> String {
    let state = if device.connected {
        "Connected"
    } else if device.nearby {
        "Nearby"
    } else {
        "Not connected"
    };
    let levels: Vec<String> = device
        .cells
        .iter()
        .map(|cell| match cell.role {
            Role::Left => format!("Left {}%", cell.percent),
            Role::Right => format!("Right {}%", cell.percent),
            Role::Case => format!("Case {}%", cell.percent),
            Role::Main => format!("{}%", cell.percent),
        })
        .collect();
    if levels.is_empty() {
        state.to_owned()
    } else {
        format!("{state} · {}", levels.join(", "))
    }
}

/// The button of a device on the list: paired devices can be connected and disconnected.
fn action(device: &DeviceView) -> &'static str {
    match (device.paired, device.connected) {
        (true, true) => "Disconnect",
        (true, false) => "Connect",
        _ => "",
    }
}
