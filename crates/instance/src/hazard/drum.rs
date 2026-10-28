//! `RB_Baron_ThTick` for `DYNAMIC_DRUM`, the barrel of N. Gin Labs: it follows the first of the
//! level's recorded paths round and round, one point a tick, and squishes a driver it rolls over.

use loader::lev::PosRot;

use crate::{Contact, Context, Damage, Instance, Tick};

/// What `RB_Hazard_CollideWithDrivers` takes as the radius: a squared distance, 400 units.
const HIT_RADIUS: u32 = 0x19000;

/// The barrel is drawn a little off its path: added to `matrix.t`'s x, and taken from its z.
const OFFSET: [i32; 2] = [0x111, 0x110];

/// A barrel, at a point of the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drum {
    pub point: usize,
}

impl Drum {
    /// `RB_Baron_LInB`: a barrel whose name ends in 0 starts half a path on (`path_len` is the
    /// length of `ptrSpawnType2`'s first path, which retail takes for it); the others start at
    /// the second point.
    pub fn new(name: &str, path_len: usize) -> Drum {
        Drum {
            point: if name.ends_with('0') { path_len / 2 } else { 1 },
        }
    }

    /// The barrel at `point` of the path: where it is (`matrix.t`) and how it is turned.
    pub fn frame(path: &[PosRot], point: usize) -> PosRot {
        let mut frame = path[point % path.len()];
        frame.pos.x = (frame.pos.x as i32 + OFFSET[0]) as i16;
        frame.pos.z = (frame.pos.z as i32 - OFFSET[1]) as i16;
        frame
    }
}

impl Instance for Drum {
    fn th_tick(&mut self, cx: &Context, out: &mut Tick) {
        let Some(path) = cx.roll_paths.first().filter(|path| !path.is_empty()) else {
            return;
        };
        self.point = (self.point + 1) % path.len();
        let at = Drum::frame(path, self.point).pos;
        let pos = [at.x as i32, at.y as i32, at.z as i32];
        out.contacts
            .push(Contact::sphere(pos, Damage::Squish, HIT_RADIUS, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_goes_round_the_path() {
        let path: Vec<PosRot> = (0..4)
            .map(|i| PosRot {
                pos: loader::lev::SVec3 { x: i, y: 0, z: 0 },
                ..Default::default()
            })
            .collect();
        let paths = [path];
        let cx = Context {
            instance: 0,
            frames: [0; crate::ANIMATIONS],
            ticks: 0,
            position_path: &[],
            roll_paths: &paths,
            blockers: &[],
        };
        let mut drum = Drum::new("drum#0", 4);
        assert_eq!(drum.point, 2);
        let mut seen = Vec::new();
        for _ in 0..4 {
            drum.th_tick(&cx, &mut Tick::default());
            seen.push(drum.point);
        }
        assert_eq!(seen, [3, 0, 1, 2]);
        assert_eq!(Drum::new("drum#1", 4).point, 1);
        // Drawn off its path.
        assert_eq!(Drum::frame(&paths[0], 1).pos.x, 1 + 0x111);
    }
}
