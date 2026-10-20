//! `RB_Minecart_ThTick`: the minecarts of Dragon Mines run along a path of positions, taking
//! eight ticks between two points, turning towards their direction a little each tick. The
//! three carts are spread over the path by the digit their names end in.

use loader::lev::{InstDef, SVec3};
use vehicle::Matrix;
use vehicle::rotmat::convert_rot_to_matrix;

use crate::{Contact, Damage, Shape, interpolate};

/// `betweenPoints_numFrames` and `rotSpeed` of a cart.
const FRAMES_BETWEEN_POINTS: i16 = 8;
const ROT_SPEED: i32 = 0x20;
/// `RB_FindDriverCollision`'s radius: a squared distance, 256 units.
const HIT_RADIUS: u32 = 0x10000;

fn s16(v: i32) -> i16 {
    v as i16
}

/// `struct Minecart` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Minecart {
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    pos_start: [i16; 3],
    pos_end: [i16; 3],
    /// From the end to the start: the cart travels against it.
    dir: [i16; 3],
    pos_index: usize,
    curr_frame: i16,
    rot_curr: SVec3,
    rot_desired: SVec3,
    /// `animFrame`
    pub anim_frame: i16,
    matrix: Matrix,
}

impl Minecart {
    /// `RB_Minecart_LInB` on `path` (`ptrSpawnType2`'s first): the digit its name ends in puts it
    /// at the start of the path, a third of the way along, or two thirds.
    pub fn new(def: &InstDef, path: &[SVec3]) -> Option<Minecart> {
        let n = path.len();
        if n < 3 {
            return None;
        }
        let digit = def.name.chars().last().and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
        let pos_index = match digit {
            0 => 1,
            1 => n / 3,
            _ => (n << 1) / 3,
        };
        let at = |i: usize| [path[i].x, path[i].y, path[i].z];
        let mut cart = Minecart {
            pos: [def.pos.x as i32, def.pos.y as i32, def.pos.z as i32],
            pos_start: at(pos_index - 1),
            pos_end: at(pos_index),
            dir: [0; 3],
            pos_index,
            curr_frame: 0,
            rot_curr: SVec3::default(),
            rot_desired: SVec3::default(),
            anim_frame: 0,
            matrix: Matrix::default(),
        };
        cart.set_direction();
        Some(cart)
    }

    /// `RB_Minecart_SetDirection`
    fn set_direction(&mut self) {
        for a in 0..3 {
            self.dir[a] = self.pos_start[a].wrapping_sub(self.pos_end[a]);
        }
        let [dx, dy, dz] = self.dir.map(i32::from);
        let flat = math::square_root0_stub((dx * dx).wrapping_add(dz * dz));
        self.rot_desired.x = s16(math::ratan2(dy, flat));
        self.rot_desired.y = s16(math::ratan2(dx, dz) - 0x800);
    }

    /// The orientation (`matrix`, without its translation).
    pub fn matrix(&self) -> Matrix {
        self.matrix
    }

