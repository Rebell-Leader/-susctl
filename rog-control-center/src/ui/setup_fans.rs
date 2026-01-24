use crate::ui::show_toast;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use log::error;
use rog_dbus::zbus_fan_curves::FanCurvesProxy;
use rog_dbus::zbus_platform::PlatformProxy;
use rog_platform::platform::PlatformProfile;
use rog_profiles::fan_curve_set::CurveData;
use slint::{ComponentHandle, Model, Weak};

use crate::config::Config;
use crate::{FanPageData, FanType, MainWindow, Node, Profile};

// Isolated Rust-side cache for fan curves (not affected by Slint reactivity)
type FanCacheKey = (i32, i32); // (Profile as i32, FanType as i32)
static FAN_CACHE: OnceLock<Mutex<HashMap<FanCacheKey, Vec<Node>>>> = OnceLock::new();

fn fan_cache() -> &'static Mutex<HashMap<FanCacheKey, Vec<Node>>> {
    FAN_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_fan_curve(profile: Profile, fan_type: FanType, nodes: Vec<Node>) {
    let key = (profile as i32, fan_type as i32);
    fan_cache().lock().unwrap().insert(key, nodes);
}

fn get_cached_fan_curve(profile: Profile, fan_type: FanType) -> Option<Vec<Node>> {
    let key = (profile as i32, fan_type as i32);
    fan_cache().lock().unwrap().get(&key).cloned()
}

pub fn update_fan_data(
    handle: Weak<MainWindow>,
    bal: Vec<CurveData>,
    perf: Vec<CurveData>,
    quiet: Vec<CurveData>,
) {
    handle
        .upgrade_in_event_loop(move |handle| {
            let global = handle.global::<FanPageData>();
            let _collect = |temp: &[u8], pwm: &[u8]| -> slint::ModelRc<Node> {
                let tmp: Vec<Node> = temp
                    .iter()
                    .zip(pwm.iter())
                    .map(|(x, y)| Node {
                        x: *x as f32,
                        y: *y as f32,
                    })
                    .collect();
                tmp.as_slice().into()
            };

            for fan in bal {
                global.set_balanced_available(true);
                let nodes_vec: Vec<Node> = fan
                    .temp
                    .iter()
                    .zip(fan.pwm.iter())
                    .map(|(x, y)| Node {
                        x: *x as f32,
                        y: *y as f32,
                    })
                    .collect();
                let nodes: slint::ModelRc<Node> = nodes_vec.as_slice().into();
                match fan.fan {
                    rog_profiles::FanCurvePU::CPU => {
                        global.set_cpu_fan_available(true);
                        global.set_balanced_cpu_enabled(fan.enabled);
                        global.set_balanced_cpu(nodes.clone());
                        cache_fan_curve(Profile::Balanced, FanType::CPU, nodes_vec);
                    }
                    rog_profiles::FanCurvePU::GPU => {
                        global.set_gpu_fan_available(true);
                        global.set_balanced_gpu_enabled(fan.enabled);
                        global.set_balanced_gpu(nodes.clone());
                        cache_fan_curve(Profile::Balanced, FanType::GPU, nodes_vec);
                    }
                    rog_profiles::FanCurvePU::MID => {
                        global.set_mid_fan_available(true);
                        global.set_balanced_mid_enabled(fan.enabled);
                        global.set_balanced_mid(nodes.clone());
                        cache_fan_curve(Profile::Balanced, FanType::Middle, nodes_vec);
                    }
                }
            }
            for fan in perf {
                global.set_performance_available(true);
                let nodes_vec: Vec<Node> = fan
                    .temp
                    .iter()
                    .zip(fan.pwm.iter())
                    .map(|(x, y)| Node {
                        x: *x as f32,
                        y: *y as f32,
                    })
                    .collect();
                let nodes: slint::ModelRc<Node> = nodes_vec.as_slice().into();
                match fan.fan {
                    rog_profiles::FanCurvePU::CPU => {
                        global.set_cpu_fan_available(true);
                        global.set_performance_cpu_enabled(fan.enabled);
                        global.set_performance_cpu(nodes.clone());
                        cache_fan_curve(Profile::Performance, FanType::CPU, nodes_vec);
                    }
                    rog_profiles::FanCurvePU::GPU => {
                        global.set_gpu_fan_available(true);
                        global.set_performance_gpu_enabled(fan.enabled);
                        global.set_performance_gpu(nodes.clone());
                        cache_fan_curve(Profile::Performance, FanType::GPU, nodes_vec);
                    }
                    rog_profiles::FanCurvePU::MID => {
                        global.set_mid_fan_available(true);
                        global.set_performance_mid_enabled(fan.enabled);
                        global.set_performance_mid(nodes.clone());
                        cache_fan_curve(Profile::Performance, FanType::Middle, nodes_vec);
                    }
                }
            }
            for fan in quiet {
                global.set_quiet_available(true);
                let nodes_vec: Vec<Node> = fan
                    .temp
                    .iter()
                    .zip(fan.pwm.iter())
                    .map(|(x, y)| Node {
                        x: *x as f32,
                        y: *y as f32,
                    })
                    .collect();
                let nodes: slint::ModelRc<Node> = nodes_vec.as_slice().into();
                match fan.fan {
                    rog_profiles::FanCurvePU::CPU => {
                        global.set_cpu_fan_available(true);
                        global.set_quiet_cpu_enabled(fan.enabled);
                        global.set_quiet_cpu(nodes.clone());
                        cache_fan_curve(Profile::Quiet, FanType::CPU, nodes_vec);
                    }
                    rog_profiles::FanCurvePU::GPU => {
                        global.set_gpu_fan_available(true);
                        global.set_quiet_gpu_enabled(fan.enabled);
                        global.set_quiet_gpu(nodes.clone());
                        cache_fan_curve(Profile::Quiet, FanType::GPU, nodes_vec);
                    }
                    rog_profiles::FanCurvePU::MID => {
                        global.set_mid_fan_available(true);
                        global.set_quiet_mid_enabled(fan.enabled);
                        global.set_quiet_mid(nodes.clone());
                        cache_fan_curve(Profile::Quiet, FanType::Middle, nodes_vec);
                    }
                }
            }
        })
        .map_err(|e| error!("update_fan_data: upgrade_in_event_loop: {e:?}"))
        .ok();
}

