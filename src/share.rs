//! Files and text with other btManage applications on the local network,
//! through `btmanage-share`, the same code the Windows and Android
//! applications use.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use btmanage_share::{Config, DISCOVERY_PORT, Event, Kind, Share};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::AppWindow;

/// Lines of history kept on screen.
const HISTORY: usize = 20;

/// Keeps the sharing running for as long as it lives.
pub struct Sharing {
    _share: Rc<Share>,
    _timer: slint::Timer,
}

/// Starts the sharing and wires it to the window. `None` when it could not
/// start; the window says why.
pub fn start(window: &AppWindow) -> Option<Sharing> {
    let inbox = inbox();
    let name = host_name();
    let config = Config {
        id: installation_id(),
        name: name.clone(),
        platform: platform().to_owned(),
        inbox: inbox.clone(),
        discovery: Some(DISCOVERY_PORT),
        port: 0,
        trusted: load_trusted(),
    };
    let share = match Share::start(config) {
        Ok(share) => Rc::new(share),
        Err(error) => {
            window.set_share_status(format!("Sharing could not start: {error}").into());
            return None;
        }
    };
    let status = format!(
        "This computer: {name}, port {}. Received files go to {}",
        share.port(),
        inbox.display()
    );
    window.set_share_status(status.into());

    let addresses = Rc::new(RefCell::new(Vec::<String>::new()));
    let history = Rc::new(RefCell::new(Vec::<String>::new()));

    let receiving = Rc::clone(&share);
    window.on_share_receive_toggled(move |on| receiving.set_receiving(on));

    let weak = window.as_weak();
    let known = Rc::clone(&addresses);
    window.on_share_pick_peer(move |index| {
        let chosen = usize::try_from(index)
            .ok()
            .and_then(|index| known.borrow().get(index).cloned());
        if let (Some(window), Some(address)) = (weak.upgrade(), chosen) {
            window.set_share_target(address.into());
        }
    });

    let weak = window.as_weak();
    let sending = Rc::clone(&share);
    window.on_share_send_text(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let target = window.get_share_target();
        let text = window.get_share_text();
        if !target.trim().is_empty() && !text.is_empty() {
            sending.send_text(target.trim(), &text);
            window.set_share_text(SharedString::new());
        }
    });

    let weak = window.as_weak();
    let pairing = Rc::clone(&share);
    window.on_share_pair(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let target = window.get_share_target();
        let code = window.get_share_pair_code();
        if !target.trim().is_empty() && !code.trim().is_empty() {
            pairing.pair(target.trim(), &code);
            window.set_share_pair_code(SharedString::new());
        }
    });

    let weak = window.as_weak();
    let sending = Rc::clone(&share);
    window.on_share_send_file(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let target = window.get_share_target();
        let path = window.get_share_file();
        if !target.trim().is_empty() && !path.trim().is_empty() {
            sending.send_file(target.trim(), Path::new(path.trim()));
        }
    });

    // Once a second: who is there, and what came and went.
    let timer = slint::Timer::default();
    let weak = window.as_weak();
    let polled = Rc::clone(&share);
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            // Six digits, read out across a room: in two groups of three.
            let code = polled.code();
            let shown = if code.len() == 6 {
                format!("{} {}", &code[..3], &code[3..])
            } else {
                code
            };
            window.set_share_code(shown.into());

            let peers = polled.peers();
            *addresses.borrow_mut() = peers.iter().map(|peer| peer.address.clone()).collect();
            let lines: Vec<SharedString> = peers
                .iter()
                .map(|peer| {
                    let state = if peer.trusted { "paired" } else { "not paired" };
                    format!("{} · {} · {state}", peer.name, peer.platform).into()
                })
                .collect();
            window.set_share_peers(ModelRc::new(VecModel::from(lines)));

            let events = polled.take_events();
            if events
                .iter()
                .any(|event| matches!(event, Event::Paired { .. }))
            {
                save_trusted(&polled.trusted());
            }
            if !events.is_empty() {
                let mut history = history.borrow_mut();
                for event in &events {
                    history.insert(0, describe(event));
                }
                history.truncate(HISTORY);
                let rows: Vec<SharedString> =
                    history.iter().map(|line| line.as_str().into()).collect();
                window.set_share_history(ModelRc::new(VecModel::from(rows)));
            }
        },
    );

    Some(Sharing {
        _share: share,
        _timer: timer,
    })
}

/// One line of history for an event.
fn describe(event: &Event) -> String {
    match event {
        Event::Received {
            from,
            kind,
            name,
            text,
            ..
        } => match kind {
            Kind::Text => format!("Text from {from}: {}", text.as_deref().unwrap_or_default()),
            Kind::File | Kind::Pair => format!("{name} from {from}"),
        },
        Event::Sent { to, kind, name, .. } => match kind {
            Kind::Text => format!("Text sent to {to}"),
            Kind::File | Kind::Pair => format!("{name} sent to {to}"),
        },
        Event::Failed {
            to,
            needs_pairing: true,
            ..
        } => format!("Not sent to {to}: pair with its code first"),
        Event::Failed {
            to, name, error, ..
        } => format!("Not sent, {name} {to}: {error}"),
        Event::Paired { name, .. } => format!("Paired with {name}"),
    }
}

/// The file the paired devices are kept in.
fn trusted_file() -> PathBuf {
    config_dir().join("share-trusted")
}

/// The paired devices kept from earlier runs, an identifier a line.
fn load_trusted() -> Vec<String> {
    fs::read_to_string(trusted_file())
        .map(|text| {
            text.lines()
                .map(str::to_owned)
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn save_trusted(ids: &[String]) {
    let _ = fs::create_dir_all(config_dir());
    let _ = fs::write(trusted_file(), ids.join("\n"));
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map_or_else(|| home().join(".config"), PathBuf::from)
        .join("btmanage")
}

fn platform() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(windows) {
        "windows"
    } else {
        "other"
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(std::env::temp_dir, PathBuf::from)
}

/// Where received files go: Downloads/btManage in the home directory.
fn inbox() -> PathBuf {
    home().join("Downloads").join("btManage")
}

/// The name of the computer, as the other devices show it.
fn host_name() -> String {
    let from_file = fs::read_to_string("/etc/hostname").ok();
    let from_env = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok());
    from_file
        .or(from_env)
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Linux computer".to_owned())
}

/// A stable identifier of this installation, kept in the configuration directory.
fn installation_id() -> String {
    let dir = config_dir();
    let file = dir.join("share-id");
    if let Ok(id) = fs::read_to_string(&file) {
        let id = id.trim().to_owned();
        if !id.is_empty() {
            return id;
        }
    }
    let id = btmanage_share::new_id();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&file, &id);
    id
}
