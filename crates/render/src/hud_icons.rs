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
    strip_of(&picked, vram, width, height)
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
    strip_of(&picked, vram, width, height)
}

fn strip_of(icons: &[Option<&Icon>], vram: &Vram, width: usize, height: usize) -> Atlas {
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
                let Ok(Some((c, _))) = vram.texel_stp(layout, u, v) else {
                    continue;
                };
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
