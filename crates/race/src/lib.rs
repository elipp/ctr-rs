//! A race, all of it that the next tick depends on: the karts, the level's objects, and the
//! little the bumps keep between ticks. Plain state and a `tick` from everyone's pads, with no
//! engine and no I/O, so that every client plays the same race and a copy of it is a snapshot:
//! [`Race`] is `Clone`, and what a clone is put back to plays on exactly as the original did.
//!
//! What a tick asks of the outside (sounds, rumble, puffs and sparks to draw) comes out as
//! [`KartTick`]s; nothing of it goes back in.

pub mod desync;
pub mod rollback;

use coll::Collision;
use instance::Objects;
use loader::lev::Level;
use vehicle::collide::Feedback;
use vehicle::exhaust::Puff;
use vehicle::sim::{Racer, TickResult, spawn};
use vehicle::sparks::{Landing, Wall};
use vehicle::{Driver, Effect, Env, Input, actions, kart_state};

/// One tick of a controller: [`Input`] without the taps, which follow from the buttons held in
/// the tick before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pad {
    pub held: u32,
    pub lx: u8,
    pub ly: u8,
    pub ry: u8,
}

impl Default for Pad {
    /// Nothing pressed, sticks centred.
    fn default() -> Pad {
        Pad {
            held: 0,
            lx: 0x80,
            ly: 0x80,
            ry: 0x80,
        }
    }
}

impl Pad {
    /// What the driver gets from the pad, `previous_held` being the buttons of the tick before.
    pub fn input(&self, previous_held: u32) -> Input {
        Input {
            buttons_held: self.held,
            buttons_tapped: self.held & !previous_held,
            stick_lx: self.lx,
            stick_ly: self.ly,
            stick_ry: self.ry,
        }
    }

    /// The pad an input came from.
    pub fn of_input(input: &Input) -> Pad {
        Pad {
            held: input.buttons_held,
            lx: input.stick_lx,
            ly: input.stick_ly,
            ry: input.stick_ry,
        }
    }
}

/// Who drives a kart of a race, and what the kart needs from its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KartSpec {
    /// In `GAME_CHARACTER_IDS` order.
    pub character: usize,
    /// The frames of the first eight animations of the kart's model (`Driver::animNumFrames`).
    pub anim_num_frames: [u16; 8],
}

/// A kart of a race: the driver, its collision state, and the buttons of its last tick.
#[derive(Clone, Debug)]
pub struct Kart {
    pub driver: Driver,
    pub coll: Collision,
    /// The buttons held in the last tick, for the taps.
    pub held: u32,
}

impl Kart {
    /// The kart of `spec` on the grid, in place `slot`, standing still.
    pub fn birth(level: &Level, slot: usize, spec: &KartSpec) -> Kart {
        let order = vehicle::grid::spawn_order(
            vehicle::grid::GridMode::Arcade,
            1,
            [0, 1, 2, 3, 4, 5, 6, 7],
        );
        let mut coll = Collision::default();
        let mut driver = Driver::new(slot as u8, spec.character);
        driver.anim_num_frames = spec.anim_num_frames;
        spawn(&mut driver, &mut coll, level, order[slot] as usize);
        Kart {
            driver,
            coll,
            held: 0,
        }
    }
}

/// What a tick did to a kart, for the outside to show or play. None of it is read by the next tick.
#[derive(Clone, Debug)]
pub struct KartTick {
    pub result: TickResult,
    /// The sounds and rumble the kart asked for.
    pub effects: Vec<Effect>,
    /// The exhaust puffs of the tick.
    pub puffs: Vec<Puff>,
    /// The sparks of a landing, and of the wall the kart scraped.
    pub landing: Option<Landing>,
    pub wall: Option<Wall>,
}

/// A race at some tick.
#[derive(Clone)]
pub struct Race {
    /// By slot. A kart bumps into the ones after it, so the order is part of the race.
    pub karts: Vec<Kart>,
    pub objects: Objects,
    /// What the objects did in the last tick: where the hazards look for drivers, and the sparks
    /// of the fireballs.
    pub last: instance::Tick,
    /// When the last bump was heard.
    pub feedback: Feedback,
    /// The ticks played: `gGT->timer`.
    pub tick: u32,
    /// Whether karts bump into each other (retail has it off online). A rule of the race: it has
    /// to be the same for everyone.
    pub kart_collisions: bool,
}

impl Race {
    /// The race at tick 0: the `specs` on the grid, kart `i` in place `i`, and `objects` as the
    /// level places them.
    pub fn new(level: &Level, objects: Objects, specs: &[KartSpec], kart_collisions: bool) -> Race {
        let karts = specs
            .iter()
            .enumerate()
            .map(|(slot, spec)| Kart::birth(level, slot, spec))
            .collect();
        Race {
            karts,
            objects,
            last: instance::Tick::default(),
            feedback: Feedback::default(),
            tick: 0,
            kart_collisions,
        }
    }

