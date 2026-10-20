//! The track's moving objects, ticked at 30 Hz like the game's threads (`game/231`). Plain state
//! and a pure `tick`, so they can go in the rollback state next to the drivers: [`Hazards`] is
//! all of a level's, built from its definition and cloneable.
//!
//! The armadillos of Dingo Canyon, the fireballs and the turtles of Mystery Caves, the minecarts of Dragon Mines,
//! and the snowball and the barrels that roll along recorded paths. Each tick says where a hazard
//! looks for a driver ([`Contact`]); what hurting one does (`VehPickState_NewState`) is not ported.

pub mod armadillo;
pub mod crates;
pub mod fireball;
pub mod fruit;
pub mod minecart;
pub mod roller;
pub mod turtle;

use math::Wrap;
use loader::dram::DramFile;
use loader::lev::{InstDef, Level, PosRot, SVec3};
use vehicle::Driver;
use vehicle::search::Instances;
use loader::model::Model;

use armadillo::Armadillo;
use crates::{CrateBox, Kind};
use fireball::{Fireball, Spark};
use fruit::Fruit;
use minecart::Minecart;
use roller::Roller;
use turtle::Turtle;

/// `VEH_PICK_DAMAGE_*`: what a hit does to the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    /// The kart spins out.
    Spin = 1,
    /// The kart is blown up and tumbles.
    Blast = 2,
    /// The kart is flattened.
    Squish = 3,
    /// The kart is set on fire.
    Burn = 4,
}

/// The shape of the reach of a hazard (`LinkedCollide_Radius`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Sphere,
    /// The minecart: any height above 32 units under it.
    Cylinder,
}

/// `RB_CheckHazardCollisions` or `RB_FindDriverCollision`: where a hazard looks for drivers this
/// tick, and what a driver in reach gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contact {
    /// The hazard's `matrix.t`, in level units.
    pub pos: [i32; 3],
    pub damage: Damage,
    /// What `LinkedCollide_Radius` takes as the radius: a squared distance.
    pub hit_radius: u32,
    /// The sound a driver that was not spinning already hears when it is hit, 0 for none.
    pub sound: i32,
    pub shape: Shape,
}

impl Contact {
    /// `LinkedCollide_Radius`'s test against an instance at `target` (its `matrix.t`).
    pub fn hits(&self, target: [i32; 3]) -> bool {
        let [dx, dy, dz] = std::array::from_fn(|a| target[a].wrapping_sub(self.pos[a]));
        let flat = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) as u32;
        match self.shape {
            Shape::Sphere => flat.wrapping_add(dy.wrapping_mul(dy) as u32) < self.hit_radius,
            Shape::Cylinder => flat < self.hit_radius && -0x20 < dy,
        }
    }
}

/// `RB_Hazard_InterpolateValue` for 32 ms ticks: `current` moves `speed` angle units (4096 is a
/// turn) towards `desired` the short way, and lands on it when it is closer than that. Like the
/// game it compares the raw values and returns a normalised angle whenever they differ.
pub(crate) fn interpolate(current: i16, desired: i16, speed: i32) -> i16 {
    if current == desired {
        return current;
    }
    let mut diff = (desired as i32 - current as i32) & 0xfff;
    if diff > 0x7ff {
        diff -= 0x1000;
    }
    let result = if diff.abs() < speed {
        desired as i32
    } else if diff > 0 {
        current as i32 + speed
    } else {
        current as i32 - speed
    };
    (result & 0xfff).wrap_i16()
}

/// A hazard and the instance of the level that it moves (an index into `Level::instances`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed<T> {
    pub instance: usize,
    pub sim: T,
    /// The frames of the first two animations of its model (`INSTANCE_GetNumAnimFrames`).
    pub frames: [i32; 2],
}

/// What one tick of the hazards of a level did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tick {
    /// Where hazards looked for drivers.
    pub contacts: Vec<Contact>,
    /// The sparks the fireballs left.
    pub sparks: Vec<Spark>,
}

/// All the hazards of a level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hazards {
    pub armadillos: Vec<Placed<Armadillo>>,
    pub fireballs: Vec<Placed<Fireball>>,
    pub minecarts: Vec<Placed<Minecart>>,
    pub rollers: Vec<Placed<Roller>>,
    pub turtles: Vec<Placed<Turtle>>,
    pub fruit: Vec<Placed<Fruit>>,
    pub crates: Vec<Placed<CrateBox>>,
    /// `sdata->randomNumber`, which breaking a fruit crate advances.
    pub random_number: i32,
    /// `ptrSpawnType2`'s first path, which the minecarts follow.
    position_path: Vec<SVec3>,
    /// `ptrSpawnType2_PosRot`, which the rollers follow.
    roll_paths: Vec<Vec<PosRot>>,
    /// The ticks run: `gGT->timer`.
    ticks: u32,
}

/// The digit an instance name ends in, which the objects use as their number.
fn digit(def: &InstDef) -> Option<usize> {
    Some(def.name.chars().last()?.to_digit(10)? as usize)
}

