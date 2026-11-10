//! A race, all of it that the next tick depends on: the karts, the level's objects, and the
//! little the bumps keep between ticks. Plain state and a `tick` from everyone's pads, with no
//! engine and no I/O, so that every client plays the same race and a copy of it is a snapshot:
//! [`Race`] is `Clone`, and what a clone is put back to plays on exactly as the original did.
//!
//! What a tick asks of the outside (sounds, rumble, puffs and sparks to draw) comes out as
//! [`KartTick`]s; nothing of it goes back in.

pub mod bot;
pub mod desync;
pub mod infer;
pub mod nn;
pub mod rng;
pub mod rollback;

use coll::Collision;
use instance::Objects;
use loader::lev::Level;
use vehicle::collide::Feedback;
use vehicle::exhaust::Puff;
use vehicle::lap::Lap;
use vehicle::sim::{Racer, TickResult, spawn};
use vehicle::sparks::{Landing, Spray, Wall};
use vehicle::{Driver, Effect, Env, Input, actions, kart_state};
use weapon::Weapons;

/// `driverRank` of a kart that has no place yet.
/// The version of the sim's behavior. Clients that play a race together must agree on it: any
/// change that makes a tick come out differently (physics, weapons, objects, the rules) bumps it,
/// and the lobby turns away a client with another. (The desync checks catch the ones that forget.)
pub const SIM_VERSION: u32 = 11;

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
    /// The race drives the kart itself ([`bot::Bot`]), and takes no pad for it.
    pub bot: bool,
    /// The bot path (into `Level::nav_paths`) a bot drives, instead of the one of its slot
    /// ([`bot::Bot::new`]).
    pub line: Option<u8>,
}

/// The rubber band of the learned bots ([`Race::band`]): karts whose stats the race sets each
/// tick by how far they are from the best human, slower ahead of them and faster behind. The
/// policy only makes a pad, so the band acts on the kart instead (retail's bots, which move
/// along their path at the speed they are given, have theirs: `vehicle::bot`).
///
/// The range is absolute, the same for every character: at a gap of
/// [`BAND_FULL_GAP_LAP_DIV`] or more, a kart that leads has the worst any engine class has, and
/// one that trails the best any has, times the difficulty's boost; at no gap, its class's own,
/// and between, blended. So a Turn kart that trails on hard has the Speed class's top speed
/// × 1.08, and the Accel class's acceleration × 1.08.
///
/// The top speed is `speed_class_stat` and `accel_speed_class_stat` (the wumpas' bonus is the
/// gap between them). Trailing, the acceleration (`accel_class_stat`) and turning (`turn_rate`,
/// `turn_input_delay`, the rate the turning builds up at, and `drift_turn_base`) go the same
/// way; leading, they stay the class's own (holding back is the top speed's to do). The policy
/// sees its speed over `speed_class_stat` (`nn::observe`), so a banded kart still looks to be at
/// its top speed, but it learned the corners at its class's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    /// The karts it acts on, a bit by slot.
    pub slots: u8,
    /// `vehicle::bot::difficulty_level`: how hard the bots race, which sets how far past the
    /// best class a kart that trails gets.
    pub difficulty: i16,
}

/// The band's pull and boost are out of this.
const BAND_ONE: i64 = 0x1000;

/// At a gap of this fraction of a lap (or more), the band pulls its hardest.
const BAND_FULL_GAP_LAP_DIV: i64 = 6;

/// How far past the best class a kart that trails gets at most, out of [`BAND_ONE`]: at
/// difficulty 0 and at [`BAND_DIFFICULTY_MAX`], blended between (easy 4%, medium 6%, hard 8%).
const BAND_BOOST: [i64; 2] = [0x52, 0x148]; // 2% .. 8%
const BAND_DIFFICULTY_MAX: i64 = 0xf0;

/// Every engine class, for the band's range.
const ENGINE_CLASSES: [vehicle::stats::EngineClass; 4] = {
    use vehicle::stats::EngineClass::*;
    [Balanced, Accel, Speed, Turn]
};

impl Band {
    /// A band on the karts of `slots`.
    pub fn new(slots: impl IntoIterator<Item = usize>, difficulty: i16) -> Band {
        let slots = slots
            .into_iter()
            .filter(|&s| s < MAX_KARTS)
            .fold(0, |m, s| m | 1 << s);
        Band { slots, difficulty }
    }

    pub fn has(&self, slot: usize) -> bool {
        slot < MAX_KARTS && self.slots & 1 << slot != 0
    }

    /// How hard the band pulls a kart `gap` behind the best human (ahead of them below 0), on a
    /// track `track` long: from `-BAND_ONE` (leading by the full gap) to `BAND_ONE` (trailing
    /// by it).
    fn pull(gap: i64, track: i64) -> i64 {
        let full = track / BAND_FULL_GAP_LAP_DIV;
        if full <= 0 {
            return 0;
        }
        (gap * BAND_ONE / full).clamp(-BAND_ONE, BAND_ONE)
    }

    /// How far past the best class a kart that trails by the full gap gets, out of [`BAND_ONE`].
    fn boost(&self) -> i64 {
        let d = (self.difficulty as i64).clamp(0, BAND_DIFFICULTY_MAX);
        BAND_BOOST[0] + (BAND_BOOST[1] - BAND_BOOST[0]) * d / BAND_DIFFICULTY_MAX
    }

    /// A stat of the class's `own` value, pulled by `pull`: toward the best of the classes
    /// times the boost when it trails, toward the worst when it leads (if the stat holds back a
    /// leader, `leading`). `stat` reads it from a class's stats.
    fn stat(
        &self,
        own: &vehicle::stats::Stats,
        stat: impl Fn(&vehicle::stats::Stats) -> i64,
        pull: i64,
        leading: bool,
    ) -> i64 {
        let values = ENGINE_CLASSES.map(|c| stat(&vehicle::stats::Stats::for_class(c)));
        let own = stat(own);
        let end = if pull > 0 {
            let best = values.into_iter().max().unwrap_or(own);
            best * (BAND_ONE + self.boost()) / BAND_ONE
        } else if leading {
            values.into_iter().min().unwrap_or(own)
        } else {
            own
        };
        own + (end - own) * pull.abs() / BAND_ONE
    }
}

