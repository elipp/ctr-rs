//! HUD pictures cut from a racer pack's icons: a strip of equally sized cells, one icon each.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::mesh::Atlas;

/// The icons with these global indices one above the other, each `width` by `height` texels:
/// the one of `ids[n]` is at rows `n * height` on. `icons` is the flat icon array of a racer's
/// pack (`loader::icons::parse_icons`). An icon the pack lacks stays clear.
pub fn strip(icons: &[Icon], vram: &Vram, ids: &[i32], width: usize, height: usize) -> Atlas {
    let picked: Vec<_> = ids
        .iter()
        .map(|id| icons.iter().find(|i| i.global_index == *id))
        .collect();
    strip_of(&picked, vram, width, height, false)
}

/// Like [`strip`], for icons the HUD draws half-transparent (`TRANS_50_DECAL`): their texels with
/// the STP bit are half themselves and half what is behind them (see [`half_transparent`]).
pub fn strip_half_transparent(
    icons: &[Icon],
    vram: &Vram,
    ids: &[i32],
    width: usize,
    height: usize,
) -> Atlas {
    let picked: Vec<_> = ids
        .iter()
        .map(|id| icons.iter().find(|i| i.global_index == *id))
        .collect();
    strip_of(&picked, vram, width, height, true)
}

/// The gamma the PlayStation's colours are in, near enough to sRGB's.
const GAMMA: f32 = 2.2;

/// A texel of the PlayStation's 50% mode (`B / 2 + F / 2`) as one that a blend by alpha in linear
/// light draws the same: the PlayStation halves the colours as they are stored, in gamma, so what
/// is left of the background is `0.5^2.2` of its light, not half. The texel's own half is made
/// up in its colour.
pub fn half_transparent([r, g, b, _]: [u8; 4]) -> [u8; 4] {
    let alpha = 1.0 - 0.5f32.powf(GAMMA);
    let channel = |v: u8| {
        let light = (v as f32 / 255.0 / 2.0).powf(GAMMA) / alpha;
        (light.min(1.0).powf(1.0 / GAMMA) * 255.0).round() as u8
    };
    [channel(r), channel(g), channel(b), (alpha * 255.0).round() as u8]
}

/// The same for icons found by their names: the ones with no global index (-1) cannot be found
/// otherwise.
pub fn strip_named(
    icons: &[Icon],
    vram: &Vram,
    names: &[&str],
    width: usize,
    height: usize,
) -> Atlas {
    let picked: Vec<_> = names
        .iter()
        .map(|name| icons.iter().find(|i| i.name == *name))
        .collect();
    strip_of(&picked, vram, width, height, false)
}

/// The icons' texels, those with the STP bit [`half_transparent`] if `half` is set.
fn strip_of(
    icons: &[Option<&Icon>],
    vram: &Vram,
    width: usize,
    height: usize,
    half: bool,
) -> Atlas {
    let mut rgba = vec![0u8; width * icons.len() * height * 4];
    for (n, icon) in icons.iter().enumerate() {
        let Some(icon) = icon else {
            continue;
        };
        let layout = &icon.layout;
        let (u0, v0) = layout.uv[0];
        for y in 0..height {
            for x in 0..width {
                let (u, v) = (u0.wrapping_add(x as u8), v0.wrapping_add(y as u8));
                let Ok(Some((c, stp))) = vram.texel_stp(layout, u, v) else {
                    continue;
                };
                let c = if half && stp { half_transparent(c) } else { c };
                rgba[((n * height + y) * width + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    Atlas {
        width,
        height: icons.len() * height,
        rgba,
    }
}
