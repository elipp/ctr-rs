//! A kart's shadow (`VehGroundShadow_Main`): four quads on the ground, one per
//! quadrant around the kart, textured with two pictures of one quarter and
//! subtracted from what is on the ground. The pictures are global icons 0 and 1
//! (`shadow1`, `shadow2`) of the racer's pack.
//!
//! The sim decides whether there is a shadow and how it is oriented
//! (`vehicle::shadow`); this makes its corners and texture.

use glam::{Mat3, Vec3};
use loader::icons::Icon;
use loader::vram::Vram;

use crate::mesh::{Atlas, SCALE};

/// Icons are 40x48 texels.
pub const SPRITE_WIDTH: usize = 40;
pub const SPRITE_HEIGHT: usize = 48;

/// Quads around the kart.
pub const QUADS: usize = 4;

/// The shadow is drawn with `0x1f` of `0x80` of each texel (`VEH_GROUND_SHADOW_LARGE_COLOR`):
/// the colour the textured primitive is modulated by, and so how dark the shadow is.
pub const GAIN: f32 = 0x1f as f32 / 128.0;

/// The ground shadow works in a space four times finer than world units.
const FINE: f32 = 4.0;

/// One quadrant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowQuad {
    /// Top left, top right, bottom left, bottom right of the quadrant's icon.
    pub corners: [Vec3; 4],
    /// Which of the two icons.
    pub icon: usize,
}

/// The quadrants of a shadow centred at `center` (world units) on the ground
/// plane `axes` (columns: right, up, forward; 1.0 = unit length), with the
/// `height` the sim gives (0x100 on the ground, smaller as the kart rises).
///
/// The half-sizes are those of `VehGroundShadow_Main`: sideways `40/64 * height`,
/// back `41/64 * height` and forward `52/64 * height`, in quarter units.
pub fn shadow_quads(center: Vec3, axes: Mat3, height: f32) -> [ShadowQuad; QUADS] {
    let scale = height / 64.0 / FINE * SCALE;
    let a0 = axes.x_axis * (40.0 * scale);
    let back = axes.z_axis * (41.0 * scale);
    let front = axes.z_axis * (52.0 * scale);
    let c = center;
    [
        ShadowQuad {
            corners: [c - a0, c, c - a0 - back, c - back],
            icon: 0,
        },
        ShadowQuad {
            corners: [c - a0, c, c - a0 + front, c + front],
            icon: 1,
        },
        ShadowQuad {
            corners: [c + a0, c, c + a0 - back, c - back],
            icon: 0,
        },
        ShadowQuad {
            corners: [c + a0, c, c + a0 + front, c + front],
            icon: 1,
        },
    ]
}

/// The two icons one above the other, each 40x48, every visible texel opaque (the
/// primitive is drawn subtractive; the colour is the texel's, scaled by [`GAIN`]
/// by the vertex colours). Returns the picture and each icon's UV corners (top
/// left, top right, bottom left, bottom right).
pub fn shadow_atlas(icons: [&Icon; 2], vram: &Vram) -> (Atlas, [[[f32; 2]; 4]; 2]) {
    let height = 2 * SPRITE_HEIGHT;
    let mut rgba = vec![0u8; SPRITE_WIDTH * height * 4];
    for (n, icon) in icons.iter().enumerate() {
        let (u0, v0) = icon.layout.uv[0];
        for y in 0..SPRITE_HEIGHT {
            for x in 0..SPRITE_WIDTH {
                let (u, v) = (u0.wrapping_add(x as u8), v0.wrapping_add(y as u8));
                let Ok(Some((c, _))) = vram.texel_stp(&icon.layout, u, v) else {
                    continue;
                };
                rgba[((n * SPRITE_HEIGHT + y) * SPRITE_WIDTH + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    let uv = |n: usize| {
        let (top, bottom) = (
            (n * SPRITE_HEIGHT) as f32 / height as f32,
            ((n + 1) * SPRITE_HEIGHT) as f32 / height as f32,
        );
        [[0.0, top], [1.0, top], [0.0, bottom], [1.0, bottom]]
    };
    (
        Atlas {
            width: SPRITE_WIDTH,
            height,
            rgba,
        },
        [uv(0), uv(1)],
    )
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
    fn quadrants_tile_a_rectangle_around_the_kart() {
        let quads = shadow_quads(Vec3::ZERO, Mat3::IDENTITY, 256.0);
        // Everything meets at the centre, and the quadrants cover the four sign combinations.
        assert!(quads.iter().all(|q| q.corners[1] == Vec3::ZERO));
        let signs = quads.map(|q| (q.corners[0].x.signum(), q.corners[3].z.signum()));
        assert_eq!(signs, [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)]);
        // 40 game units to each side, 41 behind and 52 ahead, on the ground.
        assert!((quads[3].corners[0].x - 40.0 * SCALE).abs() < 1e-6);
        assert!((quads[3].corners[3].z - 52.0 * SCALE).abs() < 1e-6);
        assert!((quads[0].corners[3].z + 41.0 * SCALE).abs() < 1e-6);
        // It shrinks with the kart's height above the ground.
        let small = shadow_quads(Vec3::ZERO, Mat3::IDENTITY, 128.0);
        assert!((small[3].corners[0].x * 2.0 - quads[3].corners[0].x).abs() < 1e-6);
    }

    #[test]
    fn shadow_icons_decode_from_the_disc() {
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
        let find = |index| icons.iter().find(|i| i.global_index == index).unwrap();
        let (atlas, _) = shadow_atlas([find(0), find(1)], &vram);
        for n in 0..2 {
            let icon = &atlas.rgba[n * SPRITE_WIDTH * SPRITE_HEIGHT * 4..]
                [..SPRITE_WIDTH * SPRITE_HEIGHT * 4];
            let visible = icon.chunks(4).filter(|p| p[3] != 0).count();
            println!(
                "shadow icon {n}: {visible} visible of {}",
                SPRITE_WIDTH * SPRITE_HEIGHT
            );
            assert!(visible > 200, "icon {n}: {visible}");
        }
    }
}
