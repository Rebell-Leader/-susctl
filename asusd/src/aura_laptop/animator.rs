//! Animation task runner for asusd daemon.
//!
//! This module provides the background thread that runs LED animations.
//! Animations persist even when the GUI is closed.
//!
//! Note: Uses std::thread and blocking for stability across contexts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use log::{debug, info, warn};
use rog_aura::animation::{apply_brightness, hsv_to_rgb, lerp_colour, AnimationMode};
use rog_aura::{AuraEffect, AuraModeNum, Colour};

use super::Aura;

/// State for the animation task
#[derive(Debug)]
pub struct AnimatorState {
    /// Current animation mode
    pub mode: Arc<Mutex<AnimationMode>>,
    /// Flag to stop the animation
    pub stop: Arc<AtomicBool>,
    /// Flag indicating if the thread is currently running
    pub running: Arc<AtomicBool>,
}

impl Default for AnimatorState {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimatorState {
    pub fn new() -> Self {
        Self {
            mode: Arc::new(Mutex::new(AnimationMode::None)),
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal the animator to stop
    pub fn signal_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Check if stop signal is set
    pub fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Clear the stop flag
    pub fn clear_stop(&self) {
        self.stop.store(false, Ordering::Relaxed);
    }

    /// Check if animator thread is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

pub fn spawn_animator(aura: Aura, state: Arc<AnimatorState>) {
    // Mark as running
    state.running.store(true, Ordering::Relaxed);

    thread::spawn(move || {
        info!("Aura animator thread started");

        // Animation state variables
        let mut hue: f32 = 0.0; // For Rainbow
        let mut color_index: usize = 0; // For ColorCycle
        let mut hold_counter: u32 = 0; // For ColorCycle hold
        let mut lerp_t: f32 = 0.0; // Interpolation factor
        let mut breathe_phase: f32 = 0.0; // For Breathe (0-2π)
        let mut pulse_phase: f32 = 0.0; // For Pulse (0-2π)

        loop {
            // Check stop flag
            if state.should_stop() {
                debug!("Animator received stop signal");
                break;
            }

            // Get current mode (with timeout to verify loop health)
            let mode_opt = if let Ok(guard) = state.mode.lock() {
                Some(guard.clone())
            } else {
                None
            };

            let mode = match mode_opt {
                Some(m) => m,
                None => {
                    warn!("Failed to lock mode mutex");
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };

            if !mode.is_active() {
                // No animation, sleep briefly and check again
                thread::sleep(Duration::from_millis(100));
                continue;
            }

            let speed_ms = mode.speed_ms().max(50); // Minimum 50ms interval

            // Generate the color for this frame
            let color = match &mode {
                AnimationMode::None => continue,

                AnimationMode::Rainbow { .. } => {
                    hue = (hue + 5.0) % 360.0;
                    hsv_to_rgb(hue, 1.0, 1.0)
                }

                AnimationMode::ColorCycle { colors, .. } => {
                    if colors.is_empty() {
                        Colour { r: 255, g: 0, b: 0 }
                    } else if hold_counter > 0 {
                        hold_counter -= 1;
                        colors[color_index]
                    } else {
                        let next_index = (color_index + 1) % colors.len();
                        lerp_t += 0.05; // Fade speed

                        if lerp_t >= 1.0 {
                            lerp_t = 0.0;
                            color_index = next_index;
                            hold_counter = 20; // Hold for ~1-2 seconds (20 * speed_ms)
                            colors[color_index] // Ensure we land exactly on target
                        } else {
                            lerp_colour(&colors[color_index], &colors[next_index], lerp_t)
                        }
                    }
                }

                AnimationMode::Breathe { color1, color2, .. } => {
                    breathe_phase += 0.05; // Slow smooth breathe
                    if breathe_phase > std::f32::consts::TAU {
                        breathe_phase = 0.0;
                    }

                    // Smooth sine wave breathe: C1 -> Black -> C2 -> Black -> C1
                    // 0..PI: Pulse C1
                    // PI..2PI: Pulse C2

                    if breathe_phase < std::f32::consts::PI {
                        let brightness = (breathe_phase.sin()).abs();
                        apply_brightness(*color1, brightness)
                    } else {
                        let brightness = ((breathe_phase - std::f32::consts::PI).sin()).abs();
                        apply_brightness(*color2, brightness)
                    }
                }

                AnimationMode::Pulse {
                    color,
                    min_brightness,
                    max_brightness,
                    ..
                } => {
                    pulse_phase += 0.1;
                    if pulse_phase > std::f32::consts::TAU {
                        pulse_phase = 0.0;
                    }

                    // Sine wave between min and max brightness
                    let t = (pulse_phase.sin() + 1.0) / 2.0; // 0-1
                    let brightness = min_brightness + (max_brightness - min_brightness) * t;
                    apply_brightness(*color, brightness)
                }
            };

            // Apply the color to the LED using async call directly
            let effect = AuraEffect {
                mode: AuraModeNum::Static,
                colour1: color,
                ..Default::default()
            };

            // Execute async code block synchronously using futures_lite
            let res = futures_lite::future::block_on(async {
                let config = aura.config.lock().await;
                let dev_type = config.led_type;
                drop(config);

                aura.write_effect_and_apply(dev_type, &effect).await
            });

            if let Err(e) = res {
                warn!("Animation frame failed: {:?}", e);
            }

            thread::sleep(Duration::from_millis(speed_ms as u64));
        }

        state.running.store(false, Ordering::Relaxed);
        info!("Aura animator thread stopped");
    });
}
