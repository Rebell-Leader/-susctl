//! Software-controlled LED animation modes for the asusd daemon.
//!
//! These modes run as background tasks in asusd and continuously update
//! the LED colors without requiring the GUI to be open.

use serde::{Deserialize, Serialize};

use crate::Colour;

/// Animation modes that can be run by the asusd daemon.
///
/// Note: This type is serialized as JSON for DBus transport since zvariant
/// doesn't support enums with heterogeneous variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnimationMode {
    /// No animation running
    None,
    /// Rainbow effect - cycles through HSV hue (0-360°)
    Rainbow {
        /// Update interval in milliseconds (lower = faster)
        speed_ms: u32,
    },
    /// Color cycle - transitions between a list of colors
    ColorCycle {
        /// Update interval in milliseconds
        speed_ms: u32,
        /// Colors to cycle through
        colors: Vec<Colour>,
    },
    /// Breathe effect - fades between two colors through black
    Breathe {
        /// Update interval in milliseconds
        speed_ms: u32,
        /// Primary color
        color1: Colour,
        /// Secondary color (fades to this, then back)
        color2: Colour,
    },
    /// Pulse effect - brightness animation (dims and brightens)
    Pulse {
        /// Update interval in milliseconds
        speed_ms: u32,
        /// Base color to pulse
        color: Colour,
        /// Minimum brightness factor (0.0 - 1.0)
        min_brightness: f32,
        /// Maximum brightness factor (0.0 - 1.0)
        max_brightness: f32,
    },
}

impl Default for AnimationMode {
    fn default() -> Self {
        Self::None
    }
}

impl AnimationMode {
    /// Returns true if this is an active animation mode
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Get the speed/interval in milliseconds
    pub fn speed_ms(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Rainbow { speed_ms } => *speed_ms,
            Self::ColorCycle { speed_ms, .. } => *speed_ms,
            Self::Breathe { speed_ms, .. } => *speed_ms,
            Self::Pulse { speed_ms, .. } => *speed_ms,
        }
    }
}

/// Apply a brightness factor to a color by scaling RGB values.
/// This allows simulating granular brightness on hardware with limited levels.
pub fn apply_brightness(color: Colour, factor: f32) -> Colour {
    let factor = factor.clamp(0.0, 1.0);
    Colour {
        r: (color.r as f32 * factor) as u8,
        g: (color.g as f32 * factor) as u8,
        b: (color.b as f32 * factor) as u8,
    }
}

/// Convert HSV to RGB color.
/// Hue: 0-360, Saturation: 0.0-1.0, Value: 0.0-1.0
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Colour {
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    Colour {
        r: ((r1 + m) * 255.0) as u8,
        g: ((g1 + m) * 255.0) as u8,
        b: ((b1 + m) * 255.0) as u8,
    }
}

/// Linear interpolation between two colors
pub fn lerp_colour(c1: &Colour, c2: &Colour, t: f32) -> Colour {
    let t = t.clamp(0.0, 1.0);
    Colour {
        r: (c1.r as f32 + (c2.r as f32 - c1.r as f32) * t) as u8,
        g: (c1.g as f32 + (c2.g as f32 - c1.g as f32) * t) as u8,
        b: (c1.b as f32 + (c2.b as f32 - c1.b as f32) * t) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hsv_to_rgb_red() {
        let c = hsv_to_rgb(0.0, 1.0, 1.0);
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn test_brightness() {
        let c = Colour {
            r: 100,
            g: 200,
            b: 50,
        };
        let dim = apply_brightness(c, 0.5);
        assert_eq!(dim.r, 50);
        assert_eq!(dim.g, 100);
        assert_eq!(dim.b, 25);
    }
}
