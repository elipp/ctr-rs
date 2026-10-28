//! `RB_FlameJet_ThTick`: the flame jets of Tiger Temple blow for 45 ticks of each cycle of 106
//! and burn a kart in the box in front of them. The fire it blows ([`Flame`]) is the drawing's.

use loader::lev::InstDef;
use vehicle::rotmat::rot_to_matrix;
use vehicle::{Matrix, Sound3D};

use crate::{Contact, Context, Damage, Flame, Instance, Mines, Shape, Tick};

/// `fjBounds`: the box in front of the nozzle, in the jet's own space. It reaches 320 units.
const MIN: [i32; 3] = [-0x40, -0x40, 0];
const MAX: [i32; 3] = [0x40, 0x80, 0x140];
/// The jet blows while its timer is under this, and its roar is cut when it reaches it.
const BLOW_TICKS: i32 = 0x2d;
/// The timer starts over once it is past this.
const LAST_TICK: i32 = 0x69;
/// The roar of the flame, looped while it blows.
const ROAR_SFX: u8 = 0x68;
/// What the flame's speed is scaled by, from the nozzle's direction.
const FLAME_SPEED: i32 = 0x4b;

/// `struct FlameJet` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlameJet {
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    matrix: Matrix,
    /// How far its flame travels in a tick along x and z, in 1/256 units (`dirX`, `dirZ`).
    dir: [i16; 2],
    cycle_timer: i32,
    /// Ticks to wait before the first cycle, so that the jets are not in step.
    cooldown: i32,
}

impl FlameJet {
    /// `RB_FlameJet_LInB`: `delay` is the halfword of `ST1_SPAWN` for the digit its name ends
    /// in, 0 if the level has none.
    pub fn new(def: &InstDef, delay: u16) -> FlameJet {
        let matrix = rot_to_matrix(def.rot);
        FlameJet {
            pos: [def.pos.x as i32, def.pos.y as i32, def.pos.z as i32],
            matrix,
            dir: [
                (-(matrix.m[0][2] as i32) * FLAME_SPEED >> 5) as i16,
                ((matrix.m[2][2] as i32) * FLAME_SPEED >> 5) as i16,
            ],
            cycle_timer: 0,
            cooldown: delay as i32,
        }
    }

    /// One game tick. While it blows, where it looks for drivers, and its roar, go in `out`.
    pub fn tick(&mut self, instance: usize, ticks: u32, out: &mut Tick) {
        if self.cooldown != 0 {
            self.cooldown -= 1;
            return;
        }
        if self.cycle_timer < BLOW_TICKS {
            out.loops.push((
                instance,
                Sound3D {
                    id: ROAR_SFX,
                    pos: self.pos,
                },
            ));
            out.flames.push(Flame {
                pos: self.pos,
                dir: self.dir,
                ticks,
            });
            out.contacts.push(Contact {
                shape: Shape::Hitbox {
                    matrix: self.matrix,
                    min: MIN,
                    max: MAX,
                },
                mines: Mines::IfNoDriver,
                ..Contact::sphere(self.pos, Damage::Burn, 0, 0)
            });
        } else if self.cycle_timer > LAST_TICK {
            // (It goes on to count from 1: the cycle is one tick shorter than the timer's range.)
            self.cycle_timer = 0;
        }
        self.cycle_timer += 1;
    }
}

impl Instance for FlameJet {
    fn th_tick(&mut self, cx: &Context, out: &mut Tick) {
        self.tick(cx.instance, cx.ticks, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    fn jet(delay: u16) -> FlameJet {
        let def = InstDef {
            name: "flamejet#0".into(),
            model: 0,
            model_name: "flamejet".into(),
            scale: Default::default(),
            color_rgba: 0,
            flags: 0,
            pos: SVec3 { x: 5, y: 6, z: 7 },
            rot: SVec3::default(),
            model_id: 0,
        };
        FlameJet::new(&def, delay)
    }

    /// Which of `n` ticks burned.
    fn burning(jet: &mut FlameJet, n: usize) -> Vec<bool> {
        (0..n)
            .map(|_| {
                let mut out = Tick::default();
                jet.tick(3, 0, &mut out);
                assert_eq!(out.loops.len(), out.contacts.len());
                !out.contacts.is_empty()
            })
            .collect()
    }

    #[test]
    fn it_blows_for_forty_five_ticks_of_a_cycle() {
        let mut jet = jet(0);
        let ticks = burning(&mut jet, 45 + 61);
        assert!(ticks[..45].iter().all(|&b| b));
        assert!(ticks[45..].iter().all(|&b| !b));
        // The tick that starts it over is silent, and counts on to 1: later cycles blow for 44.
        assert_eq!(burning(&mut jet, 1), [false]);
        assert!(burning(&mut jet, 44).iter().all(|&b| b));
        assert_eq!(burning(&mut jet, 1), [false]);
    }

    #[test]
    fn it_blows_flames_the_way_it_faces() {
        // No turn: the matrix's third column is (0, 0, 1).
        let mut jet = jet(0);
        let mut out = Tick::default();
        jet.tick(0, 7, &mut out);
        assert_eq!(out.flames.len(), 1);
        let flame = out.flames[0];
        assert_eq!((flame.pos, flame.ticks), ([5, 6, 7], 7));
        assert_eq!(flame.dir, [0, ((4096 * 0x4b) >> 5) as i16]);
    }

    #[test]
    fn it_waits_its_delay() {
        let mut jet = jet(10);
        let ticks = burning(&mut jet, 12);
        assert!(ticks[..10].iter().all(|&b| !b));
        assert!(ticks[10..].iter().all(|&b| b));
    }

    #[test]
    fn it_burns_a_driver_in_front_of_the_nozzle() {
        let mut jet = jet(0);
        let mut out = Tick::default();
        jet.tick(0, 0, &mut out);
        let contact = out.contacts[0];
        assert_eq!(contact.damage, Damage::Burn);
        assert!(contact.hits([5, 6, 7 + 100]));
        assert!(!contact.hits([5, 6, 7 + 320]));
        assert!(!contact.hits([5, 6, 7 - 10]));
        assert!(!contact.hits([5 + 64, 6, 7 + 100]));
    }
}
