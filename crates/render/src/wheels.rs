//! The wheels of a kart. Retail draws them as sprites, not as part of the model
//! (`DrawTires_Solid`): each wheel is a camera-facing square of one of 17
//! pre-rendered tyre pictures (`tireAnim`, icon group 0 of the racer's pack),
//! picked by the angle between the axle and the line of sight, with the picture
//! mirrored when the axle points away from the camera.
//!
//! Everything here is plain `f32` vectors in world units (the kart's
//! translation and rotation come from the caller): the sprites are only
//! drawn, so unlike the sim they need not be bit-exact.

use glam::{Mat3, Vec3};
use loader::icons::Icon;
use loader::texture::Blend;
use loader::vram::Vram;

use crate::mesh::{Atlas, SCALE};

/// Icons are 32x32 texels.
pub const SPRITE_SIZE: usize = 32;

/// `sDrawTiresSpriteIndexTable`: `|axle . view| / 32` (0..=128) to icon number.
const SPRITE_INDEX: [u8; 0x81] = [
    0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x02, 0x02, 0x02,
    0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
    0x04, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x06, 0x06, 0x06, 0x06,
    0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09,
    0x09, 0x09, 0x09, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b,
    0x0b, 0x0c, 0x0c, 0x0c, 0x0c, 0x0c, 0x0d, 0x0d, 0x0d, 0x0d, 0x0e, 0x0e, 0x0e, 0x0f, 0x0f, 0x10,
    0x10,
];

/// `VEH_BIRTH_WHEEL_SIZE`: `Driver::wheelSize` (the picture's size, 0 hides the wheels).
pub const WHEEL_SIZE: i32 = 0xccc;

/// Front left, front right, rear left, rear right (`wheelLocal[0..4]`); "left"
/// is +X in the kart's own frame, as in retail.
pub const WHEELS: usize = 4;

/// `DrawTires` works in a space four times finer than the world the model
/// is drawn in (it shifts the camera offset left by two).
const FINE: f32 = 4.0;

/// The instance scale `VehBirth` gives drivers (`VEH_BIRTH_DRIVER_INSTANCE_SCALE`).
pub const DRIVER_INSTANCE_SCALE: i32 = 0xccc;

/// Half the picture's size in world units (`wheelSize >> 18` of a 4096 vector, in 1/4 units).
pub fn radius(wheel_size: i32) -> f32 {
    wheel_size as f32 / 64.0 / FINE * SCALE
}

/// A wheel's square in world space and how the sprite goes onto it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelQuad {
    /// Corners in the order of the sprite's UVs: top left, top right, bottom
    /// left, bottom right.
    pub corners: [Vec3; 4],
    /// Which of the 17 icons.
    pub sprite: usize,
    /// The wheel's centre, and the part of the kart it hangs on (for the caller's depth sorting).
    pub center: Vec3,
    /// Whether the wheel is on the side of the kart that faces the camera.
    pub near_side: bool,
}

/// `DrawTiresSolid_BuildWheelLocalPairs`: for each wheel its centre (in
/// 1/4 game units, `scale` = the instance scale, 0x1000 = 1.0), and the "rim"
/// vector whose direction is the axle (4096 = unit length).
fn local_pairs(scale: [i32; 3], wheel_rotation: i16) -> [(Vec3, Vec3); WHEELS] {
    let x = ((scale[0] * 0x90) >> 12) as f32;
    let y = ((scale[1] * 0x40) >> 12) as f32;
    let front_z = ((scale[2] * 0xc7) >> 12) as f32;
    let rear_z = ((scale[2] * -0x60) >> 12) as f32;
    let centers = [
        Vec3::new(x, y, front_z),
        Vec3::new(-x, y, front_z),
        Vec3::new(x, y, rear_z),
        Vec3::new(-x, y, rear_z),
    ];
    let mut rims = [
        Vec3::new(x - 4096.0, 0.0, front_z),
        Vec3::new(-x + 4096.0, 0.0, front_z),
        Vec3::new(x - 4096.0, 0.0, rear_z),
        Vec3::new(-x + 4096.0, 0.0, rear_z),
    ];
    // The front axles follow the steering. (The hazard wobble of retail's
    // `hazardTimer` is not drawn.)
    let (sin, cos) = math::sin_cos(((wheel_rotation as i32) << 2) as u32);
    let (sin, cos) = (sin as f32, cos as f32);
    rims[0] = Vec3::new(centers[0].x - cos, 0.0, centers[0].z + sin);
    rims[1] = Vec3::new(centers[1].x + cos, 0.0, centers[1].z - sin);
    std::array::from_fn(|i| (centers[i], rims[i]))
}

/// `sDrawTiresSpriteIndexTable[min(|angle| >> 5, 0x80)]`
fn sprite_for(angle: f32) -> usize {
    let index = ((angle.abs() as i32) >> 5).min(0x80);
    SPRITE_INDEX[index as usize] as usize
}

