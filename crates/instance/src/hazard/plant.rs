//! `RB_Plant_*`: the man-eating plants of Papu's Pyramid. A plant rests, grows hungry, and while it
//! is hungry a kart in the box in front of its mouth is eaten: the kart is taken off the track
//! (`VEH_PICK_DAMAGE_MASK_GRAB`, see `vehicle::stuck::plant_eaten_init`) while the plant grabs,
//! chews and spits. A hungry plant that nobody walks into gets tired and rests again.
//!
//! The tires it spits out ([`Spit`]) are the drawing's, as is the camera that looks at the plant.

use loader::lev::InstDef;
use vehicle::rotmat::rot_to_matrix;
use vehicle::{Matrix, Sound3D};

use crate::{Contact, Context, Damage, Frames, Instance, Pose, Shape, Spit, Tick};

/// `PlantAnim_*`: the model's animations. Grabbing a mine is unused.
pub mod anim {
    pub const REST: u8 = 0;
    pub const REST_TO_HUNGRY: u8 = 1;
    pub const HUNGRY: u8 = 2;
    pub const GRAB_DRIVER: u8 = 3;
    pub const START_EAT: u8 = 5;
    pub const CHEW: u8 = 6;
    pub const SPIT: u8 = 7;
}

/// `plantBounds`: the mouth, in the plant's own space. It reaches 480 units ahead.
const MIN: [i32; 3] = [-0x40, -0x40, 0];
const MAX: [i32; 3] = [0x40, 0x80, 0x1e0];
/// Cycles of the rest animation before it grows hungry, of the hungry one before it gives up, and
/// of the chew before it spits.
const REST_CYCLES: u8 = 3;
const HUNGRY_CYCLES: u8 = 4;
const CHEW_CYCLES: u8 = 1;
/// The chew's frame that crunches, and the spit's frame that the tires leave at.
const CRUNCH_FRAME: i16 = 0xf;
const SPIT_FRAME: i16 = 0x19;
const GRAB_SFX: u8 = 0x6d;
const CHEW_SFX: u8 = 0x6e;
const SPIT_SFX: u8 = 0x6f;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// `RB_Plant_ThTick_Rest`: also grows hungry.
    Rest,
    /// `RB_Plant_ThTick_Hungry`
    Hungry,
    /// `RB_Plant_ThTick_Transition_HungryToRest`
    Calming,
    /// `RB_Plant_ThTick_Grab`
    Grabbing,
    /// `RB_Plant_ThTick_Eat`: starts to eat, chews and spits.
    Eating,
}

/// `struct Plant` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plant {
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    matrix: Matrix,
    /// Ticks to wait before the first move, so that the plants are not in step (`cooldown`).
    cooldown: i16,
    /// Which side of it the camera stands on when it has eaten a kart (`side`).
    side: bool,
    cycles: u8,
    /// A kart is in its mouth, not just a hungry plant (`boolEatingPlayer`): it is loud.
    eating_player: bool,
    phase: Phase,
    pub anim_index: u8,
    pub anim_frame: i16,
}

impl Plant {
    /// `RB_Plant_LInB`: at rest where the definition has it. `cooldown` and `side` are the entries
    /// of the level's table (`ptrSpawnType1`) for the digit its name ends in.
    pub fn new(def: &InstDef, cooldown: i16, side: i16) -> Plant {
        Plant {
            pos: [def.pos.x as i32, def.pos.y as i32, def.pos.z as i32],
            matrix: rot_to_matrix(def.rot),
            cooldown,
            side: side != 0,
            cycles: 0,
            eating_player: false,
            phase: Phase::Rest,
            anim_index: anim::REST,
            anim_frame: 0,
        }
    }

    /// Whether its animation has another frame after this one.
    fn more(&self, frames: &Frames) -> bool {
        (self.anim_frame as i32 + 1) < frames[self.anim_index as usize]
    }

    /// `ThTick_SetAndExec` to `phase`, playing `anim_index` from its first frame.
    fn start(&mut self, phase: Phase, anim_index: u8) {
        self.phase = phase;
        self.anim_index = anim_index;
        self.anim_frame = 0;
    }

    /// `RB_Plant_ThTick_Transition_HungryToRest`: a frame back, or at rest when that is the
    /// first. False if it went to rest, which runs at once.
    fn calm(&mut self) -> bool {
        if self.anim_frame - 1 > 0 {
            self.anim_frame -= 1;
            true
        } else {
            self.start(Phase::Rest, anim::REST);
            false
        }
    }

