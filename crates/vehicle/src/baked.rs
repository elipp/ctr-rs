//! `data.bakedGteMath`, so far the wheelie and the crash fall (the kart rears up while a boost at
//! full fire speed burns, and settles back down after it) and the airborne poses. Each tick of the
//! animation is a rotation and a squash (`Vector_BakeMatrixTable`) that
//! `VehPhysForce_TranslateMatrix_UpdateInstanceMatrix` puts on the facing matrix,
//! with an offset of the kart's origin that follows it.
//!
//! The squish recovery and blasted tables are not ported:
//! with those arrays the instance matrix is the facing matrix.

use math::Wrap;
use gte::Gte;
use loader::lev::SVec3;

use crate::exhaust::rotate;
use crate::rotmat::{convert_rot_to_matrix, matrix_rotate};
use crate::{Driver, Matrix, MatrixArray, actions};

/// One tick of an animation as authored: `MatrixNDAuthored`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Offset of the origin in the kart's frame, 1/256 game units.
    pub offset: [i16; 3],
    pub rot: [i16; 3],
    /// Squash and stretch, 0x1000 = 1.0.
    pub scale: [i16; 3],
}

pub const WHEELIE_START: [Entry; 11] = [
    Entry { offset: [0, -19, -537], rot: [-26, 0, 0], scale: [4096, 4262, 3686] },
    Entry { offset: [0, 45, -789], rot: [-53, 0, 0], scale: [4096, 4429, 3276] },
    Entry { offset: [0, 359, -162], rot: [-82, 0, 0], scale: [4096, 4096, 3596] },
    Entry { offset: [0, 779, 720], rot: [-112, 0, 0], scale: [4096, 4420, 4300] },
    Entry { offset: [0, 1234, 1471], rot: [-142, 0, 0], scale: [4096, 4217, 5004] },
    Entry { offset: [0, 1685, 1994], rot: [-171, 0, 0], scale: [4096, 3974, 5324] },
    Entry { offset: [0, 2093, 1958], rot: [-198, 0, 0], scale: [4096, 3771, 5233] },
    Entry { offset: [0, 2385, 1690], rot: [-222, 0, 0], scale: [4096, 3686, 5006] },
    Entry { offset: [0, 2431, 1398], rot: [-243, 0, 0], scale: [4096, 3750, 4710] },
    Entry { offset: [0, 2361, 1115], rot: [-259, 0, 0], scale: [4096, 3891, 4414] },
    Entry { offset: [0, 2232, 946], rot: [-269, 0, 0], scale: [4096, 4032, 4186] },
];

pub const WHEELIE_HOLD: [Entry; 1] = [
    Entry { offset: [0, 2132, 879], rot: [-273, 0, 0], scale: [4096, 4096, 4096] },
];

pub const WHEELIE_RECOVER: [Entry; 9] = [
    Entry { offset: [0, 1896, 865], rot: [-243, 0, 0], scale: [4096, 4057, 4171] },
    Entry { offset: [0, 1502, 844], rot: [-171, 0, 0], scale: [4096, 3974, 4338] },
    Entry { offset: [0, 716, 792], rot: [-82, 0, 0], scale: [4096, 3891, 4505] },
    Entry { offset: [0, 481, 1014], rot: [-2, 0, 0], scale: [4915, 3276, 4915] },
    Entry { offset: [0, 297, 541], rot: [-2, 0, 0], scale: [4596, 3595, 4596] },
    Entry { offset: [0, -22, -203], rot: [-2, 0, 0], scale: [4005, 4186, 4005] },
    Entry { offset: [0, -224, -389], rot: [-2, 0, 0], scale: [3686, 4505, 3686] },
    Entry { offset: [0, -287, -439], rot: [-2, 0, 0], scale: [3792, 4399, 3792] },
    Entry { offset: [0, -142, -220], rot: [-2, 0, 0], scale: [3989, 4202, 3989] },
];

