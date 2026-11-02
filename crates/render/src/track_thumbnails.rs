//! The pictures of the racing tracks that the track select shows before its preview video
//! (`MainMenu_LevelRow::videoThumbnail`), which are icons of the main menu's level.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::hud_icons;
use crate::mesh::Atlas;

/// The size of the icons, from their corners.
pub const WIDTH: usize = 169;
pub const HEIGHT: usize = 71;

/// The global icon of each racing track's picture, by `LevelId` (`arcadeTracks` in `D230.c`).
pub const ICON_IDS: [i32; 18] = [
    0x53, 0x54, 0x4f, 0x51, 0x59, 0x5a, 0x5b, 0x4c, 0x57, 0x4e, 0x4d, 0x52, 0x50, 0x58, 0x43, 0x44,
    0x55, 0x56,
];

/// All the pictures one above the other, level `n`'s at rows `n * HEIGHT` on.
pub fn atlas(icons: &[Icon], vram: &Vram) -> Atlas {
    hud_icons::strip(icons, vram, &ICON_IDS, WIDTH, HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::bigindex::{BigIndex, LevFile, Lod};
    use loader::disc::open_bigfile;
    use loader::dram::DramFile;
    use loader::icons::parse_level_icons;
    use loader::level::LevelId;
    use loader::vram::parse_vram_file;

    #[test]
    fn every_track_has_a_picture_on_the_disc() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
        let Ok((mut disc, big)) = open_bigfile(&path) else {
            return;
        };
        let file = |f| BigIndex::level(LevelId::MAIN_MENU_LEVEL, Lod::OnePlayer, f).get();
        let mut vram = Vram::default();
        for index in [file(LevFile::Vram), BigIndex::SHARED_MPK_VRM.get()] {
            for img in parse_vram_file(&big.read(&mut disc, index).unwrap()).unwrap() {
                vram.upload(&img).unwrap();
            }
        }
        let lev = DramFile::parse(&big.read(&mut disc, file(LevFile::Lev)).unwrap()).unwrap();
        let atlas = atlas(&parse_level_icons(&lev).unwrap(), &vram);
        for n in 0..ICON_IDS.len() {
            let picture = &atlas.rgba[n * WIDTH * HEIGHT * 4..][..WIDTH * HEIGHT * 4];
            let lit = picture.chunks(4).filter(|p| p[..3] != [0; 3]).count();
            assert!(lit > WIDTH * HEIGHT / 2, "track {n}: {lit} texels");
        }
    }
}