    /// One game tick. It looks for drivers in front of its mouth while it is hungry.
    pub fn tick(&mut self, frames: &Frames, out: &mut Tick) {
        let sound = |id, plant: &Plant| Sound3D { id, pos: plant.pos };
        // A thread that switches procedures runs the new one at once.
        loop {
            match self.phase {
                Phase::Rest => {
                    if self.cooldown != 0 {
                        self.cooldown = self.cooldown.wrapping_sub(1);
                    } else if self.anim_index == anim::REST {
                        if self.more(frames) {
                            self.anim_frame += 1;
                        } else {
                            self.anim_frame = 0;
                            self.cycles += 1;
                            if self.cycles == REST_CYCLES {
                                self.anim_index = anim::REST_TO_HUNGRY;
                                self.cycles = 0;
                            }
                        }
                    } else if self.anim_index == anim::REST_TO_HUNGRY {
                        if self.more(frames) {
                            self.anim_frame += 1;
                        } else {
                            self.start(Phase::Hungry, anim::HUNGRY);
                            continue;
                        }
                    }
                }
                Phase::Hungry => {
                    if self.anim_index == anim::HUNGRY {
                        if self.more(frames) {
                            self.anim_frame += 1;
                        } else {
                            self.anim_frame = 0;
                            self.cycles += 1;
                            if self.cycles == HUNGRY_CYCLES {
                                // It plays the way to hunger backwards, from its end.
                                self.cycles = 0;
                                self.phase = Phase::Calming;
                                self.anim_index = anim::REST_TO_HUNGRY;
                                self.anim_frame = frames[anim::REST_TO_HUNGRY as usize] as i16;
                                continue;
                            }
                        }
                        out.contacts.push(self.mouth());
                    }
                }
                Phase::Calming => {
                    if !self.calm() {
                        continue;
                    }
                }
                Phase::Grabbing => {
                    if self.more(frames) {
                        self.anim_frame += 1;
                    } else {
                        self.start(Phase::Eating, anim::START_EAT);
                        continue;
                    }
                }
                Phase::Eating => match self.anim_index {
                    anim::START_EAT => {
                        if self.more(frames) {
                            self.anim_frame += 1;
                        } else {
                            self.start(Phase::Eating, anim::CHEW);
                            if self.eating_player {
                                out.sounds.push(sound(CHEW_SFX, self));
                            }
                        }
                    }
                    anim::CHEW => {
                        if self.more(frames) {
                            self.anim_frame += 1;
                            if self.anim_frame == CRUNCH_FRAME && self.eating_player {
                                out.sounds.push(sound(CHEW_SFX, self));
                            }
                        } else {
                            self.anim_frame = 0;
                            self.cycles += 1;
                            if self.cycles == CHEW_CYCLES {
                                self.cycles = 0;
                                self.anim_index = anim::SPIT;
                            }
                        }
                    }
                    anim::SPIT => {
                        if self.more(frames) {
                            self.anim_frame += 1;
                            if self.anim_frame == SPIT_FRAME {
                                if self.eating_player {
                                    out.sounds.push(sound(SPIT_SFX, self));
                                }
                                out.spits.push(Spit {
                                    pos: self.pos,
                                    facing: [self.matrix.m[0][2], self.matrix.m[2][2]],
                                });
                            }
                        } else {
                            self.eating_player = false;
                            self.start(Phase::Rest, anim::REST);
                            continue;
                        }
                    }
                    _ => {}
                },
            }
            return;
        }
    }

    /// What the camera of a kart that it ate looks at.
    pub fn eater(&self) -> vehicle::stuck::PlantEater {
        vehicle::stuck::PlantEater {
            pos: self.pos,
            matrix: self.matrix,
            side: self.side,
        }
    }

    /// The box in front of its mouth, where a driver is eaten.
    fn mouth(&self) -> Contact {
        Contact {
            shape: Shape::Hitbox {
                matrix: self.matrix,
                min: MIN,
                max: MAX,
            },
            ..Contact::sphere(self.pos, Damage::Eat, 0, 0)
        }
    }
}

impl Instance for Plant {
    fn th_tick(&mut self, cx: &Context, out: &mut Tick) {
        self.tick(&cx.frames, out);
    }

    /// A driver was hurt by its mouth: it grabs, and gets on with it at once.
    fn caught(&mut self, cx: &Context, out: &mut Tick) {
        out.sounds.push(Sound3D {
            id: GRAB_SFX,
            pos: self.pos,
        });
        self.start(Phase::Grabbing, anim::GRAB_DRIVER);
        self.cycles = 0;
        self.eating_player = true;
        self.tick(&cx.frames, out);
    }

