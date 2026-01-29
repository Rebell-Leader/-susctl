use crate::config::Config;
use crate::tray::TrayStats;
use crate::{MainWindow, SystemStatus};
use rog_dbus::zbus_platform::PlatformProxy;
use slint::ComponentHandle;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::fs;
use tokio::time::Duration;
use zbus::Connection;

pub fn setup_status(
    ui: &MainWindow,
    _config: Arc<Mutex<Config>>,
    stats_tx: tokio::sync::watch::Sender<TrayStats>,
) {
    let ui_weak = ui.as_weak();

    tokio::spawn(async move {
        let mut power_history: VecDeque<i32> = VecDeque::with_capacity(150); // 300s window at 2s poll
                                                                             // DBus connection for profile
        let conn = Connection::system().await.ok();
        let platform = if let Some(c) = &conn {
            PlatformProxy::new(c).await.ok()
        } else {
            None
        };

        loop {
            let (cpu_temp, gpu_temp, cpu_fan, gpu_fan) = read_hwmon().await;
            let power_microwatts = read_power().await;

            // Rolling average logic
            if power_history.len() >= 150 {
                power_history.pop_front();
            }
            power_history.push_back(power_microwatts);

            let sum: i64 = power_history.iter().map(|&x| x as i64).sum();
            let avg_microwatts = if !power_history.is_empty() {
                sum / power_history.len() as i64
            } else {
                0
            };

            // Convert to Watts
            let power_w = power_microwatts as f64 / 1_000_000.0;
            let avg_w = avg_microwatts as f64 / 1_000_000.0;

            // Fetch profile
            let mut profile_str = "Unknown".to_string();
            if let Some(p) = &platform {
                if let Ok(prof) = p.platform_profile().await {
                    profile_str = format!("{:?}", prof);
                }
            }
            let ui_weak_loop = ui_weak.clone(); // Clone ui_weak for this iteration

            // Check dGPU state
            let is_dgpu_active = get_dgpu_state();

            // Send to Tray
            let _ = stats_tx.send(TrayStats {
                cpu_temp: format!("{}", cpu_temp),
                gpu_temp: format!("{}", gpu_temp),
                cpu_fan: format!("{}", cpu_fan),
                gpu_fan: format!("{}", gpu_fan),
                power_w: format!("{:.1}", power_w),
                power_profile: profile_str,
                is_dgpu_active,
            });

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_loop.upgrade() {
                    let global = ui.global::<SystemStatus>();
                    global.set_cpu_temp(cpu_temp);
                    global.set_gpu_temp(gpu_temp);
                    global.set_cpu_fan(cpu_fan);
                    global.set_gpu_fan(gpu_fan);

                    global.set_power_w(slint::SharedString::from(format!("{:.1}", power_w)));
                    global.set_power_avg_w(slint::SharedString::from(format!("{:.1}", avg_w)));
                }
            });

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn read_hwmon() -> (i32, i32, i32, i32) {
    let mut cpu_temp = 0;
    let mut gpu_temp = 0;
    let mut cpu_fan = 0;
    let mut gpu_fan = 0;

    let mut entries = match fs::read_dir("/sys/class/hwmon").await {
        Ok(e) => e,
        Err(_) => return (0, 0, 0, 0),
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name_path = path.join("name");

        if let Ok(name_str) = fs::read_to_string(&name_path).await {
            let name = name_str.trim();

            if name == "k10temp" || name == "coretemp" || name == "zenpower" {
                // Try temp1_input (TCtl/Package)
                if let Ok(temp) = read_val(&path.join("temp1_input")).await {
                    cpu_temp = temp / 1000;
                }
            } else if name == "amdgpu" || name == "nvidia" {
                if let Ok(temp) = read_val(&path.join("temp1_input")).await {
                    gpu_temp = temp / 1000;
                }
            } else if name == "asus" || name == "asus_custom_fan_curve" {
                if let Ok(fan) = read_val(&path.join("fan1_input")).await {
                    cpu_fan = fan;
                }
                if let Ok(fan) = read_val(&path.join("fan2_input")).await {
                    gpu_fan = fan;
                }
            }
        }
    }

    (cpu_temp, gpu_temp, cpu_fan, gpu_fan)
}

async fn read_val(path: &PathBuf) -> Result<i32, ()> {
    let s = fs::read_to_string(path).await.map_err(|_| ())?;
    s.trim().parse::<i32>().map_err(|_| ())
}

async fn read_power() -> i32 {
    let mut p = 0;
    // Try BAT0 then BAT1
    if let Ok(v) = read_val(&PathBuf::from("/sys/class/power_supply/BAT0/power_now")).await {
        p = v.abs();
    } else if let Ok(v) = read_val(&PathBuf::from("/sys/class/power_supply/BAT1/power_now")).await {
        p = v.abs();
    }

    // Check status
    if let Ok(s) = fs::read_to_string("/sys/class/power_supply/BAT0/status").await {
        if s.trim() == "Discharging" {
            return -p;
        }
    }
    p
}

// Simple check: if /sys/class/drm/card1 exists, dGPU is likely active/available.
// Integrated is always card0.
fn get_dgpu_state() -> bool {
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        let mut card_count = 0;
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if name.starts_with("card")
                    && name.chars().nth(4).is_some_and(|c| c.is_ascii_digit())
                    && !name.contains('-')
                {
                    card_count += 1;
                }
            }
        }
        return card_count > 1;
    }
    true // Fallback to true (Red) if check fails
}