/// `matArr04`: the crash fall. The kart is thrown up and back after slamming a wall, lands, and
/// squashes and stretches before it drives off.
pub const CRASH_FALL: [Entry; 16] = [
    Entry { offset: [0, 5954, 491], rot: [341, 0, 0], scale: [5632, 3328, 4403] },
    Entry { offset: [0, 11000, 4199], rot: [682, 0, 0], scale: [7168, 2560, 4710] },
    Entry { offset: [0, 10893, 3886], rot: [682, 0, 0], scale: [6345, 3011, 4524] },
    Entry { offset: [0, 10546, 3394], rot: [682, 0, 0], scale: [4816, 3849, 4179] },
    Entry { offset: [0, 9675, 2322], rot: [633, 0, 0], scale: [3993, 4300, 3993] },
    Entry { offset: [0, 8182, 1161], rot: [507, 0, 0], scale: [4020, 4247, 4020] },
    Entry { offset: [0, 5619, 171], rot: [338, 0, 0], scale: [4069, 4149, 4069] },
    Entry { offset: [0, 2727, -357], rot: [158, 0, 0], scale: [4096, 4096, 4096] },
    Entry { offset: [0, 182, 0], rot: [0, 0, 0], scale: [4710, 3788, 4710] },
    Entry { offset: [0, 1255, -108], rot: [69, 0, 0], scale: [5324, 3481, 5324] },
    Entry { offset: [0, 2330, -218], rot: [139, 0, 0], scale: [5057, 3690, 5057] },
    Entry { offset: [0, 2818, -267], rot: [170, 0, 0], scale: [4468, 4151, 4468] },
    Entry { offset: [0, 2342, -331], rot: [158, 0, 0], scale: [3879, 4612, 3879] },
    Entry { offset: [0, 1665, -315], rot: [126, 0, 0], scale: [3612, 4821, 3612] },
    Entry { offset: [0, 1019, -270], rot: [84, 0, 0], scale: [3737, 4633, 3737] },
    Entry { offset: [0, 487, -112], rot: [39, 0, 0], scale: [3970, 4284, 3970] },
];

/// `BAKED_GTE_MATRIX_JUMP_BASE`: the airborne poses start at this array.
const JUMP_BASE: u8 = 7;