/// A kart of a race: the driver, its collision state, and the buttons of its last tick.
#[derive(Clone, Debug)]
pub struct Kart {
    pub driver: Driver,
    pub coll: Collision,
    /// The buttons held in the last tick, for the taps.
    pub held: u32,
    /// What drives the kart, if the race does.
    pub bot: Option<bot::Bot>,
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
        let bot = spec
            .bot
            .then(|| bot::Bot::new(level, slot, &driver, spec.line));
        Kart {
            driver,
            coll,
            held: 0,
            bot,
        }
    }
}

impl Kart {
    /// The kart of `spec` at frame `frame` of bot path `path`, facing the next frame turned by
    /// `turn` (as `Driver::angle`, 0x1000 a turn), standing still: anywhere on the track rather
    /// than the grid (to train a bot on every part of it, and on getting out of where it faces a
    /// wall). None if the level has no such frame, or it is over a kill plane (a gap).
    pub fn birth_on_path(
        level: &Level,
        slot: usize,
        spec: &KartSpec,
        path: usize,
        frame: usize,
        turn: i32,
    ) -> Option<Kart> {
        let frames = &level.nav_paths.get(path)?.frames;
        let here = frames.get(frame)?;
        if frames.len() < 2 || here.flags & loader::lev::NavFrame::KILL_PLANE != 0 {
            return None;
        }
        let next = &frames[(frame + 1) % frames.len()];
        let angle = (math::ratan2(
            next.pos.x as i32 - here.pos.x as i32,
            next.pos.z as i32 - here.pos.z as i32,
        ) + turn)
            & 0xfff;
        let mut coll = Collision::default();
        let mut driver = Driver::new(slot as u8, spec.character);
        driver.anim_num_frames = spec.anim_num_frames;
        vehicle::sim::spawn_at(&mut driver, &mut coll, level, here.pos, angle as i16);
        let bot = spec
            .bot
            .then(|| bot::Bot::new(level, slot, &driver, spec.line));
        Some(Kart {
            driver,
            coll,
            held: 0,
            bot,
        })
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
    /// What dirt, grass or snow under the kart throws.
    pub spray: Option<Spray>,
    /// The line the kart crossed, if it did.
    pub lap: Lap,
}

/// `trafficLightsTimer` at the start of a race, in milliseconds: it counts down 32 a tick.
pub const START_LIGHTS_MS: i32 = 0xf00;

/// The lights go out for good at this timer (`MainMain`'s floor).
pub const LIGHTS_FLOOR_MS: i32 = -960;

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

/// How many of the lights are on at `lights_ms` (`trafficLightsTimer`), the three reds first
/// and then the green.
pub fn lights_on(lights_ms: i32) -> usize {
    LIGHT_AT_MS
        .iter()
        .filter(|&&(at, _)| lights_ms <= at)
        .count()
}

/// The game's racing tracks are the `LevelId`s below this (Dingo Canyon to Turbo Track).
pub const RACING_TRACKS: u32 = 18;

/// The most karts a race has, the bots' included.
pub const MAX_KARTS: usize = 8;

/// The laps of a race unless it is told otherwise (`gGT->numLaps`).
pub const DEFAULT_LAPS: u8 = 3;

/// How many checkpoints a lap is cut by (see [`Race::splits`]).
pub const CHECKPOINTS_PER_LAP: usize = 2;

/// The laps of a free drive, which has no end: more than anyone drives.
pub const FREE_DRIVE_LAPS: u8 = u8::MAX;

/// Where the weapons fired this tick go: the level, the karts as they were before any of them
/// fired, the temple doors, and the tick's milliseconds.
struct Aim<'a> {
    level: &'a Level,
    bodies: &'a [weapon::Body],
    doors: &'a coll::doors::Doors,
    elapsed_time_ms: i32,
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
    /// The bombs, missiles, warpballs and mines in play.
    pub weapons: Weapons,
    /// What they did in the last tick: the explosions, for the outside to show.
    pub last_weapons: weapon::Tick,
    /// How each kart's screen was drawn over for a clock in the last tick, by slot
    /// (`DISPLAY_Blur_Main`'s strength, see `vehicle::item::clock_frame`): white below 0, a trail
    /// above, nothing at 0.
    pub last_blurs: Vec<i32>,
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
    /// The tick each kart that has finished crossed the line at, by place (entry 0 is the
    /// winner's). A race that waits for the first few to finish is over once that tick is
    /// settled.
    pub finish_ticks: Vec<u32>,
    /// The first kart over the line of the last lap, as (slot, tick): the race has been won.
    /// None until then, and always in a free drive.
    pub winner: Option<(usize, u32)>,
    /// The time of each lap a kart has finished, by slot, in milliseconds (`gGT->lapTime`).
    pub lap_times: Vec<Vec<i32>>,
    /// How many of the checkpoints (two to a lap: the middle of it and the line) each kart has
    /// passed; 0 until it is on its way. The races of OnlineCTR show the times of these.
    pub checkpoints: Vec<u16>,
    /// The time each kart passed each checkpoint at, by slot, in milliseconds of the race clock:
    /// entry `i` is checkpoint `i + 1`.
    pub splits: Vec<Vec<i32>>,
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
    /// What the bots share, if the race has any. Their karts come after the players'.
    pub bots: Option<vehicle::bot::Bots>,
    /// The rubber band of the learned bots, which are players to the race. A rule of the race,
    /// like the laps.
    pub band: Option<Band>,
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
            last_blurs: Vec::new(),
            feedback: Feedback::default(),
            tick: 0,
            kart_collisions,
            laps: DEFAULT_LAPS,
            finished: 0,
            finish_ticks: Vec::new(),
            winner: None,
            lap_times: vec![Vec::new(); specs.len()],
            checkpoints: vec![0; specs.len()],
            splits: vec![Vec::new(); specs.len()],
            lights_ms: 0,
            intro_ticks: 0,
            light: None,
            event_ms: 0,
            bots: None,
            band: None,
        }
    }

    /// Puts the kart in `slot` (of `spec`) at frame `frame` of bot path `path`, standing still
    /// ([`Kart::birth_on_path`]). Returns whether it could: not on a frame over a gap.
    pub fn place_on_path(
        &mut self,
        level: &Level,
        slot: usize,
        spec: &KartSpec,
        path: usize,
        frame: usize,
        turn: i32,
    ) -> bool {
        match Kart::birth_on_path(level, slot, spec, path, frame, turn) {
            Some(kart) => {
                self.karts[slot] = kart;
                true
            }
            None => false,
        }
    }

    /// Fills the grid after the players with a bot of each of `specs` (`BOTS_Driver_Init`), in
    /// the slots that follow theirs. A bot the level has no path for is left out.
    pub fn add_bots(&mut self, level: &Level, setup: vehicle::bot::Setup, specs: &[KartSpec]) {
        let bots = self
            .bots
            .get_or_insert_with(|| vehicle::bot::Bots::new(setup));
        for spec in specs {
            let slot = self.karts.len();
            if slot >= MAX_KARTS {
                break;
            }
            let mut coll = Collision::default();
            let mut driver = Driver::new(slot as u8, spec.character);
            driver.anim_num_frames = spec.anim_num_frames;
            if !bots.spawn(&mut driver, &mut coll, level) {
                continue;
            }
            self.karts.push(Kart {
                driver,
                coll,
                held: 0,
                bot: None,
            });
            self.lap_times.push(Vec::new());
            self.checkpoints.push(0);
            self.splits.push(Vec::new());
        }
    }

    /// The karts the players drive: the ones before the bots'.
    pub fn players(&self) -> usize {
        self.karts
            .iter()
            .take_while(|k| k.driver.bot.is_none())
            .count()
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
        for kart in self.karts.iter_mut().filter(|k| k.driver.bot.is_none()) {
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
                    self.finish_ticks.push(self.tick);
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

    /// OnlineCTR's checkpoint tracker (`UpdateCheckpointTracker`): the lap is cut in two, and the
    /// time each kart passes each cut at is kept, to show against the first to pass it. A kart
    /// is in the middle of a lap when its distance to the finish (the one the lap counter keeps)
    /// is under half the track, and at the line when it is back above half.
    fn update_splits(&mut self, level: &Level) {
        let Some(start) = level.restart_points.first() else {
            return;
        };
        let track_length = (start.dist_to_finish as i32) << 3;
        let zones = [track_length, track_length / CHECKPOINTS_PER_LAP as i32, 0];
        for (slot, kart) in self.karts.iter().enumerate() {
            let progress = kart.driver.distance_to_finish_checkpoint as i32;
            let passed = self.checkpoints[slot] as usize;
            if passed == 0 {
                if progress < track_length {
                    self.checkpoints[slot] = 1;
                }
                continue;
            }
            let zone = passed % CHECKPOINTS_PER_LAP;
            // The kart does not pass more than the race has.
            if progress < zones[zone]
                && progress > zones[zone + 1]
                && passed <= self.laps as usize * CHECKPOINTS_PER_LAP
            {
                self.splits[slot].push(self.event_ms);
                self.checkpoints[slot] += 1;
            }
        }
    }

    /// What the kart in `slot` fired this tick (`VehPickupItem_ShootNow`'s part beyond the
    /// driver): a weapon flies or is dropped, a clock reaches every driver, the ones after it in
    /// this tick's order too. `effects` are by slot.
    fn fire(
        &mut self,
        slot: usize,
        fired: vehicle::item::Fired,
        input: &Input,
        aim: &Aim,
        effects: &mut [Vec<Effect>],
    ) {
        use vehicle::item::{Fired, Launch};
        let launch = match fired {
            Fired::Clock => {
                let mut drivers: Vec<&mut Driver> =
                    self.karts.iter_mut().map(|k| &mut k.driver).collect();
                vehicle::item::clock(slot, &mut drivers, effects);
                return;
            }
            Fired::Weapon(launch) => launch,
        };
        let (kart, effects) = (&mut self.karts[slot], &mut effects[slot]);
        let p = kart.driver.pos_curr;
        let kart_at = [p.x >> 8, p.y >> 8, p.z >> 8];
        let kind = match launch {
            Launch::Bomb => weapon::Kind::Bomb,
            Launch::Missile => weapon::Kind::Missile,
            Launch::Warpball => {
                self.weapons
                    .launch_warpball(slot, &kart.driver, aim.bodies, aim.level);
                let at = self.weapons.warpballs().last().map_or(kart_at, |w| w.pos);
                vehicle::item::launch_effects(&kart.driver, launch, at, effects);
                return;
            }
            Launch::Mine | Launch::Potion => {
                // A potion goes forward with Up held, backward with Down held (Up wins), and is
                // dropped otherwise.
                let held = input.buttons_held;
                let potion = (launch == Launch::Potion).then_some(if held & vehicle::buttons::UP != 0 {
                    weapon::mine::Throw::Forward
                } else if held & vehicle::buttons::DOWN != 0 {
                    weapon::mine::Throw::Backward
                } else {
                    weapon::mine::Throw::Drop
                });
                self.weapons.drop_mine(
                    weapon::mine::Release {
                        shooter: slot,
                        driver: &kart.driver,
                        gte: &mut kart.coll.gte,
                        potion,
                        doors: aim.doors,
                    },
                    aim.level,
                    aim.bodies,
                    aim.elapsed_time_ms,
                );
                // Heard from the crate or beaker, or the kart if it went off at once.
                let at = self.weapons.mines().last().map_or(kart_at, |m| m.pos);
                vehicle::item::launch_effects(&kart.driver, launch, at, effects);
                return;
            }
        };
        let backward = input.buttons_held & vehicle::buttons::DOWN != 0;
        let gte = &mut kart.coll.gte;
        if self
            .weapons
            .launch(kind, slot, &kart.driver, aim.bodies, gte, backward)
        {
            let at = self.weapons.projectiles().last().map_or(kart_at, |p| p.pos);
            vehicle::item::launch_effects(&kart.driver, launch, at, effects);
            kart.driver.bomb_thrown |= kind == weapon::Kind::Bomb;
        }
    }

    /// The checkpoint `kart` passed last, as (its number, the time it passed it at, and how
    /// far behind the first kart to pass it that is: 0 if it was the first). None before the
    /// first checkpoint.
    pub fn last_split(&self, slot: usize) -> Option<(usize, i32, i32)> {
        let time = *self.splits[slot].last()?;
        let number = self.splits[slot].len();
        let first = self
            .splits
            .iter()
            .filter_map(|times| times.get(number - 1))
            .min()
            .copied()
            .unwrap_or(time);
        Some((number, time, time - first))
    }

    /// The rubber band ([`Race::band`]): each of its karts gets its class's top speed, scaled by
    /// how far it is from the best human, measured as retail's bots measure it (the laps, less
    /// one behind the start line, and the distance to the finish). Unscaled with no human to
    /// chase, before the green light, and once the kart has finished.
    fn pull_band(&mut self, level: &Level, racing: bool) {
        let Some(band) = self.band else {
            return;
        };
        let track = level
            .restart_points
            .first()
            .map_or(0, |p| (p.dist_to_finish as i64) << 3);
        let progress = |d: &Driver| {
            let lap =
                d.lap_index as i64 - (d.actions_flag_set & actions::BEHIND_START_LINE != 0) as i64;
            lap * track + track - d.distance_to_finish_curr as i64
        };
        let best = self
            .karts
            .iter()
            .enumerate()
            .filter(|&(slot, k)| !band.has(slot) && k.bot.is_none() && k.driver.bot.is_none())
            .map(|(_, k)| progress(&k.driver))
            .max();
        for (slot, kart) in self.karts.iter_mut().enumerate() {
            if !band.has(slot) {
                continue;
            }
            let d = &mut kart.driver;
            let pull = match best {
                Some(best) if racing && d.actions_flag_set & actions::RACE_FINISHED == 0 => {
                    Band::pull(best - progress(d), track)
                }
                _ => 0,
            };
            let class = vehicle::stats::Stats::for_class(
                vehicle::stats::CHARACTER_ENGINES[d.character as usize],
            );
            let s = &mut d.stats;
            s.speed_class_stat = band.stat(&class, |c| c.speed_class_stat as i64, pull, true) as i16;
            s.accel_speed_class_stat =
                band.stat(&class, |c| c.accel_speed_class_stat as i64, pull, true) as i16;
            s.accel_class_stat = band.stat(&class, |c| c.accel_class_stat as i64, pull, false) as i16;
            s.turn_rate = band.stat(&class, |c| c.turn_rate as i64, pull, false) as u8;
            s.turn_input_delay = band.stat(&class, |c| c.turn_input_delay as i64, pull, false) as i16;
            s.drift_turn_base = band.stat(&class, |c| c.drift_turn_base as i64, pull, false) as i8;
        }
    }

    /// One game tick: the objects, then every player's kart from its pad (`pads[slot]`), in the
    /// order of the slots, each stage over all of them, then the bots. The result is by slot.
    pub fn tick(&mut self, level: &Level, pads: &[Pad]) -> Vec<KartTick> {
        let players = self.players();
        assert_eq!(pads.len(), players, "one pad for each player");
        // The clocks' timers on the screens, before anything ticks.
        self.last_blurs = self
            .karts
            .iter_mut()
            .map(|k| vehicle::item::clock_frame(&mut k.driver))
            .collect();
        // The objects tick with the karts and before them, so that the contacts they give are
        // this tick's.
        let blockers: Vec<instance::Blocker> = self
            .karts
            .iter()
            .map(|k| instance::Blocker {
                model: coll::model_id::PLAYER,
                at: weapon::Body::of(&k.driver).at,
            })
            .chain(
                self.weapons
                    .mines()
                    .iter()
                    .filter(|m| !m.is_dead())
                    .map(|m| instance::Blocker {
                        model: m.kind.model(),
                        at: m.pos,
                    }),
            )
            .collect();
        self.last = self.objects.tick(&blockers);
        // The doors as the karts and the weapons find them this tick.
        let doors = self.objects.doors();
        for kart in &mut self.karts {
            kart.coll.door_access_flags = doors.access;
        }
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
        // The bots wait for the green light before they count as stuck.
        let racing = self.lights_ms <= 0;
        self.pull_band(level, racing);
        // A kart that has crossed the line takes no more of its pad: it coasts. So nobody needs
        // a finished player's pads to play a tick right, and it can watch the others as a
        // spectator does (see the client's `drive_tick`).
        let inputs: Vec<Input> = self.karts[..players]
            .iter_mut()
            .zip(pads)
            .map(|(kart, pad)| {
                // A bot's pad is its own: the one it was given (nobody sends any) is neutral.
                let pad = match &mut kart.bot {
                    Some(bot) => bot.pad(level, &kart.driver, racing),
                    None => *pad,
                };
                let pad = if kart.driver.actions_flag_set & actions::RACE_FINISHED != 0 {
                    Pad::default()
                } else {
                    pad
                };
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
        let aim = Aim {
            level,
            bodies: &bodies,
            doors: &doors,
            elapsed_time_ms: env.elapsed_time_ms,
        };
        for slot in 0..players {
            let fired =
                vehicle::item::shoot_on_circle_press(&mut self.karts[slot].driver, &mut effects[slot]);
            if let Some(fired) = fired {
                self.fire(slot, fired, &inputs[slot], &aim, &mut effects);
            }
        }
        // The objects that hurt a driver go on from there: a plant that ate one chews.
        let mut hurt = Vec::new();
        for (kart, effects) in self.karts[..players].iter_mut().zip(&mut effects) {
            let first = hurt.len();
            before_tick(&mut kart.driver, &self.last, &mut hurt, effects);
            // The plant that ate the kart is the one its camera looks at.
            if let Some(&plant) = hurt.get(first) {
                kart.driver.plant_eating_me = self.objects.plant_eater(plant);
            }
        }
        let eaten_bots = hurt_bots(&mut self.karts, players, &self.last, &hurt, &mut effects);
        for instance in hurt {
            self.objects.caught(instance, &mut self.last);
        }
        for instance in eaten_bots {
            self.objects.caught_bot(instance, &mut self.last);
        }
        // Then the mines in their reach, the ones dropped this tick too: the hazards' threads run
        // after the karts' in retail.
        let karts: Vec<[i32; 3]> = self.karts.iter().map(|k| k.driver.inst_pos).collect();
        set_off_mines(&self.last.contacts, &karts, &mut self.weapons);
        let mut results = {
            let (humans, bots) = self.karts.split_at_mut(players);
            // The `ROBOT` bucket: the last born first.
            let mut bots: Vec<&mut Driver> = bots.iter_mut().rev().map(|k| &mut k.driver).collect();
            let mut racers: Vec<Racer> = humans
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
            vehicle::sim::start_tick(&mut racers, &mut self.objects.random_number);
            vehicle::sim::tick_all(
                &mut racers,
                &mut bots,
                level,
                &env,
                &mut self.objects,
                &mut self.feedback,
            )
        };
        // The bots in the air fly toward where they should come down (a cheat, as retail's).
        if racing {
            for kart in &mut self.karts {
                if let Some(bot) = &kart.bot {
                    bot.fly(level, &mut kart.driver);
                }
            }
        }
        // Then the retail bots, after the karts that run the physics.
        results.resize(self.karts.len(), Default::default());
        if let Some(bots) = &mut self.bots {
            let mut karts: Vec<vehicle::bot::Kart> = self
                .karts
                .iter_mut()
                .zip(&mut effects)
                .map(|(kart, effects)| vehicle::bot::Kart {
                    driver: &mut kart.driver,
                    coll: &mut kart.coll,
                    effects,
                })
                .collect();
            let bot_env = vehicle::bot::Env {
                elapsed_time_ms: env.elapsed_time_ms,
                traffic_lights_timer: self.lights_ms,
                laps: self.laps,
                timer: self.tick,
                kart_collisions: self.kart_collisions,
            };
            vehicle::bot::tick_all(&mut karts, bots, level, &bot_env, &mut self.objects);
        }
        // The bombs and missiles fly, after the karts have moved. A bomb that its kart set off
        // goes with this tick's flight.
        for (slot, kart) in self.karts.iter_mut().enumerate() {
            if let Some(blue) = kart.driver.shield_shot.take() {
                self.weapons.launch_shield(slot, &kart.driver, blue);
            }
            if std::mem::take(&mut kart.driver.detonate_bomb) {
                self.weapons.detonate(slot);
            }
        }
        // The rank icons' tint runs down to nothing (the HUD does it as it draws, once a tick).
        for kart in &mut self.karts {
            let timer = &mut kart.driver.damage_color_timer;
            *timer -= timer.signum();
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
            &doors,
        );
        for &(slot, timer) in &self.last_weapons.damage_colors {
            self.karts[slot].driver.damage_color_timer = timer;
        }
        for &instance in &self.last_weapons.opened_doors {
            self.objects.open_door(instance);
        }
        for hit in &self.last_weapons.crate_hits {
            self.objects
                .hit(hit.instance, &mut self.karts[hit.owner].driver, hit.model);
        }
        for hit in &self.last_weapons.hits {
            let by_player = hit
                .attacker
                .map(|slot| self.karts[slot].driver.bot.is_none());
            let victim = &mut self.karts[hit.victim].driver;
            let effects = &mut effects[hit.victim];
            let hurt = if victim.bot.is_some() {
                vehicle::bot::hurt(victim, hit.damage, by_player, effects)
            } else {
                vehicle::pick::new_state(victim, hit.damage, effects)
            };
            // A red beaker that hurt the kart makes it rain on it.
            if hurt && hit.rain {
                vehicle::cloud::init(victim, self.tick, &mut self.objects.random_number);
            }
        }
        // The exhaust fires (after the weapons' buckets) and the HUD's jump meters.
        for (kart, effects) in self.karts.iter_mut().zip(&mut effects) {
            if kart.driver.bot.is_some() {
                // A bot has the exhaust fire of its boosts, and no HUD.
                vehicle::turbo::tick(&mut kart.driver, env.elapsed_time_ms as i16, effects);
            } else {
                vehicle::sim::after_logic(&mut kart.driver, &env, effects);
            }
        }
        let racers = self.karts.len();
        for kart in &mut self.karts {
            vehicle::cloud::tick(
                &mut kart.driver,
                racers,
                env.elapsed_time_ms,
                &mut self.objects.random_number,
                &mut self.weapons.warpball_held,
            );
        }
        for (slot, kart) in self.karts.iter_mut().enumerate() {
            kart.driver.bomb_thrown = self.weapons.has_bomb(slot);
            kart.driver.tnt_scale_y = self.weapons.tnt_scale_y(slot);
        }
        // The weapon roulettes that crates started run on, and pick.
        for kart in &mut self.karts[..players] {
            vehicle::item::tick(
                &mut kart.driver,
                racers,
                &mut self.objects.random_number,
                &mut self.weapons.warpball_held,
            );
        }
        let mut ticks: Vec<KartTick> = self
            .karts
            .iter_mut()
            .zip(effects)
            .zip(results)
            .map(|((kart, mut effects), result)| {
                let (landing, wall, spray, puffs) =
                    after_tick(&mut kart.driver, &env, &mut effects);
                KartTick {
                    result,
                    effects,
                    puffs,
                    landing,
                    wall,
                    spray,
                    lap: Lap::default(),
                }
            })
            .collect();
        if let Some(bots) = &mut self.bots {
            let drivers: Vec<&Driver> = self.karts.iter().map(|k| &k.driver).collect();
            bots.update_globals(&drivers);
        }
        self.bot_weapons(level, &doors, players, &mut ticks);
        for ((slot, tick), lap) in ticks.iter_mut().enumerate().zip(self.update_laps(level)) {
            if lap.new_lap {
                self.lap_times[slot].push(lap.time);
            }
            // The first to finish is place 0 (the slot order decides a tie).
            if lap.finished && self.winner.is_none() && self.karts[slot].driver.driver_rank == 0 {
                self.winner = Some((slot, self.tick));
            }
            tick.lap = lap;
        }
        self.update_splits(level);
        if self.lights_ms < 1 {
            self.event_ms += env.elapsed_time_ms;
        }
        self.tick += 1;
        ticks
    }
}

/// The hazards that look for mines set off the first in their reach (`RB_Hazard_ThCollide_Generic`),
/// most of them only when no kart (at `karts`) is in reach, hurt or not. A mine's thread goes to
/// the front of its bucket, so the newest is found first. One set off this tick is still there
/// and stops the search, as a dead thread stays in its bucket until the frame ends.
fn set_off_mines(
    contacts: &[instance::Contact],
    karts: &[[i32; 3]],
    weapons: &mut weapon::Weapons,
) {
    for contact in contacts {
        let look = match contact.mines {
            instance::Mines::Ignored => false,
            instance::Mines::IfNoDriver => !karts.iter().any(|&at| contact.hits(at)),
            instance::Mines::Only => true,
        };
        if !look {
            continue;
        }
        if let Some(index) = weapons.mines().iter().rposition(|m| contact.hits(m.pos)) {
            weapons.set_off_mine(index);
        }
    }
}

/// What a kart does before the stages of a tick: the same for every kart, so that all clients
/// play every kart the same way.
fn before_tick(
    driver: &mut Driver,
    objects: &instance::Tick,
    hurt: &mut Vec<usize>,
    effects: &mut Vec<Effect>,
) {
    // The hazards that reached the kart's instance in their last tick hurt it
    // (`RB_CheckHazardCollisions`): the spin-out starts on this tick.
    let at = driver.inst_pos;
    let drivers = objects
        .contacts
        .iter()
        .filter(|c| c.mines != instance::Mines::Only);
    for contact in drivers.filter(|c| c.hits(at)) {
        // A plant has one mouthful: the first kart it hurts is the one it eats.
        if contact.damage == instance::Damage::Eat && hurt.contains(&contact.instance) {
            continue;
        }
        let was_spinning = driver.kart_state == kart_state::SPINNING;
        if !vehicle::pick::new_state(driver, contact.damage as u8, effects) {
            continue;
        }
        if contact.damage == instance::Damage::Eat {
            hurt.push(contact.instance);
        }
        if !was_spinning && contact.sound != 0 {
            let echo = driver.actions_flag_set & actions::ENGINE_ECHO != 0;
            effects.push(Effect::Sound {
                id: contact.sound as u8,
                echo,
            });
        }
        if !was_spinning && contact.voice {
            effects.push(Effect::Voiceline {
                id: vehicle::pick::VOICELINE_COMMON_DAMAGE,
                driver_id: driver.driver_id,
                priority: vehicle::pick::VOICELINE_PRIORITY,
            });
        }
    }
}

/// The hazards that reached no player's kart in their last tick hurt the first bot they reach
/// instead (`RB_CheckHazardCollisions` looks in the `PLAYER` bucket, then in `ROBOT`, whose
/// newest thread is the last bot). The result is the plants that have a bot in their mouth, which
/// a plant that ate a player (in `hurt`) has not.
fn hurt_bots(
    karts: &mut [Kart],
    players: usize,
    objects: &instance::Tick,
    hurt: &[usize],
    effects: &mut [Vec<Effect>],
) -> Vec<usize> {
    let mut eaten = Vec::new();
    let drivers = objects
        .contacts
        .iter()
        .filter(|c| c.mines != instance::Mines::Only);
    for contact in drivers {
        if karts[..players]
            .iter()
            .any(|k| contact.hits(k.driver.inst_pos))
        {
            continue;
        }
        let eats = contact.damage == instance::Damage::Eat;
        if eats && (hurt.contains(&contact.instance) || eaten.contains(&contact.instance)) {
            continue;
        }
        let Some(slot) = (players..karts.len())
            .rev()
            .find(|&slot| contact.hits(karts[slot].driver.inst_pos))
        else {
            continue;
        };
        let d = &mut karts[slot].driver;
        vehicle::bot::hurt(d, contact.damage as u8, None, &mut effects[slot]);
        if eats {
            eaten.push(contact.instance);
        }
    }
    eaten
}

/// What a kart does after the stages of a tick. The sparks and the puffs are for the outside to
/// draw.
fn after_tick(
    driver: &mut Driver,
    env: &Env,
    effects: &mut Vec<Effect>,
) -> (Option<Landing>, Option<Wall>, Option<Spray>, Vec<Puff>) {
    // The pickup HUD gives the fruit that were picked up to the driver.
    vehicle::wumpa::hud_tick(driver, true, effects);

    // A kart that a plant ate has no particle procedure (`DRIVER_FUNC_PARTICLES`, which is
    // `VehEmitter_DriverMain` otherwise): no sparks, smoke, skid marks or ground sounds.
    let (mut landing, mut wall, mut spray, mut puffs) = (None, None, None, Vec::new());
    driver.mud_particles = 0;
    if driver.phys_state != vehicle::PhysState::PlantEaten {
        landing = vehicle::sparks::landing(driver);
        wall = vehicle::sparks::wall(driver);
        spray = vehicle::sparks::spray(driver, env.timer);
        // The exhaust and the skid marks keep their state in the driver, so they run for every
        // kart even when nobody sees the particles.
        puffs = vehicle::exhaust::exhaust(driver);
        driver.mud_particles = vehicle::sparks::mud(driver);
        vehicle::skid::skidmarks(driver);
        if driver.actions_flag_set & actions::BOT == 0 {
            vehicle::emitter_sound::tick(driver, effects);
        }
    }
    (landing, wall, spray, puffs)
}

/// `PICKUPBOTS_UPDATE_START_DELAY`: the bots fire nothing in the first 19.2 seconds of a race.
const BOT_WEAPONS_FROM_MS: i32 = 0x4b00;
/// `PICKUPBOTS_CLOSE_DIST_SQ_BIAS` and `_RANGE`: how far from a player (squared, flat) a bot
/// next to it in the race fires at it.
const BOT_WEAPON_NEAREST: u32 = 0x90001;
const BOT_WEAPON_RANGE: u32 = 0x13a_ffff;
/// `PICKUPBOTS_TRAILING_ATTACK_DISTANCE_TO_FINISH_MIN`
const BOT_TRAILING_NEAR_FINISH: u32 = 16000;
/// `PICKUPBOTS_VOICELINE_*`: what the bot says to the player it fires at.
const BOT_VOICE_BOMB: u8 = 10;
const BOT_VOICE_MISSILE: u8 = 11;
const BOT_VOICE_MINE: u8 = 0xf;

impl Race {
    /// `PickupBots_UpdateArcade`: the bot just ahead of each player in the race, and the one just
    /// behind, may fire at it when it is near enough: the one ahead drops a TNT or a beaker now
    /// and then (Nitro or red after the first lap, half the time), or fires a bomb or a missile;
    /// the one behind a missile (not on its last lap) or a bomb. Then it waits a while
    /// (`weaponCooldown`). The weapons go off on the next tick, as ones fired this tick do.
    fn bot_weapons(
        &mut self,
        level: &Level,
        doors: &coll::doors::Doors,
        players: usize,
        ticks: &mut [KartTick],
    ) {
        if self.bots.is_none() || players == 0 || self.event_ms < BOT_WEAPONS_FROM_MS {
            return;
        }
        let rank_of = |karts: &[Kart], rank: i32| {
            let at = |k: &Kart| k.driver.driver_rank as i32 == rank;
            if rank < 0 {
                None
            } else {
                karts.iter().position(at)
            }
        };
        for player in 0..players {
            let rank = self.karts[player].driver.driver_rank as i32;
            if rank != 0
                && let Some(bot) = rank_of(&self.karts, rank - 1)
                && self.bot_may_fire(bot, player)
            {
                let rng = &mut self.objects.random_number;
                let roll = math::mix_rng_scramble(rng) % 200;
                let shot = match roll {
                    0 => {
                        let bot_driver = &mut self.karts[bot].driver;
                        if bot_driver.lap_index != 0 && math::mix_rng_scramble(rng) % 100 < 50 {
                            bot_driver.num_wumpas = 10;
                        }
                        // (Retail picks by the parity of the race's milliseconds, which here go
                        // by 32: the tick's stands in.)
                        let launch = if self.tick & 1 != 0 {
                            vehicle::item::Launch::Mine
                        } else {
                            vehicle::item::Launch::Potion
                        };
                        Some((launch, BOT_VOICE_MINE))
                    }
                    1 => Some((vehicle::item::Launch::Bomb, BOT_VOICE_BOMB)),
                    2 => Some((vehicle::item::Launch::Missile, BOT_VOICE_MISSILE)),
                    _ => None,
                };
                if let Some((launch, voice)) = shot {
                    self.bot_fires(bot, launch, voice, level, doors, ticks);
                    if roll == 0 {
                        self.karts[bot].driver.num_wumpas = 0;
                    }
                    self.bot_cooldown(bot);
                }
            }
            let p = &self.karts[player].driver;
            let still_racing =
                p.lap_index < self.laps || p.distance_to_finish_curr > BOT_TRAILING_NEAR_FINISH;
            if rank < 3
                && still_racing
                && let Some(bot) = rank_of(&self.karts, rank + 1)
                && self.bot_may_fire(bot, player)
            {
                let roll = math::mix_rng_scramble(&mut self.objects.random_number) % 800;
                let last_lap = self.karts[bot].driver.lap_index as i32 == self.laps as i32 - 1;
                let launch = if roll < 2 && !last_lap {
                    Some(vehicle::item::Launch::Missile)
                } else if roll < 4 {
                    Some(vehicle::item::Launch::Bomb)
                } else {
                    None
                };
                if let Some(launch) = launch {
                    self.bot_fires(bot, launch, BOT_VOICE_MISSILE, level, doors, ticks);
                    self.bot_cooldown(bot);
                }
            }
        }
    }

    /// Whether the kart in `bot` is a bot that can fire at the `player`'s: not hurt, still
    /// racing, its cooldown over, no TNT on its head, no clock on it, and near enough.
    fn bot_may_fire(&self, bot: usize, player: usize) -> bool {
        let b = &self.karts[bot].driver;
        let Some(data) = &b.bot else {
            return false;
        };
        let ready = data.flags & vehicle::bot::flags::DAMAGE_ACTIVE == 0
            && b.actions_flag_set & actions::RACE_FINISHED == 0
            && data.weapon_cooldown == 0
            && b.tnt_scale_y.is_none()
            && b.clock_receive == 0;
        let (p, at) = (self.karts[player].driver.inst_pos, b.inst_pos);
        let (x, z) = (p[0].wrapping_sub(at[0]), p[2].wrapping_sub(at[2]));
        let distance = x.wrapping_mul(x).wrapping_add(z.wrapping_mul(z)) as u32;
        ready && distance.wrapping_sub(BOT_WEAPON_NEAREST) < BOT_WEAPON_RANGE
    }

    /// `VehPickupItem_ShootNow` for a bot, with what it says to the player it fires at.
    fn bot_fires(
        &mut self,
        bot: usize,
        launch: vehicle::item::Launch,
        voice: u8,
        level: &Level,
        doors: &coll::doors::Doors,
        ticks: &mut [KartTick],
    ) {
        let bodies: Vec<weapon::Body> = self
            .karts
            .iter()
            .map(|k| weapon::Body::of(&k.driver))
            .collect();
        let aim = Aim {
            level,
            bodies: &bodies,
            doors,
            elapsed_time_ms: Env::default().elapsed_time_ms,
        };
        let mut effects = vec![Vec::new(); self.karts.len()];
        effects[bot].push(Effect::Voiceline {
            id: voice,
            driver_id: bot as u8,
            priority: vehicle::pick::VOICELINE_PRIORITY,
        });
        let fired = vehicle::item::Fired::Weapon(launch);
        self.fire(bot, fired, &Input::default(), &aim, &mut effects);
        for (tick, effects) in ticks.iter_mut().zip(effects) {
            tick.effects.extend(effects);
        }
    }

    /// `PickupBots_SetCooldown`: 4 to 8 seconds before the bot fires again (the drive counts it down
    /// twice a tick).
    fn bot_cooldown(&mut self, bot: usize) {
        let roll = math::mix_rng_scramble(&mut self.objects.random_number);
        if let Some(b) = &mut self.karts[bot].driver.bot {
            b.weapon_cooldown = ((roll & 0xff) + 0xf0) as i16;
        }
    }
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
            "{} {} {} {} {:?} {} {} {:?} {:?} {:?} {:?} {:?}",
            self.tick,
            self.event_ms,
            self.laps,
            self.finished,
            self.winner,
            self.lights_ms,
            self.intro_ticks,
            self.lap_times,
            self.checkpoints,
            self.splits,
            self.feedback,
            self.objects.ticks()
        );
        for kart in &self.karts {
            let _ = writeln!(hash, "{:?} {:?} {}", kart.driver, kart.coll, kart.held);
        }
        let _ = write!(
            hash,
            "{:?} {:?} {:?} {:?}",
            self.last, self.weapons, self.last_weapons, self.bots
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_goes_from_the_worst_class_to_the_best_times_the_boost() {
        use vehicle::bot::difficulty_level::{EASY, HARD};
        use vehicle::stats::{EngineClass, Stats};
        let track = 6000;
        let (easy, hard) = (Band::new([1, 2], EASY), Band::new([1, 2], HARD));
        assert!(easy.has(1) && easy.has(2) && !easy.has(0));
        // Past the full gap it pulls no harder; a track of no length not at all.
        assert_eq!(Band::pull(1000, track), BAND_ONE);
        assert_eq!(Band::pull(5000, track), BAND_ONE);
        assert_eq!(Band::pull(-5000, track), -BAND_ONE);
        assert_eq!(Band::pull(1000, 0), 0);
        let turn = Stats::for_class(EngineClass::Turn);
        let speed = |c: &Stats| c.speed_class_stat as i64;
        let accel = |c: &Stats| c.accel_class_stat as i64;
        // No gap: its own. Trailing on hard: the Speed class's top speed and the Accel class's
        // acceleration, × 1.08. Leading: the worst top speed (the Turn class's own), and its
        // own acceleration.
        assert_eq!(hard.stat(&turn, speed, 0, true), 12950);
        assert_eq!(hard.stat(&turn, speed, BAND_ONE, true), 13900 * 4424 / 4096);
        assert_eq!(hard.stat(&turn, accel, BAND_ONE, false), 544 * 4424 / 4096);
        assert_eq!(hard.stat(&turn, speed, -BAND_ONE, true), 12950);
        assert_eq!(hard.stat(&turn, accel, -BAND_ONE, false), 512);
        let fast = Stats::for_class(EngineClass::Speed);
        assert_eq!(easy.stat(&fast, speed, -BAND_ONE, true), 12950);
        // Hard helps a trailer more than easy.
        assert!(hard.stat(&turn, speed, BAND_ONE, true) > easy.stat(&turn, speed, BAND_ONE, true));
    }

    #[test]
    fn the_lights_come_on_one_by_one() {
        assert_eq!(lights_on(START_LIGHTS_MS), 0);
        assert_eq!(lights_on(0xb40 + 1), 0);
        assert_eq!(lights_on(0xb40), 1);
        assert_eq!(lights_on(0x780), 2);
        assert_eq!(lights_on(1), 3);
        assert_eq!(lights_on(0), 4);
        assert_eq!(lights_on(LIGHTS_FLOOR_MS), 4);
    }

    use instance::{Contact, Damage, Mines};
    use weapon::mine::MineKind;

    /// Mines at the x of `xs` (by the order they were dropped), and what a hazard at x = 0 that
    /// reaches 100 units sets off, by x.
    fn set_off_by(contact: Contact, karts: &[[i32; 3]], xs: &[i32]) -> Vec<i32> {
        let mut weapons = weapon::Weapons::default();
        for &x in xs {
            weapons.place_mine(MineKind::Nitro, [x, 0, 0], 0);
        }
        set_off_mines(&[contact], karts, &mut weapons);
        weapons
            .mines()
            .iter()
            .filter(|m| m.is_dead())
            .map(|m| m.pos[0])
            .collect()
    }

    fn seal() -> Contact {
        Contact::sphere([0, 0, 0], Damage::Spin, 100 * 100, 0)
    }

    #[test]
    fn a_hazard_sets_off_the_newest_mine_in_its_reach_when_no_kart_is() {
        assert_eq!(set_off_by(seal().and_mines(), &[], &[50, 500, 20]), [20]);
        assert_eq!(set_off_by(seal().and_mines(), &[], &[500]), [] as [i32; 0]);
    }

    #[test]
    fn a_kart_in_reach_keeps_the_mines_from_going_off_even_unhurt() {
        assert_eq!(
            set_off_by(seal().and_mines(), &[[0, 0, 90]], &[50]),
            [] as [i32; 0]
        );
        assert_eq!(set_off_by(seal().and_mines(), &[[0, 0, 110]], &[50]), [50]);
    }

    #[test]
    fn a_hazard_that_only_looks_for_drivers_sets_off_nothing() {
        assert_eq!(set_off_by(seal(), &[], &[50]), [] as [i32; 0]);
    }

    #[test]
    fn a_grab_sets_off_a_mine_with_the_kart_in_its_mouth() {
        let grab = Contact {
            mines: Mines::Only,
            ..seal()
        };
        assert_eq!(set_off_by(grab, &[[0, 0, 0]], &[50]), [50]);
    }

    #[test]
    fn a_contact_that_only_looks_for_mines_hurts_no_kart() {
        let mut driver = Driver::default();
        let at = driver.inst_pos;
        let grab = Contact {
            mines: Mines::Only,
            ..Contact::sphere(at, Damage::Spin, 100 * 100, 0)
        };
        let objects = instance::Tick {
            contacts: vec![grab],
            ..Default::default()
        };
        let mut effects = Vec::new();
        before_tick(&mut driver, &objects, &mut Vec::new(), &mut effects);
        assert!(!driver.pending_spin);
        assert!(effects.is_empty());
    }
}
