//! The objects a level places as instances, ticked at 30 Hz like the game's threads (`game/231`).
//! Plain state and a pure `tick`, so they can go in the rollback state next to the drivers:
//! [`Objects`] is all of a level's, built from its definition and cloneable.
//!
//! There are two kinds. The [`hazard`]s hurt a driver in reach, or throw it: each tick says where
//! one looks for a driver ([`Contact`]); what hurting one does (`VehPickState_NewState`) is not
//! ported. The [`pickup`]s are collected or broken by a touch of their hitbox. What they have in
//! common is [`Instance`]: a thread that ticks, and what the kart's collision search does at the
//! hitbox (`LInC`).

pub mod hazard;
pub mod pickup;

use loader::dram::DramFile;
use loader::lev::{InstDef, Level, PosRot, SVec3};
use loader::model::Model;
use vehicle::search::Instances;
use vehicle::{Driver, Sound3D};

use hazard::armadillo::Armadillo;
use hazard::drum::Drum;
use hazard::fireball::{Fireball, Spark};
use hazard::minecart::Minecart;
use hazard::roller::{self, Roller};
use hazard::turtle::Turtle;
use pickup::crates::{CrateBox, Kind};
use pickup::fruit::Fruit;

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

pub(crate) use math::interpolate;

/// What an object of a level does: its thread, which ticks, and what the kart's collision search
/// does when it reaches its hitbox. Both have a default of nothing, as an instance without a
/// `ThTick` or an `LInC` has.
pub trait Instance {
    /// `RB_*_ThTick`: one game tick. What the object did goes in `out`.
    fn th_tick(&mut self, _cx: &Context, _out: &mut Tick) {}

    /// `RB_*_LInC` (`MetaDataMODEL::LInC`): the kart's collision search reached the object's
    /// hitbox. The result is what the search takes it as: 1 stops nothing (the default, as for the
    /// models without an `LInC`), 0 means the object is not there to be touched.
    fn touch(&mut self, _hit: &mut Touch) -> i32 {
        1
    }
}

/// What a thread of an object can see besides itself.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The instance of the level that the object moves (an index into `Level::instances`).
    pub instance: usize,
    /// The frames of the first two animations of the object's model (`INSTANCE_GetNumAnimFrames`).
    pub frames: [i32; 2],
    /// The ticks run: `gGT->timer`.
    pub ticks: u32,
    /// `ptrSpawnType2`'s first path, which the minecarts follow.
    pub position_path: &'a [SVec3],
    /// `ptrSpawnType2_PosRot`, which the rollers follow.
    pub roll_paths: &'a [Vec<PosRot>],
}

/// A touch of a hitbox: the driver, and the `Input1.modelID` of the search that reached it.
pub struct Touch<'a> {
    pub driver: &'a mut Driver,
    pub model: i16,
    /// `sdata->randomNumber`, which breaking a fruit crate advances.
    pub random_number: &'a mut i32,
}

/// An object and the instance of the level that it moves (an index into `Level::instances`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed<T> {
    pub instance: usize,
    pub sim: T,
    /// The frames of the first two animations of its model (`INSTANCE_GetNumAnimFrames`).
    pub frames: [i32; 2],
}

/// What one tick of the objects of a level did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tick {
    /// Where hazards looked for drivers.
    pub contacts: Vec<Contact>,
    /// The sparks the fireballs left.
    pub sparks: Vec<Spark>,
    /// The sounds the objects played (`PlaySound3D`).
    pub sounds: Vec<Sound3D>,
    /// The loops the objects keep going (`PlaySound3D_Flags`), by the instance that plays each:
    /// a loop is in every tick it plays.
    pub loops: Vec<(usize, Sound3D)>,
}

/// All the objects of a level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Objects {
    pub armadillos: Vec<Placed<Armadillo>>,
    pub fireballs: Vec<Placed<Fireball>>,
    pub minecarts: Vec<Placed<Minecart>>,
    pub rollers: Vec<Placed<Roller>>,
    pub drums: Vec<Placed<Drum>>,
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

/// Where an object of the level is kept: its kind, and its index among the objects of the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Armadillo(usize),
    Fireball(usize),
    Minecart(usize),
    Roller(usize),
    Drum(usize),
    Turtle(usize),
    Fruit(usize),
    Crate(usize),
}