    /// One game tick: the objects, then every kart from its pad (`pads[slot]`), in the order of
    /// the slots, each stage over all of them. The result is by slot.
    pub fn tick(&mut self, level: &Level, pads: &[Pad]) -> Vec<KartTick> {
        assert_eq!(pads.len(), self.karts.len(), "one pad for each kart");
        // The objects tick with the karts and before them, so that the contacts they give are
        // this tick's.
        self.last = self.objects.tick();
        let env = Env {
            elapsed_event_time: self.tick as i32 * 32,
            timer: self.tick,
            kart_collisions: self.kart_collisions,
            ..Env::default()
        };
        let inputs: Vec<Input> = self
            .karts
            .iter_mut()
            .zip(pads)
            .map(|(kart, pad)| {
                let input = pad.input(kart.held);
                kart.held = pad.held;
                input
            })
            .collect();
        let mut effects: Vec<Vec<Effect>> = vec![Vec::new(); self.karts.len()];
        for ((kart, input), effects) in self.karts.iter_mut().zip(&inputs).zip(&mut effects) {
            before_tick(&mut kart.driver, input, &self.last, effects);
        }
        let results = {
            let mut racers: Vec<Racer> = self
                .karts
                .iter_mut()
                .zip(&inputs)
                .zip(&mut effects)
                .map(|((kart, input), effects)| Racer {
                    driver: &mut kart.driver,
                    coll: &mut kart.coll,
                    input,
                    effects,
                })
                .collect();
            vehicle::sim::tick_all(
                &mut racers,
                level,
                &env,
                &mut self.objects,
                &mut self.feedback,
            )
        };
        let ticks = self
            .karts
            .iter_mut()
            .zip(effects)
            .zip(results)
            .map(|((kart, mut effects), result)| {
                let (landing, wall, puffs) = after_tick(&mut kart.driver, &env, &mut effects);
                KartTick {
                    result,
                    effects,
                    puffs,
                    landing,
                    wall,
                }
            })
            .collect();
        self.tick += 1;
        ticks
    }
}

/// What a kart does before the stages of a tick: the same for every kart, so that all clients
/// play every kart the same way.
fn before_tick(
    driver: &mut Driver,
    input: &Input,
    objects: &instance::Tick,
    effects: &mut Vec<Effect>,
) {
    // M (or East on a pad) uses the mask item, once per press. (Not while a mask grab has the
    // kart: it has the mask already.)
    if input.buttons_tapped & vehicle::buttons::CIRCLE != 0
        && !matches!(
            driver.kart_state,
            kart_state::MASK_GRABBED | kart_state::ENGINE_REVVING
        )
    {
        vehicle::stuck::shoot_mask(driver, effects);
    }
    // The hazards that reached the kart's instance in their last tick hurt it
    // (`RB_CheckHazardCollisions`): the spin-out starts on this tick.
    let (_, at) = vehicle::baked::inst_matrix(driver);
    for contact in objects.contacts.iter().filter(|c| c.hits(at)) {
        let was_spinning = driver.kart_state == kart_state::SPINNING;
        if vehicle::pick::new_state(driver, contact.damage as u8, effects)
            && !was_spinning
            && contact.sound != 0
        {
            let echo = driver.actions_flag_set & actions::ENGINE_ECHO != 0;
            effects.push(Effect::Sound {
                id: contact.sound as u8,
                echo,
            });
        }
    }
}

/// What a kart does after the stages of a tick. The sparks and the puffs are for the outside to
/// draw.
fn after_tick(
    driver: &mut Driver,
    env: &Env,
    effects: &mut Vec<Effect>,
) -> (Option<Landing>, Option<Wall>, Vec<Puff>) {
    // The pickup HUD gives the fruit that were picked up to the driver.
    vehicle::wumpa::hud_tick(driver, true, effects);

    let landing = vehicle::sparks::landing(driver);
    let wall = vehicle::sparks::wall(driver);
    // The exhaust and the skid marks keep their state in the driver, so they run for every kart
    // even when nobody sees the particles.
    let puffs = vehicle::exhaust::exhaust(driver);
    vehicle::skid::skidmarks(driver);
    vehicle::turbo::tick(driver, env.elapsed_time_ms as i16);
    (landing, wall, puffs)
}

/// FNV-1a over the text written to it: a hash of a value's `Debug`, with no text kept.
struct Fnv(u64);

impl std::fmt::Write for Fnv {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        for byte in text.bytes() {
            self.0 = (self.0 ^ byte as u64).wrapping_mul(0x100_0000_01b3);
        }
        Ok(())
    }
}

impl Race {
    /// A hash of all the race holds, which two clients that played the same race agree on and
    /// ones that drifted apart do not. (It reads the `Debug` of the state, which is slow: it is
    /// for a check now and then, not every tick.)
    pub fn checksum(&self) -> u64 {
        use std::fmt::Write;
        let mut hash = Fnv(0xcbf2_9ce4_8422_2325);
        let _ = write!(
            hash,
            "{} {:?} {:?}\n",
            self.tick,
            self.feedback,
            self.objects.ticks()
        );
        for kart in &self.karts {
            let _ = write!(hash, "{:?} {:?} {}\n", kart.driver, kart.coll, kart.held);
        }
        let _ = write!(hash, "{:?}", self.last);
        hash.0
    }
}

impl rollback::Simulation for Race {
    type Context = Level;
    type Output = Vec<KartTick>;

    fn step(&mut self, level: &Level, pads: &[Pad]) -> Vec<KartTick> {
        self.tick(level, pads)
    }

    fn checksum(&self) -> u64 {
        Race::checksum(self)
    }
}
