//! `update_and_notify` is responsible for both notifications *and* updating
//! stored statuses about the system state. This is done through either direct,
//! intoify, zbus notifications or similar methods.
//!
//! This module very much functions like a stand-alone app on its own thread.

use std::fmt::Display;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, error, info, warn};
use notify_rust::{Hint, Notification, Timeout};
use rog_dbus::zbus_platform::PlatformProxy;
use rog_platform::platform::PlatformProfile;
use rog_platform::power::AsusPower;
use serde::{Deserialize, Serialize};
use supergfxctl::pci_device::GfxPower;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::error::Result;

const NOTIF_HEADER: &str = "ROG Control";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct EnabledNotifications {
    pub enabled: bool,
    pub receive_notify_gfx: bool,
    pub receive_notify_gfx_status: bool,
    pub receive_notify_platform_profile: bool,
}

impl Default for EnabledNotifications {
    fn default() -> Self {
        Self {
            enabled: true,
            receive_notify_gfx: true,
            receive_notify_gfx_status: true,
            receive_notify_platform_profile: true,
        }
    }
}

fn start_dpu_status_mon(config: Arc<Mutex<Config>>) {
    use supergfxctl::pci_device::Device;
    let dev = Device::find().unwrap_or_default();
    let mut found_dgpu = false; // just for logging
    for dev in dev {
        if dev.is_dgpu() {
            info!(
                "Found dGPU: {}, starting status notifications",
                dev.pci_id()
            );
            let enabled_notifications_copy = config.clone();
            // Plain old thread is perfectly fine since most of this is potentially blocking
            std::thread::spawn(move || {
                let mut last_status = GfxPower::Unknown;
                loop {
                    std::thread::sleep(Duration::from_millis(1500));
                    if let Ok(status) = dev.get_runtime_status() {
                        if status != GfxPower::Unknown && status != last_status {
                            if let Ok(config) = enabled_notifications_copy.lock() {
                                if !config.notifications.receive_notify_gfx_status
                                    || !config.notifications.enabled
                                {
                                    continue;
                                }
                            }
                            // Required check because status cycles through
                            // active/unknown/suspended
                            do_gpu_status_notif("dGPU status changed:", &status)
                                .show()
                                .unwrap()
                                .on_close(|_| ());
                            debug!("dGPU status changed: {:?}", &status);
                        }
                        last_status = status;
                    }
                }
            });
            found_dgpu = true;
            break;
        }
    }
    if !found_dgpu {
        warn!("Did not find a dGPU on this system, dGPU status won't be avilable");
    }
}

/// Start monitoring for platform profile changes (triggered by Fn+F5 or software)
/// and display an OSD notification when the profile changes.
fn start_platform_profile_mon(config: Arc<Mutex<Config>>, rt: &Runtime) {
    let enabled_notifications_copy = config.clone();
    rt.spawn(async move {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                error!("zbus signal: platform_profile_mon: {e}");
                return;
            }
        };
        let proxy = match PlatformProxy::builder(&conn).build().await {
            Ok(p) => p,
            Err(e) => {
                error!("zbus signal: platform_profile_mon proxy: {e}");
                return;
            }
        };

        // Get initial profile to avoid notification on startup
        let mut last_profile = proxy.platform_profile().await.ok();

        info!("Started platform profile change monitor");
        use futures_util::StreamExt;
        let mut stream = proxy.receive_platform_profile_changed().await;
        while let Some(e) = stream.next().await {
            if let Ok(config) = enabled_notifications_copy.lock() {
                if !config.notifications.enabled
                    || !config.notifications.receive_notify_platform_profile
                {
                    continue;
                }
            }
            if let Ok(new_profile) = e.get().await {
                // Only show notification if profile actually changed
                if last_profile != Some(new_profile) {
                    debug!("Platform profile changed to: {:?}", new_profile);
                    if let Err(e) = do_platform_profile_notif("Power Profile:", &new_profile)
                        .show()
                        .map(|n| n.on_close(|_| ()))
                    {
                        warn!("Failed to show platform profile notification: {e}");
                    }
                    last_profile = Some(new_profile);
                }
            }
        }
    });
}

