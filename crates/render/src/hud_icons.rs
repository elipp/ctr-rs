//! HUD pictures cut from a racer pack's icons: a strip of equally sized cells, one icon each.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::mesh::Atlas;

/// The icons with these global indices one above the other, each `width` by `height` texels:
/// the one of `ids[n]` is at rows `n * height` on. `icons` is the flat icon array of a racer's
/// pack (`loader::icons::parse_icons`). An icon the pack lacks stays clear.
pub fn strip(icons: &[Icon], vram: &Vram, ids: &[i32], width: usize, height: usize) -> Atlas {
    let mut rgba = vec![0u8; width * ids.len() * height * 4];
    for (n, id) in ids.iter().enumerate() {
        let Some(icon) = icons.iter().find(|i| i.global_index == *id) else {
            continue;
        };
        let layout = &icon.layout;
        let (u0, v0) = layout.uv[0];
        for y in 0..height {
            for x in 0..width {
                let (u, v) = (u0.wrapping_add(x as u8), v0.wrapping_add(y as u8));
                let Ok(Some((c, _))) = vram.texel_stp(layout, u, v) else {
                    continue;
                };
                rgba[((n * height + y) * width + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    Atlas {
        width,
        height: ids.len() * height,
        rgba,
    }
}
