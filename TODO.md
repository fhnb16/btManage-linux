# To do: Linux

## First: run 0.3.0 on Linux

Written without a Linux machine: the hub, BlueZ through `busctl` and `bluetoothctl` (`src/bluez.rs`), the buttons to connect and disconnect, sharing with pairing. To try: the paired devices show with the battery BlueZ has; AirPods in use show each bud and the case; the button connects and disconnects; a discovery that runs all the time does not disturb the sound of headphones in use (if it does, scan only while the window is open); sharing with a phone, paired by code.

The reading of `busctl --json` is tested against a sample written from BlueZ's description, not against a capture. If the devices do not show, the output of `busctl --system --json=short call org.bluez / org.freedesktop.DBus.ObjectManager GetManagedObjects` is what to send.

## Devices

- Remove a pairing from the application (`Adapter1.RemoveDevice`).
- Keep what the hub remembers in `$XDG_STATE_HOME/btmanage`.
- `zbus` in place of the processes, with the property signals of BlueZ instead of polling every two seconds. Needs `cargo` at hand to add the crate to the lock file.
- Pairing a new device from the application: BlueZ's agent interface.

## Interface

- The device list and the device page of the Windows client: history chart, details, actions.
- Settings: notifications, low battery threshold, theme, accent, language, autostart.
- Desktop notifications when headphones connect and when a battery runs low.
- Tray: levels in the tooltip, a menu entry per device.
- Translations. The strings are in English only.
- An icon of our own in the tray and the launcher, instead of the theme's `audio-headphones`.

## Beyond what Windows can do

- AAP over L2CAP for AirPods: noise control, conversation awareness, ear detection. BlueZ lets an application open the channel.
- The audio codec in use, from PipeWire or PulseAudio.

## Updates and distribution

- Download and replace the binary from the application, with the SHA-256 check the core already supports. Today the update line only opens the release page.
- `.deb`, `.rpm`, AppImage and Flatpak.
- Run the binary on a real Linux desktop; so far it has only been compiled there.

## Beyond Bluetooth devices

- AirPlay sender and receiver.
- AirDrop.
