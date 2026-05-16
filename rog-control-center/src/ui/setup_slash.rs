use std::sync::{Arc, Mutex};

use log::{error, info};
use rog_dbus::find_iface_async;
use rog_dbus::zbus_slash::SlashProxy;
use rog_slash::SlashMode;
use slint::ComponentHandle;

use crate::config::Config;
use crate::ui::show_toast;
use crate::{MainWindow, SlashPageData};

const SLASH_MODES: [SlashMode; 16] = [
    SlashMode::Static,
    SlashMode::Bounce,
    SlashMode::Slash,
    SlashMode::Loading,
    SlashMode::BitStream,
    SlashMode::Transmission,
    SlashMode::Flow,
    SlashMode::Flux,
    SlashMode::Phantom,
    SlashMode::Spectrum,
    SlashMode::Hazard,
    SlashMode::Interfacing,
    SlashMode::Ramp,
    SlashMode::GameOver,
    SlashMode::Start,
    SlashMode::Buzzer,
];

fn mode_to_index(mode: SlashMode) -> i32 {
    SLASH_MODES.iter().position(|&m| m == mode).unwrap_or(0) as i32
}

fn index_to_mode(index: i32) -> SlashMode {
    SLASH_MODES.get(index as usize).copied().unwrap_or(SlashMode::Flow)
}

pub fn setup_slash_page(ui: &MainWindow, _states: Arc<Mutex<Config>>) {
    let handle = ui.as_weak();
    tokio::spawn(async move {
        let Ok(slashes) = find_iface_async::<SlashProxy>("xyz.ljones.Slash").await else {
            info!("No Slash LED device found");
            return;
        };

        for slash in slashes {
            // Read initial state
            if let Ok(v) = slash.enabled().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_enabled(v)).ok();
            }
            if let Ok(v) = slash.brightness().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_brightness(v as i32)).ok();
            }
            if let Ok(v) = slash.interval().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_interval(v as i32)).ok();
            }
            if let Ok(v) = slash.mode().await {
                let idx = mode_to_index(v);
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_mode(idx)).ok();
            }
            if let Ok(v) = slash.show_on_boot().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_show_on_boot(v)).ok();
            }
            if let Ok(v) = slash.show_on_shutdown().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_show_on_shutdown(v)).ok();
            }
            if let Ok(v) = slash.show_on_sleep().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_show_on_sleep(v)).ok();
            }
            if let Ok(v) = slash.show_on_battery().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_show_on_battery(v)).ok();
            }
            if let Ok(v) = slash.show_battery_warning().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_show_battery_warning(v)).ok();
            }
            if let Ok(v) = slash.show_on_lid_closed().await {
                let h = handle.clone();
                h.upgrade_in_event_loop(move |ui| ui.global::<SlashPageData>().set_show_on_lid_closed(v)).ok();
            }

            // Wire up callbacks
            handle.upgrade_in_event_loop({
                let slash = slash.clone();
                let h = handle.clone();
                move |ui| {
                    let global = ui.global::<SlashPageData>();

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_enabled(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash LED enabled changed".into(), "Failed to set Slash enabled".into(), hh, s.set_enabled(v).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_brightness(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash brightness changed".into(), "Failed to set Slash brightness".into(), hh, s.set_brightness(v as u8).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_interval(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash interval changed".into(), "Failed to set Slash interval".into(), hh, s.set_interval(v as u8).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_mode(move |v| {
                        let mode = index_to_mode(v);
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash mode changed".into(), "Failed to set Slash mode".into(), hh, s.set_mode(mode).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_show_on_boot(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash boot setting changed".into(), "Failed to set Slash boot".into(), hh, s.set_show_on_boot(v).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_show_on_shutdown(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash shutdown setting changed".into(), "Failed".into(), hh, s.set_show_on_shutdown(v).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_show_on_sleep(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash sleep setting changed".into(), "Failed".into(), hh, s.set_show_on_sleep(v).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_show_on_battery(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash battery setting changed".into(), "Failed".into(), hh, s.set_show_on_battery(v).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_show_battery_warning(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash battery warning changed".into(), "Failed".into(), hh, s.set_show_battery_warning(v).await);
                        });
                    });

                    let s = slash.clone(); let hh = h.clone();
                    global.on_cb_show_on_lid_closed(move |v| {
                        let s = s.clone(); let hh = hh.clone();
                        tokio::spawn(async move {
                            show_toast("Slash lid closed changed".into(), "Failed".into(), hh, s.set_show_on_lid_closed(v).await);
                        });
                    });
                }
            }).map_err(|e| error!("setup_slash_page: upgrade_in_event_loop: {e:?}")).ok();
        }
    });
}
