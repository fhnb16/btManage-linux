# Working in btManage-linux

Guidance for anyone, human or AI, changing this repository.

## What this is

The Linux client of btManage: one Rust binary with a Slint interface and a StatusNotifierItem tray icon. It uses `btmanage-core` directly, as a path dependency on `../btManage-core`. It is a skeleton: BlueZ scanning is not written yet.

The maintainer has no Linux machine. The crate therefore also builds on Windows, where it runs in demo mode. Keep that working: it is how the interface is looked at.

## Rules

- **Linux-only code sits behind `#[cfg(target_os = "linux")]`** and is kept thin. Code that uses Slint goes outside it, so the Windows build type-checks it; see `toggle` and `quit_on_close` in `src/main.rs`.
- **Logic that every platform needs goes into the core**, not here.
- Tooling is Rust only. No Python or Node.
- Do not commit or push unless asked. The maintainer does that.
- **Raise the version with every change that ships**, as `major.minor.patch`: patch for a fix, minor for a new capability, major for a break. The version is `version` in `Cargo.toml`; builds run with `--locked`, so the `btmanage-linux` entry in `Cargo.lock` must carry the same number. When the core's version changes, its entry in this `Cargo.lock` changes with it. CI adds a fourth number by itself only when the same version is built again.
- The release file name `btmanage-linux-<arch>.tar.gz` is a contract with the updater (`src/update.rs`).
- Say plainly what has not been run on Linux. Most of this repository has only ever been compiled there, in CI.

## Interface

`ui/app.slint` follows the Windows panel: 360 px wide, a 4 px spacing grid, a ten-segment meter, one accent, no decoration. A value that is not known is left out.

## Checking a change

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo run --release        on Windows: the window, on made-up devices
```

The Linux-only modules do not compile on Windows. To type-check one without a Linux machine, put it into a scratch crate with its dependencies and run `cargo clippy --target x86_64-unknown-linux-gnu` there; that works for pure-Rust dependencies such as `ksni`. Slint itself needs system libraries and cannot be checked that way. CI is the real test.

## Where things go

- BlueZ: a module that reads paired devices and advertisements over D-Bus and feeds a `btmanage_core::Hub`. The application still drives the older `Tracker`; moving to the hub is the first step.
- New window content: properties on `AppWindow` in `ui/app.slint`, filled in `render` in `src/main.rs`.