    /// One game tick. `path` is the one it was made with, `frames` the number of frames of its
    /// animation. Where it looks for drivers.
    pub fn tick(&mut self, path: &[SVec3], frames: i32) -> Contact {
        self.anim_frame = if (self.anim_frame as i32 + 1) < frames { self.anim_frame + 1 } else { 0 };

        let at = |i: usize| [path[i].x, path[i].y, path[i].z];
        if self.curr_frame < FRAMES_BETWEEN_POINTS {
            self.curr_frame += 1;
        } else {
            // Reached the point.
            self.curr_frame = 1;
            if self.pos_index + 1 < path.len() {
                self.pos_index += 1;
                self.pos_start = self.pos_end;
            } else {
                // The end of the path: back to its start.
                self.pos_index = 1;
                self.pos_start = at(0);
                self.pos = self.pos_start.map(i32::from);
            }
            self.pos_end = at(self.pos_index);
            self.set_direction();
            if self.pos_index == 1 {
                self.rot_curr = self.rot_desired;
            }
        }

        for a in 0..3 {
            self.pos[a] = self.pos_start[a] as i32 - (self.curr_frame as i32 * self.dir[a] as i32) / FRAMES_BETWEEN_POINTS as i32;
        }
        self.rot_curr.y = interpolate(self.rot_curr.y, self.rot_desired.y, ROT_SPEED);
        self.rot_curr.x = interpolate(self.rot_curr.x, self.rot_desired.x, ROT_SPEED);
        self.matrix = convert_rot_to_matrix(&mut gte::Gte::default(), self.rot_curr);

        Contact { pos: self.pos, damage: Damage::Squish, hit_radius: HIT_RADIUS, sound: 0, shape: Shape::Cylinder }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str) -> InstDef {
        InstDef {
            name: name.into(),
            model: 0,
            model_name: String::new(),
            scale: SVec3 { x: 4096, y: 4096, z: 4096 },
            color_rgba: 0,
            flags: 0,
            pos: SVec3::default(),
            rot: SVec3::default(),
            model_id: 0,
        }
    }

    fn path() -> Vec<SVec3> {
        [(0, 0, 0), (80, 0, 0), (80, 0, 80), (160, 40, 80)].map(|(x, y, z)| SVec3 { x, y, z }).to_vec()
    }

    #[test]
    fn it_goes_from_point_to_point_in_eight_ticks() {
        let path = path();
        let mut cart = Minecart::new(&def("minecart#0"), &path).unwrap();
        for tick in 1..=8 {
            cart.tick(&path, 10);
            assert_eq!(cart.pos, [10 * tick, 0, 0]);
        }
        // The ninth tick is the first towards the next point.
        cart.tick(&path, 10);
        assert_eq!(cart.pos, [80, 0, 10]);
        for _ in 0..7 {
            cart.tick(&path, 10);
        }
        assert_eq!(cart.pos, [80, 0, 80]);
    }

    #[test]
    fn at_the_end_of_the_path_it_starts_again() {
        let path = path();
        let mut cart = Minecart::new(&def("minecart#0"), &path).unwrap();
        // Three legs of eight ticks, then the cart is reset to the first point and heads for the
        // second on that tick.
        for _ in 0..24 {
            cart.tick(&path, 10);
        }
        assert_eq!(cart.pos, [160, 40, 80]);
        cart.tick(&path, 10);
        assert_eq!(cart.pos, [10, 0, 0]);
    }

    #[test]
    fn the_three_carts_start_in_different_places() {
        let path: Vec<_> = (0..50).map(|i| SVec3 { x: i * 10, y: 0, z: 0 }).collect();
        let index = |name: &str| Minecart::new(&def(name), &path).unwrap().pos_index;
        assert_eq!([index("minecart#0"), index("minecart#1"), index("minecart#2")], [1, 16, 33]);
    }

    #[test]
    fn it_turns_to_face_the_way_it_goes() {
        // A straight road along +x: the direction is the other way round, and so is the yaw.
        let path: Vec<_> = (0..10).map(|i| SVec3 { x: i * 80, y: 0, z: 0 }).collect();
        let mut cart = Minecart::new(&def("minecart#0"), &path).unwrap();
        assert_eq!(cart.rot_desired.y, (math::ratan2(-80, 0) - 0x800) as i16);
        for _ in 0..40 {
            cart.tick(&path, 10);
        }
        // It started facing 0 and turns by 0x20 a tick, the short way round.
        assert_eq!(cart.rot_curr.y, 0x400);
    }

    #[test]
    fn it_squashes_what_is_on_its_rails_and_above() {
        let path = path();
        let mut cart = Minecart::new(&def("minecart#0"), &path).unwrap();
        let contact = cart.tick(&path, 10);
        assert_eq!((contact.damage, contact.shape), (Damage::Squish, Shape::Cylinder));
        assert!(contact.hits([cart.pos[0] + 100, cart.pos[1] + 1000, cart.pos[2]]));
    }
}