/// An object of the level as [`Objects::each_object`] hands it out: whatever its kind, it is only
/// an [`Instance`].
pub struct Entry<'a> {
    /// The instance of the level that it moves.
    pub instance: usize,
    pub object: &'a mut dyn Instance,
    /// What its thread sees, with its own `frames`.
    pub cx: Context<'a>,
    /// `sdata->randomNumber`, for a [`Touch`].
    pub random_number: &'a mut i32,
}

/// The objects of one kind, to `f`.
fn each<T: Instance>(
    list: &mut [Placed<T>],
    cx: &Context,
    random_number: &mut i32,
    f: &mut impl FnMut(Entry),
) {
    for placed in list {
        f(Entry {
            instance: placed.instance,
            object: &mut placed.sim,
            cx: Context {
                instance: placed.instance,
                frames: placed.frames,
                ..*cx
            },
            random_number: &mut *random_number,
        });
    }
}

/// The slots of the objects of one kind.
fn slots_of<T>(list: &[Placed<T>], slot: fn(usize) -> Slot) -> impl Iterator<Item = (usize, Slot)> {
    list.iter()
        .enumerate()
        .map(move |(i, placed)| (placed.instance, slot(i)))
}

impl Objects {
    /// The objects the level places (`RB_*_LInB` of each instance), at tick 0. `dram` is the
    /// level's file, which the instances' models are in.
    pub fn new(level: &Level, dram: &DramFile) -> Objects {
        let mut objects = Objects {
            armadillos: Vec::new(),
            fireballs: Vec::new(),
            minecarts: Vec::new(),
            rollers: Vec::new(),
            drums: Vec::new(),
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
                    model
                        .as_ref()
                        .and_then(|m| m.headers.first())
                        .and_then(|h| h.animations.get(i))
                        .map_or(0, |a| a.num_frames as i32)
                };
                [animation(0), animation(1)]
            };
            if let Some(kind) = Kind::of_model(def.model_id) {
                objects.crates.push(Placed {
                    instance,
                    sim: CrateBox::new(kind),
                    frames: [0; 2],
                });
                continue;
            }
            match def.model_name.as_str() {
                "armadillo" => {
                    let delay = digit(def)
                        .and_then(|d| level.spawn_delays.get(d))
                        .copied()
                        .unwrap_or(0);
                    objects.armadillos.push(Placed {
                        instance,
                        sim: Armadillo::new(def, delay),
                        frames: frames(),
                    });
                }
                "turtle" => {
                    let frames = frames();
                    objects.turtles.push(Placed {
                        instance,
                        sim: Turtle::new(def, frames[0]),
                        frames,
                    });
                }
                "fruit" => objects.fruit.push(Placed {
                    instance,
                    sim: Fruit::default(),
                    frames: [0; 2],
                }),
                "fireball" => objects.fireballs.push(Placed {
                    instance,
                    sim: Fireball::new(def),
                    frames: [0; 2],
                }),
                "minecart" => {
                    if let Some(sim) = Minecart::new(def, &objects.position_path) {
                        objects.minecarts.push(Placed {
                            instance,
                            sim,
                            frames: frames(),
                        });
                    }
                }
                "drum" => {
                    if !level.paths.first().is_some_and(|path| !path.is_empty()) {
                        continue;
                    }
                    objects.drums.push(Placed {
                        instance,
                        sim: Drum::new(&def.name, objects.position_path.len()),
                        frames: frames(),
                    });
                }
                "snowball" | "barrel" => {
                    if let Some(path) = digit(def)
                        .filter(|&p| level.paths.get(p).is_some_and(|path| path.len() > 1))
                    {
                        let barrel = def.model_name == "barrel";
                        objects.rollers.push(Placed {
                            instance,
                            sim: Roller { path, barrel },
                            frames: [0; 2],
                        });
                    }
                }
                _ => {}
            }
        }
        objects
    }

    /// The ticks run so far.
    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    /// Every object, in the order their threads run (which the sim depends on). This and
    /// [`Objects::slots`] are where the kinds are listed.
    pub fn each_object(&mut self, mut f: impl FnMut(Entry)) {
        let Objects {
            armadillos,
            fireballs,
            minecarts,
            rollers,
            drums,
            turtles,
            fruit,
            crates,
            random_number,
            position_path,
            roll_paths,
            ticks,
        } = self;
        let cx = Context {
            instance: 0,
            frames: [0; 2],
            ticks: *ticks,
            position_path,
            roll_paths,
        };
        each(armadillos, &cx, random_number, &mut f);
        each(fireballs, &cx, random_number, &mut f);
        each(minecarts, &cx, random_number, &mut f);
        each(turtles, &cx, random_number, &mut f);
        each(fruit, &cx, random_number, &mut f);
        each(crates, &cx, random_number, &mut f);
        each(rollers, &cx, random_number, &mut f);
        each(drums, &cx, random_number, &mut f);
    }

    /// The instance of each object and where the object is kept, for the ones that show them.
    pub fn slots(&self) -> impl Iterator<Item = (usize, Slot)> {
        slots_of(&self.armadillos, Slot::Armadillo)
            .chain(slots_of(&self.fireballs, Slot::Fireball))
            .chain(slots_of(&self.minecarts, Slot::Minecart))
            .chain(slots_of(&self.rollers, Slot::Roller))
            .chain(slots_of(&self.drums, Slot::Drum))
            .chain(slots_of(&self.turtles, Slot::Turtle))
            .chain(slots_of(&self.fruit, Slot::Fruit))
            .chain(slots_of(&self.crates, Slot::Crate))
    }

    /// One game tick of every object.
    pub fn tick(&mut self) -> Tick {
        let mut tick = Tick::default();
        self.each_object(|e| e.object.th_tick(&e.cx, &mut tick));
        self.ticks += 1;
        tick
    }

    /// Where a barrel is now and where it is a tick later, to draw it between the two.
    pub fn drum_frames(&self, drum: usize) -> Option<(PosRot, PosRot)> {
        let path = self.roll_paths.first().filter(|path| !path.is_empty())?;
        let point = self.drums[drum].sim.point;
        Some((Drum::frame(path, point), Drum::frame(path, point + 1)))
    }

    /// Where a roller is now and where it is a tick later, to draw it between the two.
    pub fn roller_frames(&self, roller: usize) -> (PosRot, PosRot) {
        let path = &self.roll_paths[self.rollers[roller].sim.path];
        let at = |tick: u32| {
            path[roller::frame_index(path.len(), tick).expect("a path of two points or more")]
        };
        (at(self.ticks), at(self.ticks + 1))
    }
}

