use std::fs;
use std::path::{Path, PathBuf};

use log::{error, info};
use serde::{Deserialize, Serialize};

use crate::{FanType, MainWindow, Node};

const ASUS_CUSTOM_FAN_NAME: &str = "asus_custom_fan_curve";
const CONFIG_FILE_NAME: &str = "custom_fans.ron";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCurvePoint {
    pub temp: u8,
    pub pwm: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomFanConfig {
    pub cpu_curve: Vec<CustomCurvePoint>,
    pub gpu_curve: Vec<CustomCurvePoint>,
    pub enabled: bool,
}

#[derive(Clone)]
struct SysfsPaths {
    root: PathBuf,
}

impl SysfsPaths {
    fn new() -> Option<Self> {
        let hwmon = Path::new("/sys/class/hwmon");
        if let Ok(entries) = fs::read_dir(hwmon) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name_path = path.join("name");
                if let Ok(name) = fs::read_to_string(&name_path) {
                    if name.trim() == ASUS_CUSTOM_FAN_NAME {
                        info!("Found ASUS Custom Fan Control at {:?}", path);
                        return Some(Self { root: path });
                    }
                }
            }
        }
        None
    }

    fn enable_path(&self, index: u8) -> PathBuf {
        self.root.join(format!("pwm{}_enable", index))
    }

    fn point_pwm_path(&self, fan_idx: u8, point_idx: u8) -> PathBuf {
        self.root
            .join(format!("pwm{}_auto_point{}_pwm", fan_idx, point_idx))
    }

    fn point_temp_path(&self, fan_idx: u8, point_idx: u8) -> PathBuf {
        self.root
            .join(format!("pwm{}_auto_point{}_temp", fan_idx, point_idx))
    }
}

// Helper to write with logging
fn write_sysfs(path: &Path, value: &str) -> std::io::Result<()> {
    // debug!("Writing {} to {:?}", value, path);
    fs::write(path, value)
}

fn _read_sysfs_u8(path: &Path) -> std::io::Result<u8> {
    let s = fs::read_to_string(path)?;
    s.trim()
        .parse::<u8>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn is_custom_fan_supported() -> bool {
    SysfsPaths::new().is_some()
}

// Logic to apply a full curve to a specific fan (1=CPU, 2=GPU usually)
// Implements the "Gradual Descent" algorithm
fn apply_curve_to_fan(
    paths: &SysfsPaths,
    fan_idx: u8,
    points: &[CustomCurvePoint],
) -> std::io::Result<()> {
    // Sort target points by temp (Hardware Requirement)
    let mut sorted_target = points.to_vec();
    sorted_target.sort_by_key(|p| p.temp);

    // Ensure we have 8 points (fill with last if needed, or sensible default)
    while sorted_target.len() < 8 {
        if let Some(last) = sorted_target.last() {
            sorted_target.push(last.clone());
        } else {
            sorted_target.push(CustomCurvePoint {
                temp: 100,
                pwm: 255,
            });
        }
    }
    sorted_target.truncate(8);

    // Validate Temp Order (Synchronous Check)
    for (i, p) in sorted_target.iter().enumerate() {
        if i > 0 {
            let prev_temp = sorted_target[i - 1].temp;
            if p.temp < prev_temp {
                error!("Invalid temp order");
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Temp disorder",
                ));
            }
        }
    }

    // Spawn completely detached thread for ALL I/O
    let paths_clone = paths.clone();
    let sorted_target = sorted_target.clone();

    std::thread::spawn(move || {
        let paths = paths_clone;

        // 1. Enable custom mode
        if let Err(e) = write_sysfs(&paths.enable_path(fan_idx), "1") {
            error!("Failed to enable custom fan mode: {}", e);
            return;
        }

        // 2. Write Temps
        for (i, p) in sorted_target.iter().enumerate() {
            let point_idx = (i + 1) as u8;
            if let Err(e) = write_sysfs(
                &paths.point_temp_path(fan_idx, point_idx),
                &p.temp.to_string(),
            ) {
                error!("Failed to write temp point {}: {}", point_idx, e);
            }
        }

        // 3. Write PWMs directly (hardware handles gradual transition)
        for (i, target_p) in sorted_target.iter().enumerate() {
            let point_idx = (i + 1) as u8;
            if let Err(e) = write_sysfs(
                &paths.point_pwm_path(fan_idx, point_idx),
                &target_p.pwm.to_string(),
            ) {
                error!("Failed to write PWM point {}: {}", point_idx, e);
            }
        }

        // 4. Ensure enable is set
        let _ = write_sysfs(&paths.enable_path(fan_idx), "1");
    });

    Ok(())
}

fn set_fan_auto(paths: &SysfsPaths, fan_idx: u8) -> std::io::Result<()> {
    // 2 = Auto (usually)
    write_sysfs(&paths.enable_path(fan_idx), "2")
}

fn load_config() -> CustomFanConfig {
    if let Some(config_dir) = dirs::config_dir() {
        let path = config_dir.join("rog").join(CONFIG_FILE_NAME);
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(cfg) = ron::from_str(&content) {
                return cfg;
            }
        }
    }
    CustomFanConfig::default()
}

fn save_config(config: &CustomFanConfig) {
    if let Some(config_dir) = dirs::config_dir() {
        let rog_dir = config_dir.join("rog");
        let _ = fs::create_dir_all(&rog_dir);
        let path = rog_dir.join(CONFIG_FILE_NAME);
        if let Ok(s) = ron::ser::to_string_pretty(config, ron::ser::PrettyConfig::default()) {
            let _ = fs::write(path, s);
        }
    }
}

// Public entry point called from setup_fans.rs or similar
// Returns immediately - all work is done in a detached thread
pub fn apply_custom_fan_curve(
    _handle_weak: slint::Weak<MainWindow>,
    fan_type: FanType,
    enabled: bool,
    nodes: Vec<Node>,
) {
    // Fan Index: 1=CPU, 2=GPU usually.
    let fan_idx = match fan_type {
        FanType::CPU => 1,
        FanType::GPU => 2,
        _ => return, // Ignore others
    };

    // Convert nodes to points (fast, CPU-only)
    let points: Vec<CustomCurvePoint> = nodes
        .iter()
        .map(|n| CustomCurvePoint {
            temp: n.x as u8,
            pwm: n.y as u8,
        })
        .collect();

    // Spawn a completely detached thread for ALL I/O
    std::thread::spawn(move || {
        // Get paths (blocking FS operation)
        let Some(paths) = SysfsPaths::new() else {
            error!("No custom fan support found");
            return;
        };

        // Save config
        let mut cfg = load_config();
        if enabled {
            match fan_type {
                FanType::CPU => cfg.cpu_curve = points.clone(),
                FanType::GPU => cfg.gpu_curve = points.clone(),
                _ => {}
            }
        }
        cfg.enabled = enabled;
        save_config(&cfg);

        // Apply curve or set auto
        if enabled {
            if let Err(e) = apply_curve_to_fan(&paths, fan_idx, &points) {
                error!("Failed to apply fan curve: {}", e);
            }
        } else if let Err(e) = set_fan_auto(&paths, fan_idx) {
            error!("Failed to set fan auto: {}", e);
        }
    });
}
