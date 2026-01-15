//! Software-based keyboard animation for keyboards that only support Static mode.
//! Provides Rainbow and Color Cycle animations via timer-based color updates.

use log::{info, warn};
use slint::Weak;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::MainWindow;

/// Animation mode enum matching the UI
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum AnimationMode {
    #[default]
    None,
    Rainbow,
    ColorCycle,
}

impl From<i32> for AnimationMode {
    fn from(v: i32) -> Self {
        match v {
            1 => AnimationMode::Rainbow,
            2 => AnimationMode::ColorCycle,
            _ => AnimationMode::None,
        }
    }
}

/// Shared state for the animator
pub struct AnimatorState {
    /// Current animation mode
    pub mode: AtomicU32,
    /// Animation speed in milliseconds (update interval)
    pub speed_ms: AtomicU32,
    /// Stop signal
    pub stop: AtomicBool,
    /// Current hue for rainbow mode (0-360)
    hue: AtomicU32,
}

impl Default for AnimatorState {
    fn default() -> Self {
        Self {
            mode: AtomicU32::new(0),
            speed_ms: AtomicU32::new(200),
            stop: AtomicBool::new(false),
            hue: AtomicU32::new(0),
        }
    }
}

/// Convert HSV to RGB (H: 0-360, S: 0-100, V: 0-100)
fn hsv_to_rgb(h: u32, s: u32, v: u32) -> (u8, u8, u8) {
    let s = s as f32 / 100.0;
    let v = v as f32 / 100.0;
    let c = v * s;
    let h_prime = (h as f32 / 60.0) % 6.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let m = v - c;

    let (r, g, b) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Format RGB as hex color string for asusctl
fn rgb_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("{:02x}{:02x}{:02x}", r, g, b)
}

// Simple LCG for random numbers to avoid pulling in rand crate
fn next_random(seed: &mut u64) -> u32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    (*seed >> 32) as u32
}

/// Start the animation loop (runs in tokio task)
pub fn start_animator(state: Arc<AnimatorState>, _ui_weak: Weak<MainWindow>) {
    info!("Starting keyboard animator");

    tokio::spawn(async move {
        // Local state for Color Cycle (RGB)
        let mut current_r: f32 = 255.0;
        let mut current_g: f32 = 0.0;
        let mut current_b: f32 = 0.0;
        let mut target_r: f32 = 0.0;
        let mut target_g: f32 = 255.0;
        let mut target_b: f32 = 0.0;
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345);

        loop {
            // Check for stop signal
            if state.stop.load(Ordering::Relaxed) {
                info!("Animator stopping");
                break;
            }

            let mode = AnimationMode::from(state.mode.load(Ordering::Relaxed) as i32);
            // Cap speed at 150ms for stability
            let raw_speed = state.speed_ms.load(Ordering::Relaxed);
            let effective_speed = raw_speed.max(150) as u64;

            if mode == AnimationMode::None {
                // No animation, sleep longer
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }

            // Calculate next color
            let hex_color = match mode {
                AnimationMode::Rainbow => {
                    // Hue step 1 for smooth, granular transitions
                    let hue = state.hue.fetch_add(1, Ordering::Relaxed) % 360;
                    let (r, g, b) = hsv_to_rgb(hue, 100, 100);
                    rgb_to_hex(r, g, b)
                }
                AnimationMode::ColorCycle => {
                    // RGB Linear Interpolation (Fading) - NOT Rainbow

                    // 1. Check distance to target
                    let dist_sq = (target_r - current_r).powi(2)
                        + (target_g - current_g).powi(2)
                        + (target_b - current_b).powi(2);

                    // If close, pick new random target color
                    if dist_sq < 100.0 {
                        let next_h = next_random(&mut seed) % 360;
                        let (r, g, b) = hsv_to_rgb(next_h, 100, 100);
                        target_r = r as f32;
                        target_g = g as f32;
                        target_b = b as f32;
                    }

                    // 2. Lerp towards target (5% per frame for smooth ease-out)
                    let factor = 0.05;
                    current_r += (target_r - current_r) * factor;
                    current_g += (target_g - current_g) * factor;
                    current_b += (target_b - current_b) * factor;

                    rgb_to_hex(current_r as u8, current_g as u8, current_b as u8)
                }
                AnimationMode::None => continue,
            };

            // Send color update via asusctl command (blocking, AWAITED to prevent races)
            let hex = hex_color.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let result = Command::new("asusctl")
                    .args([
                        "aura", "static", "-c", &hex,
                    ])
                    .output();

                if let Err(e) = result {
                    warn!("Failed to set aura color: {}", e);
                }
            })
            .await;

            // Sleep for the animation speed interval
            tokio::time::sleep(Duration::from_millis(effective_speed)).await;
        }
    });
}

/// Stop the animator
pub fn stop_animator(state: &Arc<AnimatorState>) {
    state.stop.store(true, Ordering::Relaxed);
    state.mode.store(0, Ordering::Relaxed);
}

/// Set animation mode
pub fn set_animation_mode(state: &Arc<AnimatorState>, mode: AnimationMode) {
    state.mode.store(mode as u32, Ordering::Relaxed);
    // Reset stop flag in case we're restarting
    state.stop.store(false, Ordering::Relaxed);
}

/// Set animation speed
pub fn set_animation_speed(state: &Arc<AnimatorState>, speed_ms: u32) {
    let clamped = speed_ms.clamp(50, 2000);
    state.speed_ms.store(clamped, Ordering::Relaxed);
}