    fn pose(&self) -> Option<Pose> {
        Some(Pose {
            pos: self.pos,
            matrix: self.matrix,
            animation: (self.anim_index, self.anim_frame),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    /// Rest, to hungry, hungry, grab, (unused), start eating, chew, spit.
    const FRAMES: Frames = [4, 3, 5, 6, 0, 3, 20, 30];

    fn def() -> InstDef {
        InstDef {
            name: "plant#0".into(),
            model: 0,
            model_name: "plant".into(),
            scale: Default::default(),
            color_rgba: 0,
            flags: 0,
            pos: SVec3 {
                x: 1000,
                y: 50,
                z: -2000,
            },
            rot: SVec3 {
                x: 0,
                y: 0x400,
                z: 0,
            },
            model_id: 0,
        }
    }

    /// Ticks until it is hungry (the first with a contact).
    fn hungry(plant: &mut Plant) -> usize {
        let mut ticks = 0;
        loop {
            let mut out = Tick::default();
            plant.tick(&FRAMES, &mut out);
            ticks += 1;
            if !out.contacts.is_empty() {
                return ticks;
            }
            assert!(ticks < 200);
        }
    }

    #[test]
    fn it_waits_its_cooldown_then_rests_three_cycles_then_grows_hungry() {
        let mut plant = Plant::new(&def(), 17, 0);
        let mut out = Tick::default();
        for _ in 0..17 {
            plant.tick(&FRAMES, &mut out);
        }
        assert_eq!((plant.anim_index, plant.anim_frame), (anim::REST, 0));
        // 3 cycles of 4 frames rest, 3 of growing hungry (the last starts hungry at once).
        assert_eq!(hungry(&mut plant), 12 + 3);
        assert_eq!(plant.anim_index, anim::HUNGRY);
        assert!(out.contacts.is_empty());
    }

    #[test]
    fn a_hungry_plant_that_eats_nothing_goes_back_to_rest() {
        let mut plant = Plant::new(&def(), 0, 0);
        hungry(&mut plant);
        let mut contacts = 1;
        let mut ticks = 0;
        while plant.phase != Phase::Rest {
            let mut out = Tick::default();
            plant.tick(&FRAMES, &mut out);
            contacts += out.contacts.len();
            ticks += 1;
            assert!(ticks < 200);
        }
        // Four cycles of five frames of the hungry animation, but the tick that ends the last
        // already calms down.
        assert_eq!(contacts, 4 * 5 - 1);
        assert_eq!(plant.anim_index, anim::REST);
    }

    #[test]
    fn its_mouth_is_a_box_in_front_of_it() {
        let mut plant = Plant::new(&def(), 0, 0);
        let mut out = Tick::default();
        while out.contacts.is_empty() {
            plant.tick(&FRAMES, &mut out);
        }
        let mouth = out.contacts[0];
        assert_eq!(mouth.damage, Damage::Eat);
        // The plant faces +x: the mouth reaches 480 units that way, 64 to either side.
        let at = |dx, dy, dz| mouth.hits([1000 + dx, 50 + dy, -2000 + dz]);
        assert!(at(100, 0, 0));
        assert!(at(470, 100, 60));
        assert!(at(100, 0, -60));
        assert!(!at(490, 0, 0));
        assert!(!at(-10, 0, 0));
        assert!(!at(100, 0, 70));
        assert!(!at(100, 0, -70));
        // The height is the world's, 64 down to 127 up.
        assert!(at(100, 127, 0));
        assert!(!at(100, 128, 0));
        assert!(at(100, -64, 0));
        assert!(!at(100, -65, 0));
        assert!(!at(-100, 0, 0));
    }

    #[test]
    fn a_plant_that_ate_a_kart_grabs_chews_and_spits_it_out_loudly() {
        let mut plant = Plant::new(&def(), 0, 0);
        hungry(&mut plant);
        let cx = Context {
            instance: 4,
            frames: FRAMES,
            ticks: 0,
            position_path: &[],
            roll_paths: &[],
        };
        let mut out = Tick::default();
        plant.caught(&cx, &mut out);
        // The grab starts at once.
        assert_eq!((plant.anim_index, plant.anim_frame), (anim::GRAB_DRIVER, 1));
        let mut ticks = 0;
        while plant.phase != Phase::Rest {
            plant.tick(&FRAMES, &mut out);
            ticks += 1;
            assert!(ticks < 300);
            // It does not look for another kart while it has one.
            assert!(out.contacts.is_empty());
        }
        let ids: Vec<u8> = out.sounds.iter().map(|s| s.id).collect();
        assert_eq!(ids, [GRAB_SFX, CHEW_SFX, CHEW_SFX, SPIT_SFX]);
        assert!(!plant.eating_player);
    }

    #[test]
    fn a_plant_that_ate_nothing_is_quiet_when_it_goes_through_the_motions() {
        let mut plant = Plant::new(&def(), 0, 0);
        plant.phase = Phase::Eating;
        plant.anim_index = anim::START_EAT;
        let mut out = Tick::default();
        let mut ticks = 0;
        while plant.phase != Phase::Rest {
            plant.tick(&FRAMES, &mut out);
            ticks += 1;
            assert!(ticks < 200);
        }
        assert!(out.sounds.is_empty());
    }
}
