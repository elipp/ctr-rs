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
use vehicle::lap::Lap;
use vehicle::sim::{Racer, TickResult, spawn};
use vehicle::sparks::{Landing, Wall};
use vehicle::{Driver, Effect, Env, Input, actions, kart_state};
use weapon::Weapons;

/// `driverRank` of a kart that has no place yet.
const UNSORTED: i16 = -1;

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
    /// The line the kart crossed, if it did.
    pub lap: Lap,
}

/// `trafficLightsTimer` at the start of a race, in milliseconds: it counts down 32 a tick.
pub const START_LIGHTS_MS: i32 = 0xf00;

/// The lights go out for good at this timer (`MainMain`'s floor).
const LIGHTS_FLOOR_MS: i32 = -960;

/// The ticks of the start-line fly-in (`CAM_StartOfRace`'s `transitionFrame`, 0xa5) and the one
/// that ends it. The traffic lights only start to count when it is over.
pub const INTRO_TICKS: u32 = 0xa6;

/// A step of the traffic lights (`DotLights_AudioAndVideo`): the light that came on, and the
/// sound of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Red1,
    Red2,
    Red3,
    Green,
}

/// When each light comes on: the timer goes to or under this.
const LIGHT_AT_MS: [(i32, Light); 4] = [
    (0xb40, Light::Red1),
    (0x780, Light::Red2),
    (0x3c0, Light::Red3),
    (0, Light::Green),
];

/// The laps of a race unless it is told otherwise (`gGT->numLaps`).
pub const DEFAULT_LAPS: u8 = 3;