/// What the kart's collision search does when it reaches the hitbox of one of these instances
/// (`MetaDataMODEL::LInC`): [`Instance::touch`] of the object. The turtles, the wumpa fruit and the
/// crates have one so far; the other models give 1, as those without an `LInC` do.
impl Instances for Objects {
    fn touch(&mut self, instance: usize, driver: &mut Driver, sps: &coll::Scratchpad) -> i32 {
        self.hit(instance, driver, sps.input1.model_id)
    }
}

impl Objects {
    /// The `LInC` of the object on `instance` for a hit of `model` (`Input1.modelID`): a driver's
    /// touch, or the blast of a weapon (`RB_Burst_CollLevInst`), whose owner is `driver`. 1 for an
    /// instance with no object.
    pub fn hit(&mut self, instance: usize, driver: &mut Driver, model: i16) -> i32 {
        let mut result = None;
        self.each_object(|e| {
            if e.instance == instance {
                result = Some(e.object.touch(&mut Touch {
                    driver: &mut *driver,
                    model,
                    random_number: e.random_number,
                }));
            }
        });
        result.unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contact_is_a_sphere_of_the_squared_radius() {
        // The armadillo's 0x2400 is a radius of 96 units.
        let contact = Contact {
            pos: [100, 20, -50],
            damage: Damage::Spin,
            hit_radius: 0x2400,
            sound: 0x71,
            shape: Shape::Sphere,
        };
        assert!(contact.hits([100 + 95, 20, -50]));
        assert!(!contact.hits([100 + 96, 20, -50]));
        // Height counts too.
        assert!(!contact.hits([100, 20 + 96, -50]));
        assert!(contact.hits([100 + 60, 20 + 60, -50]));
    }

    #[test]
    fn the_minecart_reaches_up_without_end_and_down_a_little() {
        let cart = Contact {
            pos: [0, 0, 0],
            damage: Damage::Squish,
            hit_radius: 0x10000,
            sound: 0,
            shape: Shape::Cylinder,
        };
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