pub fn start_notifications(
    config: Arc<Mutex<Config>>,
    rt: &Runtime,
) -> Result<Vec<JoinHandle<()>>> {
    // Setup the AC/BAT commands that will run on power status change
    let config_copy = config.clone();
    let blocking = rt.spawn_blocking(move || {
        let power = AsusPower::new()
            .map_err(|e| {
                error!("AsusPower: {e}");
                e
            })
            .unwrap();

        let mut last_state = power.get_online().unwrap_or_default();
        loop {
            if let Ok(p) = power.get_online() {
                let mut ac = String::new();
                let mut bat = String::new();
                if let Ok(config) = config_copy.lock() {
                    ac.clone_from(&config.ac_command);
                    bat.clone_from(&config.bat_command);
                }

                if p == 0 && p != last_state {
                    let prog: Vec<&str> = bat.split_whitespace().collect();
                    if (!prog.is_empty()) && (!prog[0].is_empty()) {
                        let mut cmd = Command::new(prog[0]);

                        for arg in prog.iter().skip(1) {
                            cmd.arg(*arg);
                        }
                        cmd.spawn()
                            .map_err(|e| error!("Battery power command error: {e:?}"))
                            .ok();
                    }
                } else if p != last_state {
                    let prog: Vec<&str> = ac.split_whitespace().collect();
                    if (!prog.is_empty()) && (!prog[0].is_empty()) {
                        let mut cmd = Command::new(prog[0]);

                        for arg in prog.iter().skip(1) {
                            cmd.arg(*arg);
                        }
                        cmd.spawn()
                            .map_err(|e| error!("AC power command error: {e:?}"))
                            .ok();
                    }
                }
                last_state = p;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });

    info!("Attempting to start plain dgpu status monitor");
    start_dpu_status_mon(config.clone());

    info!("Starting platform profile change monitor");
    start_platform_profile_mon(config.clone(), rt);

    // GPU MUX Mode notif
    // TODO: need to get armoury attrs and iter to find
    // let enabled_notifications_copy = config.clone();
    // tokio::spawn(async move {
    //     let conn = zbus::Connection::system().await.map_err(|e| {
    //         error!("zbus signal: receive_notify_gpu_mux_mode: {e}");
    //         e
    //     })?;
    //     let proxy = PlatformProxy::new(&conn).await.map_err(|e| {
    //         error!("zbus signal: receive_notify_gpu_mux_mode: {e}");
    //         e
    //     })?;

    //     let mut actual_mux_mode = GpuMode::Error;
    //     if let Ok(mode) = proxy.gpu_mux_mode().await {
    //         actual_mux_mode = GpuMode::from(mode);
    //     }

    //     info!("Started zbus signal thread: receive_notify_gpu_mux_mode");
    //     while let Some(e) =
    // proxy.receive_gpu_mux_mode_changed().await.next().await {         if let
    // Ok(config) = enabled_notifications_copy.lock() {             if
    // !config.notifications.enabled || !config.notifications.receive_notify_gfx {
    //                 continue;
    //             }
    //         }
    //         if let Ok(out) = e.get().await {
    //             let mode = GpuMode::from(out);
    //             if mode == actual_mux_mode {
    //                 continue;
    //             }
    //             do_mux_notification("Reboot required. BIOS GPU MUX mode set to",
    // &mode).ok();         }
    //     }
    //     Ok::<(), zbus::Error>(())
    // });

    Ok(vec![blocking])
}

fn base_notification<T>(message: &str, data: &T) -> Notification
where
    T: Display,
{
    let mut notif = Notification::new();
    notif
        .appname(NOTIF_HEADER)
        .summary(&format!("{message} {data}"))
        .timeout(Timeout::Milliseconds(3000))
        .hint(Hint::Category("device".into()));
    notif
}

fn do_gpu_status_notif(message: &str, data: &GfxPower) -> Notification {
    let mut notif = base_notification(message, &<&str>::from(data).to_owned());
    let icon = match data {
        GfxPower::Suspended => "asus_notif_blue",
        GfxPower::Off => "asus_notif_green",
        GfxPower::AsusDisabled => "asus_notif_white",
        GfxPower::AsusMuxDiscreet | GfxPower::Active => "asus_notif_red",
        GfxPower::Unknown => "gpu-integrated",
    };
    notif.icon(icon);
    notif
}

/// Create a notification for platform profile (power mode) changes.
/// Uses profile-specific icons and user-friendly names.
fn do_platform_profile_notif(message: &str, profile: &PlatformProfile) -> Notification {
    let profile_name = match profile {
        PlatformProfile::Balanced => "Balanced",
        PlatformProfile::Performance => "Performance",
        PlatformProfile::Quiet => "Quiet",
        PlatformProfile::LowPower => "Low Power",
        PlatformProfile::Custom => "Custom",
    };
    let mut notif = base_notification(message, &profile_name.to_owned());

    // Use appropriate icons for each profile
    // These icons should be available in the system or ROG icon pack
    let icon = match profile {
        PlatformProfile::Balanced => "asus_notif_blue", // Blue for balanced
        PlatformProfile::Performance => "asus_notif_red", // Red for performance
        PlatformProfile::Quiet => "asus_notif_green",   // Green for quiet/power saving
        PlatformProfile::LowPower => "asus_notif_green", // Green for low power
        PlatformProfile::Custom => "asus_notif_white",  // White for custom
    };
    notif.icon(icon);
    notif
}