/// A race at some tick.
#[derive(Clone)]
pub struct Race {
    /// By slot. A kart bumps into the ones after it, so the order is part of the race.
    pub karts: Vec<Kart>,
    pub objects: Objects,
    /// What the objects did in the last tick: where the hazards look for drivers, and the sparks
    /// of the fireballs.
    pub last: instance::Tick,
    /// The bombs and missiles in flight.
    pub weapons: Weapons,
    /// What they did in the last tick: the explosions, for the outside to show.
    pub last_weapons: weapon::Tick,
    /// When the last bump was heard.
    pub feedback: Feedback,
    /// The ticks played: `gGT->timer`.
    pub tick: u32,
    /// Whether karts bump into each other (retail has it off online). A rule of the race: it has
    /// to be the same for everyone.
    pub kart_collisions: bool,
    /// How many laps the race has (`gGT->numLaps`). A rule of the race, like the one above.
    pub laps: u8,
    /// How many karts have finished the race (`numPlayersFinishedRace`): the next to finish is
    /// that place.
    pub finished: i16,
    /// The time of each lap a kart has finished, by slot, in milliseconds (`gGT->lapTime`).
    pub lap_times: Vec<Vec<i32>>,
    /// `trafficLightsTimer`: above 0 the karts wait on the grid. It holds at the start while the
    /// fly-in plays, and then counts down. 0 in a race with no start ceremony.
    pub lights_ms: i32,
    /// How many ticks the fly-in takes: the lights wait for it.
    pub intro_ticks: u32,
    /// The light that came on in the last tick, if one did.
    pub light: Option<Light>,
    /// `elapsedEventTime`: the milliseconds since the green light (since tick 0 in a race with
    /// no ceremony). The lap times are of this clock.
    pub event_ms: i32,
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
            weapons: Weapons::default(),
            last_weapons: weapon::Tick::default(),
            feedback: Feedback::default(),
            tick: 0,
            kart_collisions,
            laps: DEFAULT_LAPS,
            finished: 0,
            lap_times: vec![Vec::new(); specs.len()],
            lights_ms: 0,
            intro_ticks: 0,
            light: None,
            event_ms: 0,
        }
    }

    /// Starts the race with the ceremony: the camera flies in (if the level has the fly-in), then
    /// the traffic lights count down, and the karts rev their engines on the grid until they go
    /// green. (Without it a race starts straight in driving.)
    pub fn start_ceremony(&mut self, level: &Level) {
        self.lights_ms = START_LIGHTS_MS;
        self.intro_ticks = if level.fly_in.is_some() {
            INTRO_TICKS
        } else {
            0
        };
        for kart in &mut self.karts {
            vehicle::stuck::rev_engine_init(&mut kart.driver);
        }
    }

    /// `PlayLevel_UpdateLapStats`: every kart's progress along the track and the finish line,
    /// then the places. The karts that finished have theirs, in the order they crossed (the
    /// slot order when it is the same tick); the others are sorted by lap and distance.
    fn update_laps(&mut self, level: &Level) -> Vec<Lap> {
        let event_time = self.event_ms;
        let mut next_place = 0;
        let laps: Vec<Lap> = self
            .karts
            .iter_mut()
            .map(|kart| {
                let d = &mut kart.driver;
                let lap = vehicle::lap::update(d, level, self.laps, event_time);
                if lap.finished {
                    d.driver_rank = self.finished;
                    self.finished += 1;
                }
                if d.actions_flag_set & actions::RACE_FINISHED == 0 {
                    d.driver_rank = UNSORTED;
                } else {
                    next_place = next_place.max(d.driver_rank as i32 + 1);
                }
                lap
            })
            .collect();
        for place in next_place..self.karts.len() as i32 {
            let mut best = None;
            let mut best_lap = -10;
            let mut nearest = 0x3fff_ffff;
            for (slot, kart) in self.karts.iter().enumerate() {
                let d = &kart.driver;
                if d.driver_rank != UNSORTED {
                    continue;
                }
                // A kart behind the start line has not begun its lap.
                let lap = d.lap_index as i32
                    - (d.actions_flag_set & actions::BEHIND_START_LINE != 0) as i32;
                let to_finish = d.distance_to_finish_curr as i32;
                if lap > best_lap || (lap == best_lap && to_finish < nearest) {
                    nearest = to_finish;
                    best_lap = lap;
                    best = Some(slot);
                }
            }
            if let Some(slot) = best {
                self.karts[slot].driver.driver_rank = place as i16;
            }
        }
        laps
    }

    /// One game tick: the objects, then every kart from its pad (`pads[slot]`), in the order of
    /// the slots, each stage over all of them. The result is by slot.
    pub fn tick(&mut self, level: &Level, pads: &[Pad]) -> Vec<KartTick> {
        assert_eq!(pads.len(), self.karts.len(), "one pad for each kart");
        // The objects tick with the karts and before them, so that the contacts they give are
        // this tick's.
        self.last = self.objects.tick();
        self.light = None;
        if self.tick >= self.intro_ticks && self.lights_ms > LIGHTS_FLOOR_MS {
            let before = self.lights_ms;
            self.lights_ms = (before - 32).max(LIGHTS_FLOOR_MS);
            self.light = LIGHT_AT_MS
                .iter()
                .find(|&&(at, _)| before > at && self.lights_ms <= at)
                .map(|&(_, light)| light);
        }
        let env = Env {
            traffic_lights_timer: self.lights_ms,
            elapsed_event_time: self.event_ms,
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
        // The karts that asked to fire last tick fire, before any of them moves.
        let bodies: Vec<weapon::Body> = self
            .karts
            .iter()
            .map(|k| weapon::Body::of(&k.driver))
            .collect();
        for (slot, ((kart, input), effects)) in self
            .karts
            .iter_mut()
            .zip(&inputs)
            .zip(&mut effects)
            .enumerate()
        {
            if let Some(launch) = vehicle::item::shoot_on_circle_press(&mut kart.driver, effects) {
                let kind = match launch {
                    vehicle::item::Launch::Bomb => weapon::Kind::Bomb,
                    vehicle::item::Launch::Missile => weapon::Kind::Missile,
                    vehicle::item::Launch::Mine => {
                        let gte = &mut kart.coll.gte;
                        self.weapons.drop_mine(
                            slot,
                            &kart.driver,
                            level,
                            &bodies,
                            gte,
                            env.elapsed_time_ms,
                        );
                        vehicle::item::launch_effects(&kart.driver, launch, effects);
                        continue;
                    }
                };
                let backward = input.buttons_held & vehicle::buttons::DOWN != 0;
                let gte = &mut kart.coll.gte;
                if self
                    .weapons
                    .launch(kind, slot, &kart.driver, &bodies, gte, backward)
                {
                    vehicle::item::launch_effects(&kart.driver, launch, effects);
                    kart.driver.bomb_thrown |= kind == weapon::Kind::Bomb;
                }
            }
        }
        for (kart, effects) in self.karts.iter_mut().zip(&mut effects) {
            before_tick(
                &mut kart.driver,
                &self.last,
                &mut self.objects.random_number,
                effects,
            );
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
        // The bombs and missiles fly, after the karts have moved. A bomb that its kart set off
        // goes with this tick's flight.
        for (slot, kart) in self.karts.iter_mut().enumerate() {
            if std::mem::take(&mut kart.driver.detonate_bomb) {
                self.weapons.detonate(slot);
            }
        }
        let bodies: Vec<weapon::Body> = self
            .karts
            .iter()
            .map(|k| weapon::Body::of(&k.driver))
            .collect();
        self.last_weapons = self.weapons.tick(
            level,
            &bodies,
            env.elapsed_time_ms,
            &mut self.objects.random_number,
        );
        for hit in &self.last_weapons.hits {
            let victim = &mut self.karts[hit.victim].driver;
            vehicle::pick::new_state(victim, hit.damage, &mut effects[hit.victim]);
        }
        for (slot, kart) in self.karts.iter_mut().enumerate() {
            kart.driver.bomb_thrown = self.weapons.has_bomb(slot);
        }
        // The weapon roulettes that crates started run on, and pick.
        for kart in &mut self.karts {
            vehicle::item::tick(&mut kart.driver, &mut self.objects.random_number);
        }
        let mut ticks: Vec<KartTick> = self
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
                    lap: Lap::default(),
                }
            })
            .collect();
        for ((slot, tick), lap) in ticks.iter_mut().enumerate().zip(self.update_laps(level)) {
            if lap.new_lap {
                self.lap_times[slot].push(lap.time);
            }
            tick.lap = lap;
        }
        if self.lights_ms < 1 {
            self.event_ms += env.elapsed_time_ms;
        }
        self.tick += 1;
        ticks
    }
}

/// What a kart does before the stages of a tick: the same for every kart, so that all clients
/// play every kart the same way.
fn before_tick(
    driver: &mut Driver,
    objects: &instance::Tick,
    random_number: &mut i32,
    effects: &mut Vec<Effect>,
) {
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
    // A kart that was blasted draws which way its tumble plays, as the init stage does.
    vehicle::tumble::roll_direction(driver, random_number);
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
        let _ = writeln!(
            hash,
            "{} {} {} {} {} {} {:?} {:?} {:?}",
            self.tick,
            self.event_ms,
            self.laps,
            self.finished,
            self.lights_ms,
            self.intro_ticks,
            self.lap_times,
            self.feedback,
            self.objects.ticks()
        );
        for kart in &self.karts {
            let _ = writeln!(hash, "{:?} {:?} {}", kart.driver, kart.coll, kart.held);
        }
        let _ = write!(
            hash,
            "{:?} {:?} {:?}",
            self.last, self.weapons, self.last_weapons
        );
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
