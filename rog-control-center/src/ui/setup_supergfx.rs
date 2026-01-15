use crate::config::Config;
use crate::ui::show_toast;
use crate::{MainWindow, SupergfxPageData};
use slint::{ComponentHandle, Model, SharedString, VecModel};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use zbus::proxy;

#[proxy(
    interface = "org.supergfxctl.Daemon",
    default_service = "org.supergfxctl.Daemon",
    default_path = "/org/supergfxctl/Gfx"
)]
trait Supergfx {
    fn supported(&self) -> zbus::Result<Vec<String>>;
    fn mode(&self) -> zbus::Result<String>;
    fn set_mode(&self, mode: &str) -> zbus::Result<()>;
    fn vendor(&self) -> zbus::Result<String>;
}

pub fn setup_supergfx(ui: &MainWindow, _config: Arc<Mutex<Config>>) {
    let ui_weak = ui.as_weak();

    tokio::spawn(async move {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Failed to connect to system bus: {}", e);
                return;
            }
        };

        let proxy = match SupergfxProxy::new(&conn).await {
            Ok(p) => p,
            Err(e) => {
                log::warn!("Failed to create Supergfx proxy: {}", e);
                return;
            }
        };

        // Register Callbacks on UI Thread
        {
            let proxy_copy = proxy.clone();
            let ui_weak_copy = ui_weak.clone();
            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                let handle_copy = ui_weak_copy.clone();
                ui.global::<SupergfxPageData>()
                    .on_set_mode(move |mode_str| {
                        let proxy = proxy_copy.clone();
                        let handle = handle_copy.clone();
                        tokio::spawn(async move {
                            show_toast(
                                format!("Switching to {}. Logout required.", mode_str).into(),
                                "Failed to set mode".into(),
                                handle,
                                proxy.set_mode(&mode_str).await,
                            );
                        });
                    });
            });
        }

        // Fetch Initial State
        // Vendor
        if let Ok(vendor) = proxy.vendor().await {
            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                ui.global::<SupergfxPageData>().set_vendor(vendor.into())
            });
        }

        // Supported Modes
        if let Ok(supported) = proxy.supported().await {
            let modes: Vec<SharedString> = supported
                .iter()
                .map(|s| SharedString::from(s.as_str()))
                .collect();
            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                let mode_model = Rc::new(VecModel::from(modes));
                ui.global::<SupergfxPageData>()
                    .set_supported_modes(mode_model.into())
            });
        }

        // Current Mode
        if let Ok(mode) = proxy.mode().await {
            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                let g = ui.global::<SupergfxPageData>();
                g.set_current_mode(mode.clone().into());
                // Update selection index
                let model = g.get_supported_modes();
                for (i, m) in model.iter().enumerate() {
                    if m == mode.as_str() {
                        g.set_selected_index(i as i32);
                        break;
                    }
                }
            });
        }

        // No signal monitoring implemented as supergfxctl state changes usually require user action/logout
    });
}
