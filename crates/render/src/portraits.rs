//! The racers' portraits (`MetaDataCHAR::iconID`): the pictures of the faces that the race
//! results and the character select cut from `gGT->ptrIcons`. Every racer's pack carries all
//! sixteen, in the shared texture pages.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::mesh::Atlas;

/// A portrait is 44x26 texels.
pub const WIDTH: usize = 44;
pub const HEIGHT: usize = 26;

/// `MetaDataCharacters[character].iconID`: the global icon of each character's portrait, in
/// character order (Crash ... Oxide). The order is not quite the character order: the icons were
/// packed as the artists made them (Pinstripe's is 43, after Papu's 41 and Roo's 40).
pub const ICON_IDS: [i32; 16] = [
    32, 33, 34, 35, 36, 37, 38, 39, 43, 41, 40, 42, 44, 53, 54, 55,
];

/// All the portraits one above the other, character `n`'s at rows `n * HEIGHT` on: `icons` is the
/// flat icon array of a racer's pack (`loader::icons::parse_icons`). A portrait the pack lacks
/// stays clear.
pub fn portrait_atlas(icons: &[Icon], vram: &Vram) -> Atlas {
    let height = ICON_IDS.len() * HEIGHT;
    let mut rgba = vec![0u8; WIDTH * height * 4];
    for (n, id) in ICON_IDS.iter().enumerate() {
        let Some(icon) = icons.iter().find(|i| i.global_index == *id) else {
            continue;
        };
        let layout = &icon.layout;
        let (u0, v0) = layout.uv[0];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let (u, v) = (u0.wrapping_add(x as u8), v0.wrapping_add(y as u8));
                let Ok(Some((c, _))) = vram.texel_stp(layout, u, v) else {
                    continue;
                };
                rgba[((n * HEIGHT + y) * WIDTH + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    Atlas {
        width: WIDTH,
        height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::bigindex::BigIndex;
    use loader::disc::open_bigfile;
    use loader::dram::DramFile;
    use loader::icons::parse_icons;
    use loader::vram::parse_vram_file;

    #[test]
    fn portraits_decode_from_the_disc() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
        let Ok((mut disc, big)) = open_bigfile(&path) else {
            return;
        };
        let mut vram = Vram::default();
        let shared = big.read(&mut disc, BigIndex::SHARED_MPK_VRM.get()).unwrap();
        for img in parse_vram_file(&shared).unwrap() {
            vram.upload(&img).unwrap();
        }
        let pack = big.read(&mut disc, BigIndex::ARCADE_1P_PACK.get()).unwrap();
        let icons = parse_icons(&DramFile::parse(&pack).unwrap()).unwrap();
        let atlas = portrait_atlas(&icons, &vram);
        for n in 0..ICON_IDS.len() {
            let picture = &atlas.rgba[n * WIDTH * HEIGHT * 4..][..WIDTH * HEIGHT * 4];
            let visible = picture.chunks(4).filter(|p| p[3] != 0).count();
            assert!(
                visible > WIDTH * HEIGHT / 2,
                "portrait {n}: {visible} texels"
            );
        }
    }
}
