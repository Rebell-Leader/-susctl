use std::sync::{Arc, Mutex};

use log::{debug, error, info};
use rog_aura::animation::AnimationMode;
use rog_aura::keyboard::LaptopAuraPower;
use rog_aura::{AuraDeviceType, Colour, PowerZones};
use rog_dbus::zbus_aura::AuraProxy;
use slint::{ComponentHandle, Model, RgbaColor, SharedString};

use crate::config::Config;
use crate::ui::show_toast;
use crate::{
    set_ui_callbacks, set_ui_props_async, AuraPageData, MainWindow, PowerZones as SlintPowerZones,
};

fn decode_hex(s: &str) -> RgbaColor<u8> {
    let s = s.trim_start_matches('#');
    if s.len() < 6 {
        return RgbaColor {
            alpha: 255,
            red: 0,
            green: 0,
            blue: 0,
        };
    }
    let c: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(164))
        .collect();
    RgbaColor {
        alpha: 255,
        red: *c.first().unwrap_or(&255),
        green: *c.get(1).unwrap_or(&128),
        blue: *c.get(2).unwrap_or(&32),
    }
}

/// Returns the first available Aura interface
// TODO: return all
async fn find_aura_iface() -> Result<AuraProxy<'static>, Box<dyn std::error::Error>> {
    let conn = zbus::Connection::system().await?;
    let f = zbus::fdo::ObjectManagerProxy::new(&conn, "xyz.ljones.Asusd", "/").await?;
    let interfaces = f.get_managed_objects().await?;
    let mut aura_paths = Vec::new();
    for v in interfaces.iter() {
        for k in v.1.keys() {
            if k.as_str() == "xyz.ljones.Aura" {
                println!("Found aura device at {}, {}", v.0, k);
                aura_paths.push(v.0.clone());
            }
        }
    }
    if aura_paths.len() > 1 {
        println!("Multiple aura devices found: {aura_paths:?}");
        println!("TODO: enable selection");
    }
    if let Some(path) = aura_paths.first() {
        return Ok(AuraProxy::builder(&conn)
            .path(path.clone())?
            .destination("xyz.ljones.Asusd")?
            .build()
            .await?);
    }

    Err("No Aura interface".into())
}

