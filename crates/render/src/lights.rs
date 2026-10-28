//! The traffic lights of a race's start: three red lights and a green one, each a picture of a
//! racer's pack when off and when on (`lightredoff`, `lightredon`, `lightgreenoff`,
//! `lightgreenon`; icons with no global index).

use loader::icons::Icon;
use loader::vram::Vram;

use crate::hud_icons;
use crate::mesh::Atlas;

/// A light is 48 by 32 texels.
pub const WIDTH: usize = 48;
pub const HEIGHT: usize = 32;

/// How many lights there are: the reds, then the green.
pub const LIGHTS: usize = 4;

const NAMES: [&str; 4] = ["lightredoff", "lightredon", "lightgreenoff", "lightgreenon"];

/// The lights' pictures one above the other: red off, red on, green off, green on.
pub fn atlas(icons: &[Icon], vram: &Vram) -> Atlas {
    hud_icons::strip_named(icons, vram, &NAMES, WIDTH, HEIGHT)
}

/// The picture (its place in [`atlas`]) of light `n`, 0 to 2 the reds and 3 the green, when `on`
/// of the lights are on.
pub fn cell(n: usize, on: usize) -> usize {
    let green = n == LIGHTS - 1;
    green as usize * 2 + (n < on) as usize
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
    fn the_lights_turn_on_in_order() {
        assert_eq!([0, 1, 2, 3].map(|n| cell(n, 0)), [0, 0, 0, 2]);
        assert_eq!([0, 1, 2, 3].map(|n| cell(n, 2)), [1, 1, 0, 2]);
        assert_eq!([0, 1, 2, 3].map(|n| cell(n, 4)), [1, 1, 1, 3]);
    }

    #[test]
    fn lights_decode_from_the_disc() {
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
        let atlas = atlas(&icons, &vram);
        // Each light is there (the corners of its cell are clear), and an "on" light is brighter
        // than its "off" one.
        let picture = |n: usize| &atlas.rgba[n * WIDTH * HEIGHT * 4..][..WIDTH * HEIGHT * 4];
        let brightness = |n: usize| -> u32 {
            picture(n)
                .chunks(4)
                .map(|p| p[0] as u32 + p[1] as u32 + p[2] as u32)
                .sum()
        };
        for n in 0..4 {
            let visible = picture(n).chunks(4).filter(|p| p[3] != 0).count();
            assert!(visible > WIDTH * HEIGHT / 3, "light {n}: {visible} texels");
        }
        assert!(brightness(1) > brightness(0));
        assert!(brightness(3) > brightness(2));
    }
}