impl Hazards {
    /// The hazards the level places (`RB_*_LInB` of each instance), at tick 0. `dram` is the
    /// level's file, which the instances' models are in.
    pub fn new(level: &Level, dram: &DramFile) -> Hazards {
        let mut hazards = Hazards {
            armadillos: Vec::new(),
            fireballs: Vec::new(),
            minecarts: Vec::new(),
            rollers: Vec::new(),
            turtles: Vec::new(),
            fruit: Vec::new(),
            crates: Vec::new(),
            random_number: math::RANDOM_NUMBER_INIT,
            position_path: level.position_paths.first().cloned().unwrap_or_default(),
            roll_paths: level.paths.clone(),
            ticks: 0,
        };
        for (instance, def) in level.instances.iter().enumerate() {
            let frames = || {
                let model = Model::parse_at(dram, def.model).ok();
                let animation = |i: usize| {
                    model.as_ref().and_then(|m| m.headers.first()).and_then(|h| h.animations.get(i)).map_or(0, |a| a.num_frames as i32)
                };
                [animation(0), animation(1)]
            };
            if let Some(kind) = Kind::of_model(def.model_id) {
                hazards.crates.push(Placed { instance, sim: CrateBox::new(kind), frames: [0; 2] });
                continue;
            }
            match def.model_name.as_str() {
                "armadillo" => {
                    let delay = digit(def).and_then(|d| level.spawn_delays.get(d)).copied().unwrap_or(0);
                    hazards.armadillos.push(Placed { instance, sim: Armadillo::new(def, delay), frames: frames() });
                }
                "turtle" => {
                    let frames = frames();
                    hazards.turtles.push(Placed { instance, sim: Turtle::new(def, frames[0]), frames });
                }
                "fruit" => hazards.fruit.push(Placed { instance, sim: Fruit::default(), frames: [0; 2] }),
                "fireball" => hazards.fireballs.push(Placed { instance, sim: Fireball::new(def), frames: [0; 2] }),
                "minecart" => {
                    if let Some(sim) = Minecart::new(def, &hazards.position_path) {
                        hazards.minecarts.push(Placed { instance, sim, frames: frames() });
                    }
                }
                "snowball" | "barrel" => {
                    if let Some(path) = digit(def).filter(|&p| level.paths.get(p).is_some_and(|path| path.len() > 1)) {
                        hazards.rollers.push(Placed { instance, sim: Roller { path }, frames: [0; 2] });
                    }
                }
                _ => {}
            }
        }
        hazards
    }

    /// The ticks run so far.
    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    /// One game tick of every hazard.
    pub fn tick(&mut self) -> Tick {
        let mut tick = Tick::default();
        for a in &mut self.armadillos {
            tick.contacts.extend(a.sim.tick(a.frames));
        }
        for f in &mut self.fireballs {
            if let Some(step) = f.sim.tick() {
                tick.sparks.push(step.spark);
                tick.contacts.push(step.contact);
            }
        }
        for m in &mut self.minecarts {
            tick.contacts.push(m.sim.tick(&self.position_path, m.frames[0]));
        }
        for t in &mut self.turtles {
            t.sim.tick(t.frames[0]);
        }
        for c in &mut self.crates {
            c.sim.tick();
        }
        for r in &self.rollers {
            let path = &self.roll_paths[r.sim.path];
            let frame = roller::frame_index(path.len(), self.ticks).expect("a path of two points or more");
            tick.contacts.push(roller::contact(&path[frame]));
        }
        self.ticks += 1;
        tick
    }

    /// Where a roller is now and where it is a tick later, to draw it between the two.
    pub fn roller_frames(&self, roller: usize) -> (PosRot, PosRot) {
        let path = &self.roll_paths[self.rollers[roller].sim.path];
        let at = |tick: u32| path[roller::frame_index(path.len(), tick).expect("a path of two points or more")];
        (at(self.ticks), at(self.ticks + 1))
    }
}

/// What the kart's collision search does when it reaches the hitbox of one of these instances
/// (`MetaDataMODEL::LInC`). The turtles, the wumpa fruit and the crates have one so far; the other models give 1,
/// as those without an `LInC` do.
impl Instances for Hazards {
    fn touch(&mut self, instance: usize, driver: &mut Driver, sps: &coll::Scratchpad) -> i32 {
        if let Some(turtle) = self.turtles.iter().find(|t| t.instance == instance) {
            return turtle.sim.linc(driver);
        }
        if let Some(fruit) = self.fruit.iter_mut().find(|f| f.instance == instance) {
            return fruit.sim.linc(driver, sps.input1.model_id);
        }
        if let Some(crate_box) = self.crates.iter_mut().find(|c| c.instance == instance) {
            return crate_box.sim.linc(driver, sps.input1.model_id, &mut self.random_number);
        }
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contact_is_a_sphere_of_the_squared_radius() {
        // The armadillo's 0x2400 is a radius of 96 units.
        let contact = Contact { pos: [100, 20, -50], damage: Damage::Spin, hit_radius: 0x2400, sound: 0x71, shape: Shape::Sphere };
        assert!(contact.hits([100 + 95, 20, -50]));
        assert!(!contact.hits([100 + 96, 20, -50]));
        // Height counts too.
        assert!(!contact.hits([100, 20 + 96, -50]));
        assert!(contact.hits([100 + 60, 20 + 60, -50]));
    }

    #[test]
    fn the_minecart_reaches_up_without_end_and_down_a_little() {
        let cart = Contact { pos: [0, 0, 0], damage: Damage::Squish, hit_radius: 0x10000, sound: 0, shape: Shape::Cylinder };
        assert!(cart.hits([100, 5000, 100]));
        assert!(cart.hits([100, -31, 100]));
        assert!(!cart.hits([100, -32, 100]));
        assert!(!cart.hits([300, 0, 0]));
    }

    #[test]
    fn the_turn_takes_the_short_way() {
        assert_eq!(interpolate(0x100, 0x800, 0x100), 0x200);
        // Exactly half a turn away goes the negative way, as the C does.
        assert_eq!(interpolate(0x100, 0x900, 0x100), 0);
        assert_eq!(interpolate(0x900, 0x100, 0x100), 0x800);
        assert_eq!(interpolate(0xf80, 0x80, 0x100), 0x80);
        assert_eq!(interpolate(0, 0xf00, 0x100), 0xf00);
        // A negative target is met by its normalised self.
        assert_eq!(interpolate(0x410, -0xc00, 0x20), 0x400);
    }
}
