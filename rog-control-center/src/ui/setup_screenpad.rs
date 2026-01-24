use log::{debug, error};
use rog_dbus::zbus_backlight::BacklightProxy;
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::ui::show_toast;
use crate::{MainWindow, ScreenpadPageData};

pub fn setup_screenpad(ui: &MainWindow, _config: Arc<Mutex<Config>>) {
    let handle = ui.as_weak();

    tokio::spawn(async move {
        // Create the connections/proxies here
        let conn = match zbus::Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("Failed to connect to system bus for Screenpad: {e:}");
                return;
            }
        };

        let backlight = match BacklightProxy::builder(&conn).build().await {
            Ok(backlight) => backlight,
            Err(e) => {
                error!("Failed to create backlight proxy for Screenpad: {e:}");
                return;
            }
        };

        // Initialize state
        debug!("Initializing Screenpad page data");

        // Use helper to set initial properties
        if let Ok(val) = backlight.screenpad_brightness().await {
            handle
                .upgrade_in_event_loop(move |h| {
                    h.global::<ScreenpadPageData>().set_brightness(val);
                    // Assume power is on if brightness > 0
                    h.global::<ScreenpadPageData>().set_power(val > 0);
                })
                .ok();
        }

        if let Ok(gamma_str) = backlight.screenpad_gamma().await {
            if let Ok(gamma) = gamma_str.parse::<f32>() {
                handle
                    .upgrade_in_event_loop(move |h| {
                        h.global::<ScreenpadPageData>().set_gamma(gamma);
                    })
                    .ok();
            }
        }

        if let Ok(sync) = backlight.screenpad_sync_with_primary().await {
            handle
                .upgrade_in_event_loop(move |h| {
                    h.global::<ScreenpadPageData>().set_sync_with_primary(sync);
                })
                .ok();
        }

        // Set up callbacks
        let handle_copy = handle.clone();
        let backlight_copy = backlight.clone();
        handle
            .upgrade_in_event_loop(move |h| {
                let global = h.global::<ScreenpadPageData>();

                // Brightness Callback
                let hl = handle_copy.clone();
                let bl = backlight_copy.clone();
                global.on_cb_brightness(move |val| {
                    let bl = bl.clone();
                    let hl = hl.clone();
                    tokio::spawn(async move {
                        show_toast(
                            format!("Screenpad brightness set to {}", val).into(),
                            "Failed to set Screenpad brightness".into(),
                            hl,
                            bl.set_screenpad_brightness(val).await,
                        );
                    });
                });

                // Gamma Callback
                let hl = handle_copy.clone();
                let bl = backlight_copy.clone();
                global.on_cb_gamma(move |val| {
                    let bl = bl.clone();
                    let hl = hl.clone();
                    tokio::spawn(async move {
                        show_toast(
                            format!("Screenpad gamma set to {:.2}", val).into(),
                            "Failed to set Screenpad gamma".into(),
                            hl,
                            bl.set_screenpad_gamma(&val.to_string()).await,
                        );
                    });
                });

                // Sync Callback
                let hl = handle_copy.clone();
                let bl = backlight_copy.clone();
                global.on_cb_sync_with_primary(move |val| {
                    let bl = bl.clone();
                    let hl = hl.clone();
                    tokio::spawn(async move {
                        show_toast(
                            format!(
                                "Screenpad sync {}",
                                if val { "enabled" } else { "disabled" }
                            )
                            .into(),
                            "Failed to toggle Screenpad sync".into(),
                            hl,
                            bl.set_screenpad_sync_with_primary(val).await,
                        );
                    });
                });

                // Power Callback (Toggle brightness to 0/last or 100)
                let hl = handle_copy.clone();
                let bl = backlight_copy.clone();
                global.on_cb_power(move |val| {
                    let bl = bl.clone();
                    let hl = hl.clone();
                    tokio::spawn(async move {
                        let target = if val { 100 } else { 0 };
                        let _ = bl.set_screenpad_brightness(target).await;
                        hl.upgrade_in_event_loop(move |h| {
                            h.global::<ScreenpadPageData>().set_brightness(target);
                        })
                        .ok();
                    });
                });
            })
            .ok();

        // Optional: Value watches for external changes
        // (Similar to setup_system.rs if needed)
    });
}