/// The airborne pose per character (`matArr07` ..): the kart tilts and lifts a little more
/// the longer it is in the air, over four steps.
pub const JUMP: [[Entry; 4]; 13] = [
    [
        Entry { offset: [-209, 664, 179], rot: [-3, 2, -41], scale: [4096, 4096, 4096] },
        Entry { offset: [-672, 2127, 574], rot: [-6, 9, -132], scale: [4096, 4096, 4096] },
        Entry { offset: [-1133, 3591, 969], rot: [-10, 15, -224], scale: [4096, 4096, 4096] },
        Entry { offset: [-1343, 4255, 1149], rot: [-11, 18, -265], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-157, 43, 121], rot: [-49, 0, -27], scale: [4096, 4096, 4096] },
        Entry { offset: [-503, 137, 389], rot: [-153, 0, -88], scale: [4096, 4096, 4096] },
        Entry { offset: [-849, 231, 656], rot: [-256, 0, -148], scale: [4096, 4096, 4096] },
        Entry { offset: [-1006, 275, 778], rot: [-303, 0, -176], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [78, 328, -103], rot: [35, 0, -17], scale: [4096, 4096, 4096] },
        Entry { offset: [252, 1051, -330], rot: [113, 0, -56], scale: [4096, 4096, 4096] },
        Entry { offset: [425, 1774, -557], rot: [192, 0, -96], scale: [4096, 4096, 4096] },
        Entry { offset: [505, 2103, -659], rot: [227, 0, -113], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-83, 252, 0], rot: [-20, 0, -10], scale: [4096, 4096, 4096] },
        Entry { offset: [-267, 808, 0], rot: [-62, 0, -32], scale: [4096, 4096, 4096] },
        Entry { offset: [-452, 1364, 0], rot: [-103, 0, -54], scale: [4096, 4096, 4096] },
        Entry { offset: [-536, 1617, 0], rot: [-121, 0, -64], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [11, 176, -96], rot: [19, 0, -13], scale: [4096, 4096, 4096] },
        Entry { offset: [38, 566, -310], rot: [60, 0, -44], scale: [4096, 4096, 4096] },
        Entry { offset: [62, 954, -524], rot: [102, 0, -74], scale: [4096, 4096, 4096] },
        Entry { offset: [74, 1131, -621], rot: [121, 0, -88], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-107, 313, -298], rot: [31, 0, -14], scale: [4096, 4096, 4096] },
        Entry { offset: [-342, 1004, -954], rot: [100, 0, -44], scale: [4096, 4096, 4096] },
        Entry { offset: [-576, 1693, -1610], rot: [169, 0, -75], scale: [4096, 4096, 4096] },
        Entry { offset: [-684, 2008, -1909], rot: [201, 0, -89], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-61, 81, 115], rot: [-20, 0, -11], scale: [4096, 4096, 4096] },
        Entry { offset: [-196, 259, 369], rot: [-64, 0, -37], scale: [4096, 4096, 4096] },
        Entry { offset: [-331, 437, 625], rot: [-108, 0, -63], scale: [4096, 4096, 4096] },
        Entry { offset: [-393, 519, 740], rot: [-128, 0, -75], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-62, 180, -81], rot: [20, 0, -9], scale: [4096, 4096, 4096] },
        Entry { offset: [-199, 578, -260], rot: [65, 0, -30], scale: [4096, 4096, 4096] },
        Entry { offset: [-336, 976, -440], rot: [111, 0, -51], scale: [4096, 4096, 4096] },
        Entry { offset: [-399, 1157, -521], rot: [131, 0, -61], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-150, 114, 0], rot: [-4, 0, -28], scale: [4096, 4096, 4096] },
        Entry { offset: [-481, 363, 0], rot: [-13, 0, -92], scale: [4096, 4096, 4096] },
        Entry { offset: [-812, 613, 0], rot: [-22, 0, -155], scale: [4096, 4096, 4096] },
        Entry { offset: [-963, 727, 0], rot: [-26, 0, -184], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [0, 57, 38], rot: [-7, 0, -2], scale: [4096, 4096, 4096] },
        Entry { offset: [0, 199, 132], rot: [-25, 0, -7], scale: [4096, 4096, 4096] },
        Entry { offset: [0, 374, 247], rot: [-47, 0, -14], scale: [4096, 4096, 4096] },
        Entry { offset: [0, 533, 352], rot: [-67, 0, -20], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [0, 238, -83], rot: [18, 0, -6], scale: [4096, 4096, 4096] },
        Entry { offset: [0, 761, -267], rot: [58, 0, -20], scale: [4096, 4096, 4096] },
        Entry { offset: [0, 1284, -450], rot: [98, 0, -34], scale: [4096, 4096, 4096] },
        Entry { offset: [0, 1523, -534], rot: [117, 0, -40], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-145, 0, 148], rot: [-19, 0, -14], scale: [4096, 4096, 4096] },
        Entry { offset: [-463, 0, 475], rot: [-62, 0, -45], scale: [4096, 4096, 4096] },
        Entry { offset: [-782, 0, 803], rot: [-105, 0, -76], scale: [4096, 4096, 4096] },
        Entry { offset: [-927, 0, 951], rot: [-125, 0, -91], scale: [4096, 4096, 4096] },
    ],
    [
        Entry { offset: [-237, 373, 55], rot: [-10, 0, -19], scale: [4096, 4096, 4096] },
        Entry { offset: [-760, 1196, 174], rot: [-34, 0, -62], scale: [4096, 4096, 4096] },
        Entry { offset: [-1281, 2018, 294], rot: [-58, 0, -105], scale: [4096, 4096, 4096] },
        Entry { offset: [-1520, 2392, 349], rot: [-69, 0, -125], scale: [4096, 4096, 4096] },
    ],
];

const BLEND_FULL: i32 = 0x100;

/// The entries of an animation, if it is ported.
fn entries(array: MatrixArray) -> Option<&'static [Entry]> {
    match array {
        MatrixArray::WheelieStart => Some(&WHEELIE_START),
        MatrixArray::WheelieHold => Some(&WHEELIE_HOLD),
        MatrixArray::WheelieRecover => Some(&WHEELIE_RECOVER),
        MatrixArray::CrashFall => Some(&CRASH_FALL),
        MatrixArray::Jump(n) => JUMP.get(n.checked_sub(JUMP_BASE)? as usize).map(|e| e.as_slice()),
        _ => None,
    }
}

