//! `RB_Fireball_ThTick`: a ball of fire shoots out of the lava of Mystery Caves, up and back
//! down, and again. The ball itself is never drawn; it leaves a trail of sparks.

use math::Wrap;
use loader::lev::InstDef;

use crate::{Contact, Damage, Shape};

/// Milliseconds a tick lasts (`elapsedTimeMS`).
const TICK_MS: i16 = 32;
/// How far under its place the fireball starts, and where it is gone from the game's mind.
const BELOW_LAVA: i32 = 0x440;
const RISE_SPEED: i16 = 200;
/// Gravity a tick: `(elapsedTimeMS * 10) >> 5`.
const GRAVITY: i16 = 10;
const TERMINAL_SPEED: i16 = -200;
/// One cycle, in milliseconds.
const CYCLE_MS: i16 = 0xb40;
/// What the odd ones wait before they start, so that the fireballs are not in step.
const ODD_DELAY_MS: i16 = 1440;
/// How fast the spark rises per unit of the ball's speed, and its limit (1/256 units a tick).
const SPARK_SPEED_SCALE: i32 = 0x180;
const SPARK_SPEED_LIMIT: i32 = 0x7fff;
/// `RB_CheckHazardCollisions(fireInst, t, 4, 0x10000, 0)`: a squared distance of 256 units.
const HIT_RADIUS: u32 = 0x10000;

/// `struct Fireball` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fireball {
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    /// Where the definition puts it; the lava.
    rest_y: i32,
    vel_y: i16,
    cycle_timer: i16,
    cooldown: i16,
}

/// A spark the fireball leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spark {
    /// Level units.
    pub pos: [i32; 3],
    /// 1/256 units a tick.
    pub vel_y: i16,
}

/// What a fireball that is above the lava does in a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub spark: Spark,
    pub contact: Contact,
}

impl Fireball {
    /// `RB_Fireball_LInB`: the digit the name ends in makes the odd ones wait.
    pub fn new(def: &InstDef) -> Fireball {
        let id = def.name.chars().last().and_then(|c| c.to_digit(10)).unwrap_or(0);
        Fireball {
            pos: [def.pos.x as i32, def.pos.y as i32, def.pos.z as i32],
            rest_y: def.pos.y as i32,
            vel_y: 96,
            cycle_timer: 0,
            cooldown: if id & 1 != 0 { ODD_DELAY_MS } else { 0 },
        }
    }

    /// One game tick. The spark it leaves and where it looks for drivers, if it is above the lava.
    pub fn tick(&mut self) -> Option<Step> {
        if self.cooldown != 0 {
            self.cooldown = (self.cooldown - TICK_MS).max(0);
            return None;
        }

        let mut step = None;
        if self.pos[1] >= self.rest_y - BELOW_LAVA {
            // Moves with its speed, then gravity pulls it.
            self.pos[1] += (self.vel_y as i32 * TICK_MS as i32) >> 5;
            self.vel_y = (self.vel_y - GRAVITY).max(TERMINAL_SPEED);
            // The spark goes against the ball's vertical motion.
            let vel_y = (-(self.vel_y as i32) * SPARK_SPEED_SCALE).clamp(-SPARK_SPEED_LIMIT, SPARK_SPEED_LIMIT);
            step = Some(Step {
                spark: Spark { pos: self.pos, vel_y: vel_y.wrap_i16() },
                contact: Contact { pos: self.pos, damage: Damage::Burn, hit_radius: HIT_RADIUS, sound: 0, shape: Shape::Sphere },
            });
        }

        self.cycle_timer -= TICK_MS;
        if self.cycle_timer < 1 {
            // Back under the lava, rising: a new cycle.
            self.pos[1] = self.rest_y - BELOW_LAVA;
            self.vel_y = RISE_SPEED;
            self.cycle_timer = CYCLE_MS;
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    fn def(name: &str) -> InstDef {
        InstDef {
            name: name.into(),
            model: 0,
            model_name: String::new(),
            scale: SVec3::default(),
            color_rgba: 0,
            flags: 0,
            pos: SVec3 { x: 100, y: -1090, z: 200 },
            rot: SVec3::default(),
            model_id: 0,
        }
    }

    #[test]
    fn it_rises_slows_and_falls_back_to_the_lava() {
        let mut f = Fireball::new(&def("fireball#0"));
        // The first tick is the start-up one: it leaves a spark and goes under the lava.
        assert!(f.tick().is_some());
        assert_eq!(f.pos[1], -1090 - 0x440);
        let mut heights = vec![];
        while let Some(step) = f.tick() {
            heights.push(step.spark.pos[1]);
        }
        let top = *heights.iter().max().unwrap();
        // 200, 190, ...: the ball rises 200 + 190 + ... + 10 = 2100 units, then falls.
        assert_eq!(top, -1090 - 0x440 + 2100, "{heights:?}");
        assert!(heights.windows(2).take(15).all(|w| w[1] > w[0]));
        assert!(heights.last().unwrap() < &top);
    }

    #[test]
    fn the_spark_goes_against_the_ball() {
        let mut f = Fireball::new(&def("fireball#0"));
        f.tick();
        // Rising at 190 after gravity: the spark is pushed down, as hard as it can be.
        let spark = f.tick().unwrap().spark;
        assert_eq!(spark.vel_y as i32, -0x7fff);
        // Near the top it is slow and the spark is not.
        let mut f = Fireball::new(&def("fireball#0"));
        f.tick();
        let slow = std::iter::from_fn(|| f.tick()).find(|s| s.spark.vel_y.abs() < 0x7fff).unwrap();
        assert_eq!(slow.spark.vel_y as i32, -80 * 0x180);
    }

    #[test]
    fn a_fireball_burns_what_it_flies_through() {
        let mut f = Fireball::new(&def("fireball#0"));
        f.tick();
        let step = f.tick().unwrap();
        assert_eq!((step.contact.damage, step.contact.pos), (Damage::Burn, f.pos));
        // 256 units around it, but not a track's width.
        assert!(step.contact.hits([f.pos[0] + 200, f.pos[1], f.pos[2]]));
        assert!(!step.contact.hits([f.pos[0] + 300, f.pos[1], f.pos[2]]));
    }

    #[test]
    fn a_cycle_is_ninety_ticks_and_the_odd_ones_start_late() {
        let mut odd = Fireball::new(&def("fireball#1"));
        let mut ticks = 0;
        while odd.tick().is_none() {
            ticks += 1;
        }
        // 1440 ms of 32 ms ticks.
        assert_eq!(ticks, 45);
        let mut f = Fireball::new(&def("fireball#0"));
        f.tick();
        let start = f.clone();
        let mut n = 0;
        loop {
            f.tick();
            n += 1;
            if f == start {
                break;
            }
            assert!(n < 1000);
        }
        assert_eq!(n, 90);
    }
}
