//! `data.bakedGteMath`, so far the wheelie: the kart rears up while a boost at
//! full fire speed burns, and settles back down after it. Each tick of the
//! animation is a rotation and a squash (`Vector_BakeMatrixTable`) that
//! `VehPhysForce_TranslateMatrix_UpdateInstanceMatrix` puts on the facing matrix,
//! with an offset of the kart's origin that follows it.
//!
//! The crash fall, squish recovery, blasted and airborne tables are not ported:
//! with those arrays the instance matrix is the facing matrix.

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


const BLEND_FULL: i32 = 0x100;

/// The entries of an animation, if it is ported.
fn entries(array: MatrixArray) -> Option<&'static [Entry]> {
    match array {
        MatrixArray::WheelieStart => Some(&WHEELIE_START),
        MatrixArray::WheelieHold => Some(&WHEELIE_HOLD),
        MatrixArray::WheelieRecover => Some(&WHEELIE_RECOVER),
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
    ((blend * (to as i32 - 1)) >> 8) as u8
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
        d.matrix_array = MatrixArray::CrashFall;
        assert_eq!(inst_matrix(&d).0, plain);
    }
}