/// The matrix `Vector_BakeMatrixTable` makes of an entry: the squash applied to the rotation.
pub fn bake(gte: &mut Gte, entry: &Entry) -> Matrix {
    let [x, y, z] = entry.rot;
    let rot = convert_rot_to_matrix(gte, SVec3 { x, y, z });
    let mut scale = Matrix::default();
    for (a, s) in entry.scale.into_iter().enumerate() {
        scale.m[a][a] = s;
    }
    matrix_rotate(gte, &scale, &rot)
}

/// `VehPhysForce_TranslateMatrix_RemapIndex`: where in `to` an animation that is at `index` of
/// `from` continues, running the other way.
fn remap_index(index: u8, from: usize, to: usize) -> u8 {
    let quotient = ((index as i32) << 8) / (from as i32 - 1);
    let blend = (BLEND_FULL - quotient).clamp(0, BLEND_FULL);
    ((blend * (to as i32 - 1)) >> 8).wrap_u8()
}

/// `VehPhysForce_TranslateMatrix_UpdateMatrixAnimation`, for the wheelie:
/// it starts while a boost burns at full fire speed, and recovers when that ends.
pub fn update_matrix_animation(d: &mut Driver) {
    let boosting = d.reserves != 0
        && d.fire_speed >= d.stats.speed_class_stat
        && d.actions_flag_set & actions::TURBO_INPUT_LATCH == 0;
    let (start, recover) = (WHEELIE_START.len(), WHEELIE_RECOVER.len());
    if boosting {
        match d.matrix_array {
            MatrixArray::None => {
                d.matrix_array = MatrixArray::WheelieStart;
                d.matrix_index = 0;
            }
            MatrixArray::WheelieStart => {
                d.matrix_index += 1;
                if d.matrix_index as usize >= start {
                    d.matrix_array = MatrixArray::WheelieHold;
                    d.matrix_index = 0;
                }
            }
            MatrixArray::WheelieRecover => {
                d.matrix_index = remap_index(d.matrix_index, recover, start);
                d.matrix_array = MatrixArray::WheelieStart;
            }
            _ => {}
        }
    } else {
        match d.matrix_array {
            MatrixArray::WheelieHold => {
                d.matrix_array = MatrixArray::WheelieRecover;
                d.matrix_index = 0;
            }
            MatrixArray::WheelieStart => {
                d.matrix_index = remap_index(d.matrix_index, start, recover);
                d.matrix_array = MatrixArray::WheelieRecover;
            }
            MatrixArray::WheelieRecover => {
                d.matrix_index += 1;
                if d.matrix_index as usize >= recover {
                    d.matrix_array = MatrixArray::None;
                    d.matrix_index = 0;
                }
            }
            _ => {}
        }
    }
}

