//! The minimap of a race (`UI_Map_DrawMap`, `UI_Map_DrawDrivers`): the level's picture of the
//! track at the bottom right, and a dot on it for each kart.
//!
//! The picture is two icons of the level's own, a top and a bottom half, drawn semi-transparent
//! in the PS1's additive mode: its texels with the STP bit add to the picture behind, the others
//! cover it. [`map_image`] keeps that in one premultiplied picture, alpha 0 for the texels that
//! add and 255 for the ones that cover, which a premultiplied blend (`src + dst * (1 - alpha)`)
//! draws as the PS1 does.

use loader::icons::{Icon, IconGroup};
use loader::lev::Minimap;
use loader::vram::Vram;

use crate::mesh::Atlas;

/// `gGT->ptrIcons` of the picture's top and bottom halves.
pub const TOP_ICON: i32 = 3;
pub const BOTTOM_ICON: i32 = 4;

/// Where the picture's bottom right corner is, on retail's 512 by 240 screen.
pub const RIGHT: i32 = 500;
pub const BOTTOM: i32 = 195;

/// `gGT->iconGroup[UI_MAP_ICON_GROUP]`, and the dots in it of a kart a bot drives
/// (`UI_MAP_PLAYER_ICON_AI`) and of one a player does (`UI_MAP_PLAYER_ICON_HUMAN`).
pub const ICON_GROUP: i16 = 5;
pub const BOT_DOT: usize = 0x31;
pub const PLAYER_DOT: usize = 0x32;

/// `UI_MAP_ICON_Y_OFFSET`
const ICON_Y_OFFSET: i32 = 0x10;

/// The colours of the characters' dots (`data.colors` from `CRASH_BLUE` on), `0xBBGGRR`, by
/// character.
pub const CHARACTER_COLORS: [u32; 16] = [
    0xF56600, 0xF6, 0xDC5B, 0xF500F5, 0xFF00A9, 0xCCAA, 0xE6E600, 0xFF0055, 0x806040, 0xD3E6,
    0x7FFF, 0x4797CC, 0xFFBF40, 0xFFFFFF, 0x808080, 0x50E680,
];

/// `WHITE`, which a player's dot flashes to.
pub const WHITE: u32 = 0xFFFFFF;

/// Where on retail's screen the dot of a kart at world `x`, `z` has its top left corner
/// (`UI_Map_GetIconPos`, one player). Like retail, the position is not taken from the map's world
/// start before it is scaled.
pub fn icon_pos(map: &Minimap, x: i32, z: i32) -> (i32, i32) {
    let range_x = map.world_end.0 as i32 - map.world_start.0 as i32;
    let range_y = map.world_end.1 as i32 - map.world_start.1 as i32;
    let (size_x, size_y) = (map.icon_size.0 as i32, map.icon_size.1 as i32);
    let (start_x, origin_y) = (
        map.icon_start.0 as i32,
        map.icon_start.1 as i32 - ICON_Y_OFFSET,
    );
    // `(s32)((u32)pos * size) / range`: the product wraps, and the quotient goes towards 0.
    let across = |pos: i32, range: i32| pos.wrapping_mul(size_x).checked_div(range).unwrap_or(0);
    let down = |pos: i32, range: i32| {
        pos.wrapping_mul(size_y)
            .wrapping_mul(2)
            .checked_div(range)
            .unwrap_or(0)
    };
    match map.mode {
        0 => (start_x + across(x, range_x), origin_y + down(z, range_y)),
        1 => (start_x - across(z, range_y), origin_y + down(x, range_x)),
        2 => (start_x - across(x, range_x), origin_y - down(z, range_y)),
        _ => (start_x + across(z, range_y), origin_y - down(x, range_x)),
    }
}

/// An icon's size in texels, `u1 - u0` by `v2 - v0`.
fn size(icon: &Icon) -> (usize, usize) {
    let [(u0, v0), (u1, _), (_, v2), _] = icon.layout.uv;
    (u1.wrapping_sub(u0) as usize, v2.wrapping_sub(v0) as usize)
}

/// Each texel of `icon`, from its top left, with its STP bit; `None` for the clear ones.
fn texels(
    icon: &Icon,
    vram: &Vram,
) -> impl Iterator<Item = (usize, usize, Option<([u8; 4], bool)>)> {
    let (width, height) = size(icon);
    let (u0, v0) = icon.layout.uv[0];
    (0..height).flat_map(move |y| {
        (0..width).map(move |x| {
            let (u, v) = (u0.wrapping_add(x as u8), v0.wrapping_add(y as u8));
            (x, y, vram.texel_stp(&icon.layout, u, v).ok().flatten())
        })
    })
}

