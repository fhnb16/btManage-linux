# To do: Linux

## First: run 0.3.0 on Linux

Written without a Linux machine: the hub, BlueZ through `busctl` and `bluetoothctl` (`src/bluez.rs`), the buttons to connect and disconnect, sharing with pairing. To try: the paired devices show with the battery BlueZ has; AirPods in use show each bud and the case; the button connects and disconnects; a discovery that runs all the time does not disturb the sound of headphones in use (if it does, scan only while the window is open); sharing with a phone, paired by code.

The reading of `busctl --json` is tested against a sample written from BlueZ's description, not against a capture. If the devices do not show, the output of `busctl --system --json=short call org.bluez / org.freedesktop.DBus.ObjectManager GetManagedObjects` is what to send.

## Then: AAP and the console of 0.4.0

Written without a Linux machine. To try:

- AirPods connected: within a few seconds the levels move in steps of 1 %, and "Noise control" shows under them. Changing it changes the mode, and changing the mode on the buds moves the choice. If not, the terminal says why the channel failed (`btmanage: AAP to …`). The kernel may need the channel at a security level the socket does not ask for yet (`BT_SECURITY`); LibrePods on Linux is the place to compare.
- The console: GATT on a Bluetooth LE mouse (connect lists the characteristics, read 2A19, subscribe to it), RFCOMM channel 1 on a serial module with AT, raw AAP on L2CAP 0x1001. Notifications come through `bluetoothctl`, whose lines are shown as it prints them; the stripping of its colours and prompt is written from memory of its output.
- RFCOMM needs the channel number: there is no SDP lookup yet, so a service that is not on channel 1 has to be found with `sdptool browse` or similar.

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

- AAP: conversation awareness and ear detection. Levels and noise control are done (0.4.0).
- The console: an SDP lookup of RFCOMM channels by service UUID, and HFP AT commands to a headset.
- The audio codec in use, from PipeWire or PulseAudio.

## Updates and distribution

- Download and replace the binary from the application, with the SHA-256 check the core already supports. Today the update line only opens the release page.
- `.deb`, `.rpm`, AppImage and Flatpak.
- Run the binary on a real Linux desktop; so far it has only been compiled there.

## Beyond Bluetooth devices

- AirPlay sender and receiver.
- AirDrop.
