# To do: Linux

## First: real devices

- Move from the single-model `Tracker` to `btmanage_core::Hub`, as the Windows and Android clients have.
- BlueZ over D-Bus: paired devices with name, address, class, connection state; the `org.bluez.Battery1` interface for their levels; advertisement monitoring for manufacturer and service data.
- Connect, disconnect and remove devices through BlueZ.
- Keep what the hub remembers in `$XDG_STATE_HOME/btmanage`.

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