/// The map's picture, premultiplied (see the module): the bottom half, and above it the top half
/// if `top` is given, both against the right edge as retail draws them. `None` if the level has
/// no bottom half.
pub fn map_image(icons: &[Icon], vram: &Vram, top: bool) -> Option<Atlas> {
    let find = |id| icons.iter().find(|i| i.global_index == id);
    let bottom = find(BOTTOM_ICON)?;
    let top = top.then(|| find(TOP_ICON)).flatten();
    let halves: Vec<&Icon> = top.into_iter().chain([bottom]).collect();
    let width = halves.iter().map(|i| size(i).0).max()?;
    let height = halves.iter().map(|i| size(i).1).sum();
    let mut rgba = vec![0u8; width * height * 4];
    let mut row = 0;
    for icon in halves {
        let (w, h) = size(icon);
        for (x, y, texel) in texels(icon, vram) {
            let Some(([r, g, b, _], stp)) = texel else {
                continue;
            };
            let at = ((row + y) * width + width - w + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[r, g, b, if stp { 0 } else { 255 }]);
        }
        row += h;
    }
    Some(Atlas {
        width,
        height,
        rgba,
    })
}

/// Dot `id` of the racer pack's `groups` tinted `color` (`0xBBGGRR`) as the PS1 tints a texture:
/// each channel times the colour's over 128. Opaque, as retail draws it. `None` if the pack lacks
/// it.
pub fn dot_image(groups: &[IconGroup], vram: &Vram, id: usize, color: u32) -> Option<Atlas> {
    let icon = groups
        .iter()
        .find(|g| g.group_id == ICON_GROUP)?
        .icons
        .get(id)?;
    let (width, height) = size(icon);
    let tint = [color & 0xff, color >> 8 & 0xff, color >> 16 & 0xff];
    let mut rgba = vec![0u8; width * height * 4];
    for (x, y, texel) in texels(icon, vram) {
        let Some((c, _)) = texel else {
            continue;
        };
        let at = (y * width + x) * 4;
        for k in 0..3 {
            rgba[at + k] = (c[k] as u32 * tint[k] / 128).min(255) as u8;
        }
        rgba[at + 3] = 255;
    }
    Some(Atlas {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::bigindex::{BigIndex, LevFile, Lod};
    use loader::disc::open_bigfile;
    use loader::dram::DramFile;
    use loader::icons::{parse_groups, parse_level_icons};
    use loader::level::LevelId;
    use loader::vram::parse_vram_file;

    fn map(mode: i16) -> Minimap {
        Minimap {
            world_end: (1000, 2000),
            world_start: (-1000, -2000),
            icon_size: (100, 50),
            icon_start: (400, 150),
            mode,
            top_half_mode: 0,
        }
    }

    #[test]
    fn dots_turn_with_the_map() {
        // Across: 300 * 100 / 2000 = 15; down: 800 * 50 * 2 / 4000 = 20, from 150 - 16.
        assert_eq!(icon_pos(&map(0), 300, 800), (415, 154));
        // A quarter turn: z across (800 * 100 / 4000), x down (300 * 100 / 2000).
        assert_eq!(icon_pos(&map(1), 300, 800), (380, 149));
        assert_eq!(icon_pos(&map(2), 300, 800), (385, 114));
        assert_eq!(icon_pos(&map(3), 300, 800), (420, 119));
        // The quotient goes towards 0, as in C.
        assert_eq!(icon_pos(&map(0), -301, 0), (385, 134));
    }

    #[test]
    fn every_track_has_a_map() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
        let Ok((mut disc, big)) = open_bigfile(&path) else {
            return;
        };
        let shared =
            parse_vram_file(&big.read(&mut disc, BigIndex::SHARED_MPK_VRM.get()).unwrap()).unwrap();
        let mut vram = Vram::default();
        for img in &shared {
            vram.upload(img).unwrap();
        }
        let pack = big.read(&mut disc, BigIndex::ARCADE_1P_PACK.get()).unwrap();
        let groups = parse_groups(&DramFile::parse(&pack).unwrap()).unwrap();
        for id in [BOT_DOT, PLAYER_DOT] {
            let dot = dot_image(&groups, &vram, id, WHITE).expect("the pack has the dots");
            assert!(
                dot.rgba.chunks(4).any(|p| p[3] != 0),
                "dot {id:#x} is clear"
            );
        }
        for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
            let mut read = |f| {
                big.read(
                    &mut disc,
                    BigIndex::level(LevelId(id), Lod::OnePlayer, f).get(),
                )
                .unwrap()
            };
            let lev = DramFile::parse(&read(LevFile::Lev)).unwrap();
            let mut vram = Vram::default();
            for img in parse_vram_file(&read(LevFile::Vram))
                .unwrap()
                .iter()
                .chain(&shared)
            {
                vram.upload(img).unwrap();
            }
            let layout = Minimap::parse(&lev).unwrap().expect("a track has a map");
            let icons = parse_level_icons(&lev).unwrap();
            let image = map_image(&icons, &vram, layout.top_half_mode == 0).unwrap();
            assert!(
                image.rgba.chunks(4).any(|p| p[3] == 255),
                "level {id}'s map is clear"
            );
            // The track's edge adds to the scene behind.
            assert!(image.rgba.chunks(4).any(|p| p[3] == 0 && p[..3] != [0; 3]));
        }
    }
}
