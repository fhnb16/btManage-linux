//! Tray icon through the StatusNotifierItem protocol, which KDE, Xfce, Cinnamon
//! and GNOME (with the AppIndicator extension) understand.

use ksni::blocking::TrayMethods;

/// Callbacks run on the tray's own thread; hop to the UI thread inside them.
type Callback = Box<dyn Fn() + Send + Sync>;

pub struct AppTray {
    pub on_activate: Callback,
    pub on_quit: Callback,
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        "btmanage".into()
    }

    fn title(&self) -> String {
        "btManage".into()
    }

    fn icon_name(&self) -> String {
        // From the freedesktop icon naming specification, so every theme has it.
        "audio-headphones".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.on_activate)();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show".into(),
                activate: Box::new(|tray: &mut Self| (tray.on_activate)()),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| (tray.on_quit)()),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Puts the icon into the tray. Keep the handle: dropping it removes the icon.
pub fn spawn(tray: AppTray) -> Result<ksni::blocking::Handle<AppTray>, ksni::Error> {
    tray.spawn()
}
