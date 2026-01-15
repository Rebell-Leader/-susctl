use crate::config::Config;
use crate::set_ui_callbacks;
use crate::ui::show_toast;
use crate::{MainWindow, SlashPageData};
use rog_dbus::{find_iface_async, zbus_slash::SlashProxy};
use rog_slash::SlashMode;
use slint::{ComponentHandle, Model};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

pub fn setup_slash(ui: &MainWindow, _config: Arc<Mutex<Config>>) {
    let ui_weak = ui.as_weak();

    tokio::spawn(async move {
        // Find the Slash interface proxy
        let proxies = match find_iface_async::<SlashProxy>("xyz.ljones.Slash").await {
            Ok(p) => p,
            Err(e) => {
                log::warn!("Failed to find Slash interface: {}", e);
                return;
            }
        };

        let proxy = match proxies.first() {
            Some(p) => p.clone(),
            None => return,
        };

        // UI Callbacks (MUST be done on UI thread to access global state)
        {
            let proxy_copy = proxy.clone();

            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                let proxy = proxy_copy.clone();
                let ui_handle = ui;

                set_ui_callbacks!(ui_handle, SlashPageData(), proxy.enabled(), "Slash enabled {}", "Failed to enable slash");

                // Fix: Cast f32 (UI) to u8 (DBus)
                set_ui_callbacks!(ui_handle, SlashPageData(as f32), proxy.brightness(as u8), "Slash brightness set to {}", "Failed to set slash brightness");
                set_ui_callbacks!(ui_handle, SlashPageData(as f32), proxy.interval(as u8), "Slash interval set to {}", "Failed to set slash interval");

                set_ui_callbacks!(ui_handle, SlashPageData(), proxy.show_battery_warning(), "Battery warning set to {}", "Failed to set battery warning");
                set_ui_callbacks!(ui_handle, SlashPageData(), proxy.show_on_battery(), "Show on battery set to {}", "Failed to set show on battery");
                set_ui_callbacks!(ui_handle, SlashPageData(), proxy.show_on_boot(), "Show on boot set to {}", "Failed to set show on boot");
                set_ui_callbacks!(ui_handle, SlashPageData(), proxy.show_on_shutdown(), "Show on shutdown set to {}", "Failed to set show on shutdown");
                set_ui_callbacks!(ui_handle, SlashPageData(), proxy.show_on_sleep(), "Show on sleep set to {}", "Failed to set show on sleep");
                set_ui_callbacks!(ui_handle, SlashPageData(), proxy.show_on_lid_closed(), "Show on lid closed set to {}", "Failed to set show on lid closed");
            });
        }

        // Custom Mode Logic - Callback setup
        {
            let proxy_copy = proxy.clone();
            let ui_weak_copy = ui_weak.clone();
            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                let data = ui.global::<SlashPageData>();

                data.on_cb_mode_index(move |idx| {
                    let proxy_copy = proxy_copy.clone();
                    let handle_weak = ui_weak_copy.clone();

                    let mode_str_opt = if let Some(h) = handle_weak.upgrade() {
                        let d = h.global::<SlashPageData>();
                        if idx >= 0 && (idx as usize) < d.get_modes().row_count() {
                            Some(d.get_modes().row_data(idx as usize).unwrap_or_default())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(mode_str) = mode_str_opt {
                        if let Ok(mode) = SlashMode::from_str(&mode_str) {
                            tokio::spawn(async move {
                                show_toast(
                                    format!("Slash mode set to {}", mode).into(),
                                    "Failed to set slash mode".into(),
                                    handle_weak,
                                    proxy_copy.set_mode(mode).await,
                                );
                            });
                        }
                    }
                });
            });
        }

        // D-Bus Signal -> UI
        let proxy_copy = proxy.clone();
        let handle_copy = ui_weak.clone();
        tokio::spawn(async move {
            let mut changes = proxy_copy.receive_mode_changed().await;
            use futures_util::StreamExt;
            while let Some(change) = changes.next().await {
                if let Ok(mode) = change.get().await {
                    let mode_str = mode.to_string();
                    let handle_copy = handle_copy.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(h) = handle_copy.upgrade() {
                            let d = h.global::<SlashPageData>();
                            let model = d.get_modes();
                            for (i, m) in model.iter().enumerate() {
                                if m == mode_str {
                                    d.set_mode_index(i as i32);
                                    break;
                                }
                            }
                        }
                    });
                }
            }
        });

        if let Ok(m) = proxy.mode().await {
            let mode_str = m.to_string();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(h) = ui_weak.upgrade() {
                    let d = h.global::<SlashPageData>();
                    let model = d.get_modes();
                    for (i, m) in model.iter().enumerate() {
                        if m == mode_str {
                            d.set_mode_index(i as i32);
                            break;
                        }
                    }
                }
            });
        }
    });
}
