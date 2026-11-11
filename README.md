# btManage for Linux

A tray application that shows the battery levels of Bluetooth devices. Written in Rust with [Slint](https://slint.dev).

**Status: skeleton.** The window, the tray icon, the link to the core and the check for updates are in place. Scanning through BlueZ is not, so real devices do not show up yet; `--demo` shows the interface with made-up data. What is planned is in [TODO.md](TODO.md).

## Build

Needs Rust (stable) and the btManage-core repository checked out next to this one:

```
btManage-core/
btManage-linux/
```

On Debian and Ubuntu, the system libraries:

```
sudo apt-get install pkg-config libfontconfig1-dev libfreetype6-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libgl1-mesa-dev libegl1-mesa-dev libdbus-1-dev
```

```
cargo build --release
./target/release/btmanage --demo
```

The tray icon uses the StatusNotifierItem protocol: KDE, Xfce and Cinnamon support it out of the box, GNOME needs the AppIndicator extension. Without a tray host the application runs as a plain window.

## Preview on Windows

The same crate builds on Windows and runs there in demo mode, as a way to look at the interface without a Linux machine:

```
cargo run --release
```

## Updates

At start the application asks GitHub for the latest release of this repository and, if it is newer, shows a line under the devices that opens the release page. Nothing is downloaded or replaced on its own. The request carries nothing but a fixed client name.

## CI and releases

`.github/workflows/build.yml` builds x86_64 and aarch64 binaries on Ubuntu 22.04, so they run with glibc 2.35 or newer, and a Windows preview. A push to `main` or `master` also publishes a release: the version is `version` from `Cargo.toml`, with a fourth number appended when that version has been released before. Pushing a tag `v1.2.3` releases exactly that version.

Because btManage-core is private, the workflow needs a repository secret `CORE_REPO_TOKEN`: a fine-grained personal access token with read access to the contents of btManage-core. Pull requests from forks are skipped, since they do not receive secrets.