pub fn setup_fan_curve_page(ui: &MainWindow, _config: Arc<Mutex<Config>>) {
    let handle = ui.as_weak();

    tokio::spawn(async move {
        // Create the connections/proxies here to prevent future delays in process
        let conn = match zbus::Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("{e:}");
                return;
            }
        };

        let fans = match FanCurvesProxy::new(&conn).await {
            Ok(fans) => fans,
            Err(e) => {
                error!("{e:}");
                return;
            }
        };

        let platform = match PlatformProxy::new(&conn).await {
            Ok(platform) => platform,
            Err(e) => {
                error!("{e:}");
                return;
            }
        };

        let platform_profile_choices = match platform.platform_profile_choices().await {
            Ok(choices) => choices,
            Err(e) => {
                error!("{e:}");
                return;
            }
        };

        let handle_copy = handle.clone();
        // Do initial setup
        let balanced = match fans.fan_curve_data(PlatformProfile::Balanced).await {
            Ok(data) => data,
            Err(e) => {
                error!("Couldn't get balanced data: {e:}");
                return;
            }
        };

        let perf = match fans.fan_curve_data(PlatformProfile::Performance).await {
            Ok(data) => data,
            Err(e) => {
                error!("Couldn't get performance data: {e:}");
                return;
            }
        };

        // TODO: the fan curve stuff was written donkeys ago with the expectation that
        // only 3 profiles existed
        let profile = if platform_profile_choices.contains(&PlatformProfile::Quiet) {
            PlatformProfile::Quiet
        } else {
            PlatformProfile::LowPower
        };
        let quiet = match fans.fan_curve_data(profile).await {
            Ok(data) => data,
            Err(e) => {
                error!("Couldn't get quiet data: {e:}");
                return;
            }
        };

        update_fan_data(handle, balanced, perf, quiet);

        let choices_for_ui = platform_profile_choices.clone();
        let handle_next1 = handle_copy.clone();
        if let Err(e) = handle_copy.upgrade_in_event_loop(move |handle| {
            let handle_weak_for_fans = handle.as_weak();
            let global = handle.global::<FanPageData>();
            let fans1 = fans.clone();
            let choices = choices_for_ui.clone();
            global.on_set_profile_default(move |profile| {
                let fans = fans1.clone();
                let handle_next = handle_next1.clone();
                let choices = choices.clone();
                tokio::spawn(async move {
                    let mut target: PlatformProfile = profile.into();
                    if target == PlatformProfile::Quiet
                        && !choices.contains(&PlatformProfile::Quiet)
                    {
                        target = PlatformProfile::LowPower;
                    }
                    if fans.set_curves_to_defaults(target).await.is_err() {
                        return;
                    }
                    let Ok(balanced) = fans
                        .fan_curve_data(PlatformProfile::Balanced)
                        .await
                        .map_err(|e| error!("{e:}"))
                    else {
                        return;
                    };
                    let Ok(perf) = fans
                        .fan_curve_data(PlatformProfile::Performance)
                        .await
                        .map_err(|e| error!("{e:}"))
                    else {
                        return;
                    };
                    let Ok(quiet) = fans
                        .fan_curve_data(PlatformProfile::Quiet)
                        .await
                        .map_err(|e| error!("{e:}"))
                    else {
                        return;
                    };
                    update_fan_data(handle_next, balanced, perf, quiet);
                });
            });

            let handle_weak_for_cancel = handle_weak_for_fans.clone();
            global.on_set_fan_data(move |fan, profile, enabled, data| {
                if crate::ui::setup_fan_curve_custom::is_custom_fan_supported() {
                    let handle_weak = handle_weak_for_fans.clone();
                    let data: Vec<Node> = data.iter().collect();

                    use log::info;
                    info!("MainThread: Request to apply custom curve for {:?}", fan);

                    // Explicitly spawn a thread to handle this, preventing ANY main thread blocking
                    std::thread::spawn(move || {
                        info!("WorkerThread: applying curve for {:?}", fan);
                        crate::ui::setup_fan_curve_custom::apply_custom_fan_curve(
                            handle_weak.clone(),
                            fan,
                            enabled,
                            data,
                        );
                        info!("WorkerThread: returned from apply (async), clearing busy flag for {:?}", fan);

                        // Clear busy flag
                        let _ = handle_weak.upgrade_in_event_loop(move |h| {
                            let g = h.global::<FanPageData>();
                            match fan {
                                FanType::CPU => g.set_is_busy_cpu(false),
                                FanType::GPU => g.set_is_busy_gpu(false),
                                FanType::Middle => g.set_is_busy_mid(false),
                            }
                            info!("MainThread: cleared busy flag for {:?}", fan);
                        });
                    });

                    return;
                }

                let fans = fans.clone();
                let handle_weak = handle_weak_for_fans.clone();
                let nodes_vec: Vec<Node> = data.iter().collect();
                let _data_copy = nodes_vec.clone();
                let cache_copy = nodes_vec.clone(); // Clone for cache update
                let fan_data = fan_data_for(fan, enabled, nodes_vec);
                tokio::spawn(async move {
                    show_toast(
                        "Fan curve applied".into(),
                        "Failed to apply fan curve".into(),
                        handle_weak.clone(),
                        fans.set_fan_curve(profile.into(), fan_data).await,
                    );
                    let _ = handle_weak.upgrade_in_event_loop(move |h| {
                        let g = h.global::<FanPageData>();
                        // Update Rust-side cache (isolated from Slint properties)
                        cache_fan_curve(profile, fan, cache_copy);

                        match fan {
                            FanType::CPU => g.set_is_busy_cpu(false),
                            FanType::GPU => g.set_is_busy_gpu(false),
                            FanType::Middle => g.set_is_busy_mid(false),
                        }
                    });
                });
            });
            global.on_cancel(move |fan_type, profile| {
                let handle_weak = handle_weak_for_cancel.clone();
                let _ = handle_weak.upgrade_in_event_loop(move |h: MainWindow| {
                    let global = h.global::<FanPageData>();

                    // Retrieve from isolated Rust cache
                    let nodes_opt = get_cached_fan_curve(profile, fan_type);

                    if let Some(nodes_vec) = nodes_opt {
                        use log::info;
                        info!("Canceling {:?} {:?} - restoring {} nodes from isolated cache", fan_type, profile, nodes_vec.len());

                        let new_model: slint::ModelRc<Node> = nodes_vec.as_slice().into();

                        match (profile, fan_type) {
                            (crate::Profile::Balanced, FanType::CPU) => global.set_balanced_cpu(new_model),
                            (crate::Profile::Balanced, FanType::GPU) => global.set_balanced_gpu(new_model),
                            (crate::Profile::Balanced, FanType::Middle) => global.set_balanced_mid(new_model),
                            (crate::Profile::Performance, FanType::CPU) => global.set_performance_cpu(new_model),
                            (crate::Profile::Performance, FanType::GPU) => global.set_performance_gpu(new_model),
                            (crate::Profile::Performance, FanType::Middle) => global.set_performance_mid(new_model),
                            (crate::Profile::Quiet, FanType::CPU) => global.set_quiet_cpu(new_model),
                            (crate::Profile::Quiet, FanType::GPU) => global.set_quiet_gpu(new_model),
                            (crate::Profile::Quiet, FanType::Middle) => global.set_quiet_mid(new_model),
                            _ => {}
                        }
                    } else {
                        log::warn!("Cancel failed: No cached data for {:?} {:?}", fan_type, profile);
                    }
                });
            });
            // Initialize warning
            if crate::ui::setup_fan_curve_custom::is_custom_fan_supported() {
                global.set_show_custom_warning(true);
            }
        }) {
            error!("setup_fan_curve_page: upgrade_in_event_loop: {e:?}");
        }
    });
}

fn fan_data_for(fan: FanType, enabled: bool, data: Vec<Node>) -> CurveData {
    let mut temp = [0u8; 8];
    let mut pwm = [0u8; 8];
    for (i, n) in data.iter().enumerate() {
        if i == 8 {
            break;
        }
        temp[i] = n.x as u8;
        pwm[i] = n.y as u8;
    }

    dbg!(&fan, enabled);

    CurveData {
        fan: fan.into(),
        pwm,
        temp,
        enabled,
    }
}
