//! Where the race's 2D art goes. Retail draws it on a 512 by 240 screen, and the HUD here places
//! it the same way, a pixel of that screen being [`PIXEL`] logical pixels. Retail's pixels are
//! taller than wide (`weapon::HUD_PIXEL_ASPECT`), and so are these: at 2 by 3, the 240 lines fill a
//! window 720 pixels tall, and every texel is a whole number of pixels. Like the rest of the HUD,
//! it keeps this size whatever the window's.

use bevy::prelude::*;

/// A pixel of retail's screen, in logical pixels.
pub const PIXEL: Vec2 = Vec2::new(2.0, 3.0);

/// `n` pixels of retail's screen across.
pub fn x(n: f32) -> Val {
    Val::Px(n * PIXEL.x)
}

/// `n` lines of retail's screen down.
pub fn y(n: f32) -> Val {
    Val::Px(n * PIXEL.y)
}