pub fn setup_aura_page(ui: &MainWindow, _states: Arc<Mutex<Config>>) {
    ui.global::<AuraPageData>().on_cb_hex_from_colour(|c| {
        format!("#{:02X}{:02X}{:02X}", c.red(), c.green(), c.blue()).into()
    });

    ui.global::<AuraPageData>()
        .on_cb_hex_to_colour(|s| decode_hex(s.as_str()).into());

    let handle = ui.as_weak();
    tokio::spawn(async move {
        let Ok(aura) = find_aura_iface().await else {
            info!("This device appears to have no aura interfaces");
            return Ok::<(), zbus::Error>(());
        };

        set_ui_props_async!(handle, aura, AuraPageData, brightness);
        set_ui_props_async!(handle, aura, AuraPageData, led_mode);
        set_ui_props_async!(handle, aura, AuraPageData, led_mode_data);
        set_ui_props_async!(handle, aura, AuraPageData, led_power);
        set_ui_props_async!(handle, aura, AuraPageData, device_type);

        if let Ok(mut pow3r) = aura.supported_power_zones().await {
            let dev_type = aura
                .device_type()
                .await
                .unwrap_or(AuraDeviceType::LaptopKeyboard2021);
            log::debug!("Available LED power modes {pow3r:?}");
            handle
                .upgrade_in_event_loop(move |handle| {
                    let names: Vec<SharedString> = handle
                        .global::<AuraPageData>()
                        .get_power_zone_names()
                        .iter()
                        .collect();

                    if dev_type.is_old_laptop() {
                        // Need to add the specific KeyboardAndLightbar
                        if pow3r.contains(&PowerZones::Keyboard)
                            && pow3r.contains(&PowerZones::Lightbar)
                        {
                            pow3r.push(PowerZones::KeyboardAndLightbar);
                        }
                        let names: Vec<SharedString> =
                            pow3r.iter().map(|n| names[(*n) as usize].clone()).collect();
                        handle
                            .global::<AuraPageData>()
                            .set_power_zone_names_old(names.as_slice().into());
                    } else {
                        let power: Vec<SlintPowerZones> =
                            pow3r.iter().map(|p| (*p).into()).collect();

                        handle
                            .global::<AuraPageData>()
                            .set_supported_power_zones(power.as_slice().into());
                    }
                })
                .ok();
        }

        if let Ok(modes) = aura.supported_basic_modes().await {
            log::debug!("Available LED modes {modes:?}");

            // Check if only Static mode is available (enable software animation)
            let static_only = modes.len() == 1 && modes.iter().any(|m| *m == 0.into());

            // Clone proxy for callbacks
            let aura_for_animation = aura.clone();

            handle
                .upgrade_in_event_loop(move |handle| {
                    let m: Vec<i32> = modes.iter().map(|n| (*n).into()).collect();
                    handle
                        .global::<AuraPageData>()
                        .set_supported_basic_modes(m.as_slice().into());
                    // Get the translated names
                    let names = handle.global::<AuraPageData>().get_mode_names();

                    let res: Vec<SharedString> = names
                        .iter()
                        .enumerate()
                        .filter(|(n, _)| modes.contains(&(*n as i32).into()) && *n != 9)
                        .map(|(_, i)| i)
                        .collect();
                    handle
                        .global::<AuraPageData>()
                        .set_available_mode_names(res.as_slice().into());

                    // Enable software animation if only Static mode is available
                    if static_only {
                        info!("Only Static mode available - enabling software animation controls");
                        handle
                            .global::<AuraPageData>()
                            .set_soft_animation_available(true);

                        // Connect mode callback - uses DBus to start animation in daemon
                        let aura_mode = aura_for_animation.clone();
                        let handle_weak = handle.as_weak();
                        handle
                            .global::<AuraPageData>()
                            .on_cb_soft_animation_mode(move |mode| {
                                let aura_inner = aura_mode.clone();
                                let handle = match handle_weak.upgrade() {
                                    Some(h) => h,
                                    None => return,
                                };

                                let data = handle.global::<AuraPageData>().get_led_mode_data();
                                let c1 = data.colour1;
                                let c2 = data.colour2;

                                let c1_rog = Colour {
                                    r: c1.red(),
                                    g: c1.green(),
                                    b: c1.blue(),
                                };
                                let c2_rog = Colour {
                                    r: c2.red(),
                                    g: c2.green(),
                                    b: c2.blue(),
                                };

                                let anim_mode = match mode {
                                    1 => AnimationMode::Rainbow { speed_ms: 100 },
                                    2 => AnimationMode::ColorCycle {
                                        speed_ms: 200,
                                        colors: vec![
                                            Colour { r: 255, g: 0, b: 0 },
                                            Colour { r: 0, g: 255, b: 0 },
                                            Colour { r: 0, g: 0, b: 255 },
                                        ],
                                    },
                                    3 => AnimationMode::Breathe {
                                        speed_ms: 100,
                                        color1: c1_rog,
                                        color2: c2_rog,
                                    },
                                    4 => AnimationMode::Pulse {
                                        speed_ms: 50,
                                        color: c1_rog,
                                        min_brightness: 0.2,
                                        max_brightness: 1.0,
                                    },
                                    _ => AnimationMode::None,
                                };
                                tokio::spawn(async move {
                                    let json =
                                        serde_json::to_string(&anim_mode).unwrap_or_default();
                                    if anim_mode == AnimationMode::None {
                                        if let Err(e) = aura_inner.stop_animation().await {
                                            error!("Failed to stop animation: {e}");
                                        }
                                    } else {
                                        if let Err(e) = aura_inner.start_animation(json).await {
                                            error!("Failed to start animation: {e}");
                                        }
                                    }
                                });
                            });
                    }
                })
                .map_err(|e| error!("{e:}"))
                .ok();
        }

        let proxy_copy = aura.clone();
        handle
            .upgrade_in_event_loop(move |handle| {
                set_ui_callbacks!(handle,
                    AuraPageData(.into()),
                    proxy_copy.brightness(.into()),
                    "Keyboard LED brightness successfully set to {}",
                    "Setting keyboard LED brightness failed"
                );

                set_ui_callbacks!(handle,
                    AuraPageData(.into()),
                    proxy_copy.led_mode(.into()),
                    "Keyboard LED mode successfully set to {}",
                    "Setting keyboard LEDmode failed"
                );

                let proxy_data = proxy_copy.clone();
                let aura_soft = proxy_copy.clone();
                let handle_weak = handle.as_weak();

                handle
                    .global::<AuraPageData>()
                    .on_cb_led_mode_data(move |data| {
                        // 1. Update hardware mode
                        let p = proxy_data.clone();
                        let d = data.clone();
                        tokio::spawn(async move {
                            if let Err(e) = p.set_led_mode_data(d.into()).await {
                                error!("Setting keyboard LED mode failed: {e}");
                            } else {
                                debug!("Keyboard LED mode set");
                            }
                        });

                        // 2. Update software animation if active
                        let handle = match handle_weak.upgrade() {
                            Some(h) => h,
                            None => return,
                        };

                        let soft_mode = handle.global::<AuraPageData>().get_soft_animation_mode();
                        if soft_mode != 0 {
                            let c1 = data.colour1;
                            let c2 = data.colour2;
                            let c1_rog = Colour {
                                r: c1.red(),
                                g: c1.green(),
                                b: c1.blue(),
                            };
                            let c2_rog = Colour {
                                r: c2.red(),
                                g: c2.green(),
                                b: c2.blue(),
                            };

                            let anim_mode = match soft_mode {
                                1 => AnimationMode::Rainbow { speed_ms: 100 },
                                2 => AnimationMode::ColorCycle {
                                    speed_ms: 200,
                                    colors: vec![
                                        Colour { r: 255, g: 0, b: 0 },
                                        Colour { r: 0, g: 255, b: 0 },
                                        Colour { r: 0, g: 0, b: 255 },
                                    ],
                                },
                                3 => AnimationMode::Breathe {
                                    speed_ms: 100,
                                    color1: c1_rog,
                                    color2: c2_rog,
                                },
                                4 => AnimationMode::Pulse {
                                    speed_ms: 50,
                                    color: c1_rog,
                                    min_brightness: 0.2,
                                    max_brightness: 1.0,
                                },
                                _ => AnimationMode::None,
                            };

                            let aura_s = aura_soft.clone();
                            tokio::spawn(async move {
                                if let Ok(json) = serde_json::to_string(&anim_mode) {
                                    if let Err(e) = aura_s.start_animation(json).await {
                                        error!("Failed to update software animation: {e}");
                                    }
                                }
                            });
                        }
                    });

                // set_ui_callbacks!(handle,
                //     AuraPageData(.clone().into()),
                //     proxy_copy.led_power(.into()),
                //     "Keyboard LED power successfully set to {:?}",
                //     "Setting keyboard power failed"
                // );

                handle.invoke_external_colour_change();
            })
            .ok();

        let handle_copy = handle.clone();
        let proxy_copy = aura.clone();
        handle
            .upgrade_in_event_loop(|handle| {
                handle
                    .global::<AuraPageData>()
                    .on_cb_led_power(move |power| {
                        let handle_copy = handle_copy.clone();
                        let proxy_copy = aura.clone();
                        let power: LaptopAuraPower = power.into();
                        tokio::spawn(async move {
                            show_toast(
                                "Aura power settings changed".into(),
                                "Failed to set Aura power settings".into(),
                                handle_copy,
                                proxy_copy.set_led_power(power).await,
                            );
                        });
                    });
            })
            .map_err(|e| error!("{e:}"))
            .ok();

        // Need to update the UI if the mode changes
        let handle_copy = handle.clone();
        // spawn required since the while let never exits
        tokio::spawn(async move {
            let mut x = proxy_copy.receive_led_mode_data_changed().await;
            use futures_util::StreamExt;
            while let Some(e) = x.next().await {
                if let Ok(out) = e.get().await {
                    handle_copy
                        .upgrade_in_event_loop(move |handle| {
                            handle
                                .global::<AuraPageData>()
                                .invoke_update_led_mode_data(out.into());
                            handle.invoke_external_colour_change();
                        })
                        .map_err(|e| error!("{e:}"))
                        .ok();
                }
            }
        });
        debug!("Aura setup tasks complete");
        Ok(())
    });
}
