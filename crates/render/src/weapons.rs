//! The weapon icons of the HUD (`UI_Weapon_DrawSelf`): what the driver holds, and what the
//! roulette shuffles through. Every racer's pack carries them.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::hud_icons;
use crate::mesh::Atlas;

/// A weapon icon on screen is 48x32 texels. It is stored 32x48, a quarter turn clockwise from
/// how it is shown (the HUD draws it turned counter-clockwise), and [`weapon_atlas`] turns it.
pub const WIDTH: usize = 48;
pub const HEIGHT: usize = 32;

/// `UI_WEAPON_ICON_BASE`: the global icon of `heldItemID` 0 (turbo); the others follow in the
/// order of the items.
const ICON_BASE: i32 = 5;

/// `UI_WEAPON_JUICED_ICON_BASE`: the icons of the items that are better with 10 wumpa fruit
/// (TNT, potion and shield), by item as well.
const JUICED_ICON_BASE: i32 = 0x11;

/// `UI_WEAPON_MASK_UKA_ICON`: the mask of a bad guy.
const UKA_ICON: i32 = 0x32;

/// `UI_WEAPON_MASK_GOOD_CHARACTER_BITS`: Crash, Coco, Polar and Pura (bit n for character n)
/// have Aku Aku; the others get Uka Uka.
const GOOD_MASK_CHARACTERS: u32 = 0xc9;

/// `HELD_ITEM_*`
pub mod item {
    pub const TNT: u8 = 3;
    pub const POTION: u8 = 4;
    pub const SHIELD: u8 = 6;
    pub const MASK: u8 = 7;
}

/// The icons in the order of [`weapon_atlas`]'s cells: the items 0 to 11 (all the roulette
/// shows; the spring, which it never lands on, has none), the juiced TNT, potion and shield, and
/// the Uka mask.
pub const ICON_IDS: [i32; 16] = [
    5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 0x14, 0x15, 0x17, UKA_ICON,
];

/// The global icon for a driver holding `item` (a `HELD_ITEM_*` below the roulette), as
/// `UI_Weapon_DrawSelf` picks it: `juiced` is whether the driver has 10 wumpa fruit, `character`
/// whose mask it is.
pub fn icon_id(item: u8, juiced: bool, character: u8) -> i32 {
    if item == item::MASK && (GOOD_MASK_CHARACTERS >> character) & 1 == 0 {
        return UKA_ICON;
    }
    let base = if juiced && matches!(item, item::TNT | item::POTION | item::SHIELD) {
        JUICED_ICON_BASE
    } else {
        ICON_BASE
    };
    base + item as i32
}

/// The cell of an icon in [`weapon_atlas`], if it is one of [`ICON_IDS`].
pub fn cell(icon_id: i32) -> Option<usize> {
    ICON_IDS.iter().position(|&id| id == icon_id)
}

/// All the weapon icons one above the other (see [`hud_icons::strip`]), in the order of
/// [`ICON_IDS`].
pub fn weapon_atlas(icons: &[Icon], vram: &Vram) -> Atlas {
    // As stored: HEIGHT wide and WIDTH tall.
    let stored = hud_icons::strip(icons, vram, &ICON_IDS, HEIGHT, WIDTH);
    let mut rgba = vec![0u8; WIDTH * ICON_IDS.len() * HEIGHT * 4];
    for n in 0..ICON_IDS.len() {
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                // A quarter turn counter-clockwise: the stored top right corner is the top left.
                let (from_x, from_y) = (HEIGHT - 1 - y, n * WIDTH + x);
                let from = (from_y * HEIGHT + from_x) * 4;
                let to = ((n * HEIGHT + y) * WIDTH + x) * 4;
                rgba[to..to + 4].copy_from_slice(&stored.rgba[from..from + 4]);
            }
        }
    }
    Atlas {
        width: WIDTH,
        height: ICON_IDS.len() * HEIGHT,
        rgba,
    }
}

/// `UI_WEAPON_BG_SHINE_ICON_INDEX`: the glow behind the weapon and the wumpa fruit of a driver
/// that is juiced up, 48x32 texels.
pub const SHINE_ICON: i32 = 0x31;
pub const SHINE_WIDTH: usize = 48;
pub const SHINE_HEIGHT: usize = 32;

/// A quadrant of the glow is one texel less each way than the icon: the game stretches `u1 - u0`
/// texels over it (and `v2 - v0`), so the last column and row are never drawn.
const QUADRANT_WIDTH: usize = SHINE_WIDTH - 1;
const QUADRANT_HEIGHT: usize = SHINE_HEIGHT - 1;

/// The size of the glow's picture: the four quadrants.
pub const SHINE_IMAGE_WIDTH: usize = QUADRANT_WIDTH * 2;
pub const SHINE_IMAGE_HEIGHT: usize = QUADRANT_HEIGHT * 2;

