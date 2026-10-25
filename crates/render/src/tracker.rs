//! The lock-on icon of the missile warning (`UI_TrackerBG`), which the HUD draws four times,
//! mirrored, around the kart that a missile chases.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::hud_icons;
use crate::mesh::Atlas;

/// `UI_TRACKER_LOCK_ICON`
pub const LOCK_ICON: i32 = 0x2d;

/// The icon's size in texels, `u1 - u0` by `v2 - v0` (what `UI_TrackerBG` scales), or None if the
/// pack lacks it.
pub fn lock_size(icons: &[Icon]) -> Option<(usize, usize)> {
    let layout = &icons.iter().find(|i| i.global_index == LOCK_ICON)?.layout;
    let [(u0, v0), (u1, _), (_, v2), _] = layout.uv;
    Some((u1.wrapping_sub(u0) as usize, v2.wrapping_sub(v0) as usize))
}

/// The lock-on icon's picture, [`lock_size`] texels (clear, and 1 by 1, if the pack lacks it).
pub fn lock_image(icons: &[Icon], vram: &Vram) -> Atlas {
    let (width, height) = lock_size(icons).unwrap_or((1, 1));
    hud_icons::strip(icons, vram, &[LOCK_ICON], width, height)
}