/// Where a kart's wheels are drawn for a camera at `eye`.
///
/// `position` and `rotation` are the kart's (instance) pose in world units,
/// `scale` the instance scale per axis (0x1000 = 1.0) and `wheel_size` retail's
/// `Driver::wheelSize`.
pub fn wheel_quads(
    position: Vec3,
    rotation: Mat3,
    scale: [i32; 3],
    wheel_size: i32,
    wheel_rotation: i16,
    eye: Vec3,
) -> [WheelQuad; WHEELS] {
    let radius = radius(wheel_size);
    let pairs = local_pairs(scale, wheel_rotation);
    std::array::from_fn(|i| {
        let (center, rim) = pairs[i];
        let center = position + rotation * (center / FINE * SCALE);
        let axle = rotation * rim;

        // `viewNormal`: from the wheel to the camera.
        let to_eye = (eye - center).normalize_or_zero();
        // The square's sides: across the axle, and along it (as seen from the
        // camera). `axisB` is the negative of the second cross product.
        let across = to_eye.cross(axle).normalize_or_zero();
        let along = -to_eye.cross(across).normalize_or_zero();
        let corner = |a: f32, b: f32| center + across * (a * radius) + along * (b * radius);
        let [p0, p1, p2, p3] = [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(-1.0, 1.0),
            corner(1.0, 1.0),
        ];

        // `DrawTiresSolid_ApplyCornerOrder`: the picture is mirrored when the
        // axle points away from the camera, and the right-hand wheels are
        // the left-hand ones the other way round.
        let angle = axle.dot(to_eye);
        let right_hand = i % 2 == 1;
        let corners = match (right_hand, angle < 0.0) {
            (false, false) => [p3, p1, p2, p0],
            (true, false) => [p2, p0, p3, p1],
            (false, true) => [p1, p3, p0, p2],
            (true, true) => [p0, p2, p1, p3],
        };

        let side = rotation * Vec3::new(if right_hand { -1.0 } else { 1.0 }, 0.0, 0.0);
        WheelQuad {
            corners,
            sprite: sprite_for(angle),
            center,
            near_side: side.dot(to_eye) > 0.0,
        }
    })
}

/// The 17 icons in a column of 32x32 sprites, with their transparent texels
/// clear and their STP ones half transparent, as the tyre primitive blends them
/// (`Blend::Average`). Returns the picture and each icon's UV corners
/// (top left, top right, bottom left, bottom right).
pub fn sprite_atlas(icons: &[Icon], vram: &Vram) -> (Atlas, Vec<[[f32; 2]; 4]>) {
    let height = icons.len() * SPRITE_SIZE;
    let mut rgba = vec![0u8; SPRITE_SIZE * height * 4];
    for (n, icon) in icons.iter().enumerate() {
        let layout = &icon.layout;
        let (u0, v0) = layout.uv[0];
        for y in 0..SPRITE_SIZE {
            for x in 0..SPRITE_SIZE {
                // The icons are 32 texels wide and tall; their last UV is the last texel.
                let (u, v) = (u0.wrapping_add(x as u8), v0.wrapping_add(y as u8));
                let Ok(Some((mut c, stp))) = vram.texel_stp(layout, u, v) else {
                    continue;
                };
                if stp && layout.blend() == Blend::Average {
                    c[3] = 128;
                }
                rgba[((n * SPRITE_SIZE + y) * SPRITE_SIZE + x) * 4..][..4].copy_from_slice(&c);
            }
        }
    }
    let uvs = (0..icons.len())
        .map(|n| {
            let top = (n * SPRITE_SIZE) as f32 / height as f32;
            let bottom = ((n + 1) * SPRITE_SIZE) as f32 / height as f32;
            [[0.0, top], [1.0, top], [0.0, bottom], [1.0, bottom]]
        })
        .collect();
    (
        Atlas {
            width: SPRITE_SIZE,
            height,
            rgba,
        },
        uvs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::bigindex::BigIndex;
    use loader::disc::open_bigfile;
    use loader::dram::DramFile;
    use loader::icons::parse_groups;
    use loader::vram::parse_vram_file;

    #[test]
    fn sprite_table_covers_all_icons() {
        assert_eq!(SPRITE_INDEX[0], 0);
        assert_eq!(*SPRITE_INDEX.last().unwrap(), 16);
        assert!(SPRITE_INDEX.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(sprite_for(-4096.0), 16);
        assert_eq!(sprite_for(100.0), 1);
    }

    #[test]
    fn side_on_wheel_is_flat_and_end_on_wheel_faces_the_camera() {
        let eye_side = Vec3::new(500.0, 0.2, 0.0);
        let quads = wheel_quads(
            Vec3::ZERO,
            Mat3::IDENTITY,
            [0xccc; 3],
            WHEEL_SIZE,
            0,
            eye_side,
        );
        // Looking along the axle: the picture of the tyre seen from the side. (The rim vector
        // is the axle plus the wheel's offset, so retail's side-on view is a little short of 16.)
        assert!(quads[0].sprite >= 14, "{}", quads[0].sprite);
        assert!(quads[0].near_side && !quads[1].near_side);
        let eye_front = Vec3::new(0.0, 0.2, 500.0);
        let quads = wheel_quads(
            Vec3::ZERO,
            Mat3::IDENTITY,
            [0xccc; 3],
            WHEEL_SIZE,
            0,
            eye_front,
        );
        // Looking across the axle: the thin tread.
        assert!(quads[0].sprite <= 1, "{}", quads[0].sprite);
        // The square is as big as the picture says: 2 * 12.8 game units.
        let size = (quads[0].corners[0] - quads[0].corners[3]).length();
        assert!(
            (size / SCALE - 12.8 * 2.0 * 2.0f32.sqrt()).abs() < 0.5,
            "{size}"
        );
    }

    #[test]
    fn sprites_decode_from_the_disc() {
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
        let groups = parse_groups(&DramFile::parse(&pack).unwrap()).unwrap();
        let icons = &groups.iter().find(|g| g.group_id == 0).unwrap().icons;
        let (atlas, uvs) = sprite_atlas(icons, &vram);
        assert_eq!(uvs.len(), 17);
        for n in 0..17 {
            let sprite =
                &atlas.rgba[n * SPRITE_SIZE * SPRITE_SIZE * 4..][..SPRITE_SIZE * SPRITE_SIZE * 4];
            let visible = sprite.chunks(4).filter(|p| p[3] != 0).count();
            assert!(visible > 100, "icon {n}: {visible} visible texels");
        }
    }
}