/// The kart model's matrix and position (`inst->matrix`) in game units: the facing matrix
/// with the animation on it, and the origin moved with it.
pub fn inst_matrix(d: &Driver) -> (Matrix, [i32; 3]) {
    let facing = d.matrix_facing_dir;
    let bounce = (d.screen_offset_y as i32 * 3) >> 3;
    let pos = [d.pos_curr.x, d.pos_curr.y, d.pos_curr.z];
    let baked = entries(d.matrix_array).and_then(|table| table.get(d.matrix_index as usize));
    match baked {
        Some(entry) => {
            let mut gte = Gte::default();
            let baked = bake(&mut gte, entry);
            let matrix = matrix_rotate(&mut gte, &facing, &baked);
            let moved = rotate(&facing, entry.offset.map(i32::from));
            (matrix, [(pos[0] + moved[0]) >> 8, ((pos[1] + moved[1]) >> 8) + bounce, (pos[2] + moved[2]) >> 8])
        }
        None => (facing, [pos[0] >> 8, (pos[1] >> 8) + bounce, pos[2] >> 8]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boosting() -> Driver {
        let mut d = Driver::new(0, 0);
        d.reserves = 100;
        d.fire_speed = 0x6000;
        d.stats.speed_class_stat = 0x5000;
        d.matrix_facing_dir = Matrix { m: [[0x1000, 0, 0], [0, 0x1000, 0], [0, 0, 0x1000]] };
        d
    }

    #[test]
    fn a_boost_rears_up_holds_and_settles_down() {
        let mut d = boosting();
        update_matrix_animation(&mut d);
        assert_eq!((d.matrix_array, d.matrix_index), (MatrixArray::WheelieStart, 0));
        for _ in 0..WHEELIE_START.len() - 1 {
            update_matrix_animation(&mut d);
        }
        assert_eq!((d.matrix_array, d.matrix_index), (MatrixArray::WheelieStart, 10));
        update_matrix_animation(&mut d);
        assert_eq!((d.matrix_array, d.matrix_index), (MatrixArray::WheelieHold, 0));
        update_matrix_animation(&mut d);
        assert_eq!(d.matrix_array, MatrixArray::WheelieHold);

        d.reserves = 0;
        update_matrix_animation(&mut d);
        assert_eq!((d.matrix_array, d.matrix_index), (MatrixArray::WheelieRecover, 0));
        for _ in 0..WHEELIE_RECOVER.len() {
            update_matrix_animation(&mut d);
        }
        assert_eq!((d.matrix_array, d.matrix_index), (MatrixArray::None, 0));
    }

    #[test]
    fn ending_the_boost_mid_rise_continues_from_the_same_height() {
        let mut d = boosting();
        for _ in 0..6 {
            update_matrix_animation(&mut d);
        }
        assert_eq!((d.matrix_array, d.matrix_index), (MatrixArray::WheelieStart, 5));
        d.reserves = 0;
        update_matrix_animation(&mut d);
        // Halfway up of 11 is halfway down of 9.
        assert_eq!(d.matrix_array, MatrixArray::WheelieRecover);
        assert_eq!(d.matrix_index, 4);
    }

    #[test]
    fn the_wheelie_pitches_the_kart_up_and_moves_its_origin() {
        let mut d = boosting();
        let (plain, _) = inst_matrix(&d);
        d.matrix_array = MatrixArray::WheelieHold;
        let (up, t) = inst_matrix(&d);
        assert_ne!(plain, up);
        // The hold is about 8 units up and 3 forward of the origin.
        assert_eq!(t, [0, 2132 >> 8, 879 >> 8]);
        // The body's forward axis (the third column) now points up a little.
        assert!(up.m[1][2] != 0);
        // Not ported arrays leave the facing matrix alone.
        d.matrix_array = MatrixArray::Jump(0);
        assert_eq!(inst_matrix(&d).0, plain);
    }

    #[test]
    fn the_crash_fall_throws_the_kart_up_and_lets_it_land() {
        let mut d = boosting();
        let (plain, _) = inst_matrix(&d);
        d.matrix_array = MatrixArray::CrashFall;
        let (first, t) = inst_matrix(&d);
        assert_ne!(first, plain);
        assert_eq!(t[1], 5954 >> 8);
        d.matrix_index = CRASH_FALL.len().wrap_u8() - 1;
        assert_ne!(inst_matrix(&d).0, plain);
        // Past the end, the animation is over.
        d.matrix_index = CRASH_FALL.len().wrap_u8();
        assert_eq!(inst_matrix(&d).0, plain);
    }

    #[test]
    fn the_airborne_pose_tilts_the_kart_more_as_the_jump_goes_on() {
        let mut d = boosting();
        let (plain, _) = inst_matrix(&d);
        d.matrix_array = MatrixArray::Jump(JUMP_BASE);
        let poses: Vec<_> = (0..4)
            .map(|i| {
                d.matrix_index = i;
                inst_matrix(&d)
            })
            .collect();
        assert!(poses.iter().all(|(m, _)| *m != plain));
        // Oxide's first step is about 0.8 units up, the last 16.
        assert_eq!(poses[0].1[1], 664 >> 8);
        assert_eq!(poses[3].1[1], 4255 >> 8);
    }
}