/// How big the glow is drawn next to the texture: `UI_Weapon_DrawBG`'s
/// `scale * UI_WEAPON_BG_SHINE_SCALE_MUL >> 16` at the HUD's scale of 1.
pub const SHINE_SCALE: f32 = 0xd000 as f32 / 0x10000 as f32;

/// `UI_WeaponBG_AnimateShine`'s colours at their dimmest (`sine` 0), as `(r, g, b)` with 0x80 being
/// full brightness: the corner of the picture the quads share is the bright one, the outer corners
/// dark, and the edges between orange. At the other end of the pulse every one of them is doubled.
const BRIGHT: [f32; 3] = [0x7f as f32, 0x7f as f32, 0.0];
const MID: [f32; 3] = [0x7f as f32, 0x32 as f32, 0.0];
const DARK: [f32; 3] = [0x21 as f32, 0x10 as f32, 0.0];

/// The glow of a juiced-up driver, as `UI_WeaponBG_DrawShine` draws it: the icon's four mirrored
/// copies, [`SHINE_IMAGE_WIDTH`] by [`SHINE_IMAGE_HEIGHT`] texels, shaded from dark at the outer corners to bright in the middle (the
/// picture is Gouraud shaded by the quad's four colours).
///
/// Retail adds this and then subtracts a grey copy of it on the PlayStation's GPU; a HUD node
/// only blends by alpha. So the picture is the added light: its colour with its own brightness as
/// the alpha, at the dimmest of the pulse. The pulse is the node's tint.
pub fn shine_image(icons: &[Icon], vram: &Vram) -> Atlas {
    let (width, height) = (SHINE_IMAGE_WIDTH, SHINE_IMAGE_HEIGHT);
    let mut rgba = vec![0u8; width * height * 4];
    let Some(icon) = icons.iter().find(|i| i.global_index == SHINE_ICON) else {
        return Atlas {
            width,
            height,
            rgba,
        };
    };
    let layout = &icon.layout;
    let (u0, v0) = layout.uv[0];
    for y in 0..height {
        for x in 0..width {
            // The four quads are mirrors of each other, their dark corners outside.
            let (qx, qy) = (x.min(width - 1 - x), y.min(height - 1 - y));
            let Ok(Some((texel, _))) =
                vram.texel_stp(layout, u0.wrapping_add(qx as u8), v0.wrapping_add(qy as u8))
            else {
                continue;
            };
            let (fu, fv) = (
                qx as f32 / QUADRANT_WIDTH as f32,
                qy as f32 / QUADRANT_HEIGHT as f32,
            );
            // Vertex 0 is dark, 1 and 2 orange, 3 bright.
            let light: [f32; 3] = std::array::from_fn(|c| {
                let shade = DARK[c] * (1.0 - fu) * (1.0 - fv)
                    + MID[c] * (fu * (1.0 - fv) + (1.0 - fu) * fv)
                    + BRIGHT[c] * fu * fv;
                (texel[c] as f32 / 255.0 * shade / 128.0).min(1.0)
            });
            let strength = light.iter().cloned().fold(0.0, f32::max);
            if strength <= 0.0 {
                continue;
            }
            let out = &mut rgba[(y * width + x) * 4..][..4];
            for c in 0..3 {
                out[c] = (light[c] / strength * 255.0).round() as u8;
            }
            out[3] = (strength * 255.0).round() as u8;
        }
    }
    Atlas {
        width,
        height,
        rgba,
    }
}

/// `UI_WeaponBG_AnimateShine`'s pulse for the theta the HUD has reached (4096 to the turn): the
/// brightness of the colours over their dimmest, 1 to 2.
pub fn shine_pulse(theta: u32) -> f32 {
    let turn = (theta % 4096) as f32 / 4096.0;
    1.0 + (turn * std::f32::consts::TAU).sin().abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pulse_goes_from_dim_to_double() {
        assert_eq!(shine_pulse(0), 1.0);
        assert!((shine_pulse(1024) - 2.0).abs() < 1e-5);
        assert!((shine_pulse(2048) - 1.0).abs() < 1e-5);
        assert!((shine_pulse(3072) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn the_game_picks_the_icons_this_way() {
        assert_eq!(icon_id(0, false, 0), 5);
        assert_eq!(icon_id(item::TNT, false, 0), 8);
        assert_eq!(icon_id(item::TNT, true, 0), 0x14);
        assert_eq!(icon_id(item::SHIELD, true, 0), 0x17);
        // Only the three are juiced.
        assert_eq!(icon_id(1, true, 0), 6);
        // Crash has Aku Aku, Cortex has Uka Uka.
        assert_eq!(icon_id(item::MASK, false, 0), 12);
        assert_eq!(icon_id(item::MASK, false, 1), UKA_ICON);
        // Every item the roulette lands on has a cell, in every state.
        for item in (0..12).filter(|&i| i != 5) {
            for (juiced, character) in [(false, 0), (true, 0), (false, 1)] {
                assert!(cell(icon_id(item, juiced, character)).is_some(), "{item}");
            }
        }
    }
}
