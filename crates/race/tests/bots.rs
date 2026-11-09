//! The bots drive the race tracks: each of them gets round a lap in time, without falling off
//! the track. Skipped without the disc image. Run with `--nocapture` for the lap times, to tune
//! the steering by.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use race::bot::Steering;
use race::{KartSpec, Pad, Race};
use vehicle::stats::{CHARACTER_ENGINES, EngineClass};
use vehicle::{PhysState, actions, kart_state};

fn level(id: LevelId) -> Option<(Level, Objects)> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
    let level = Level::parse(&dram).unwrap();
    let objects = Objects::new(&level, &dram);
    Some((level, objects))
}

/// One of each engine class.
const BOTS: usize = 4;
const CLASSES: [EngineClass; BOTS] = [
    EngineClass::Balanced,
    EngineClass::Accel,
    EngineClass::Speed,
    EngineClass::Turn,
];

/// Two minutes: a lap of any track takes well under one.
const TICKS: u32 = 30 * 120;

/// The first character of an engine class.
fn character_of(class: EngineClass) -> usize {
    CHARACTER_ENGINES.iter().position(|&c| c == class).unwrap()
}

/// A bot of each engine class, the classes turned by `turn` places along the slots: as each slot
/// has a path of its own, so that over the tracks every class drives every path.
fn bots(turn: usize) -> Vec<KartSpec> {
    (0..BOTS)
        .map(|slot| KartSpec {
            character: character_of(CLASSES[(slot + turn) % BOTS]),
            anim_num_frames: [0; 8],
            bot: true,
            line: None,
        })
        .collect()
}

/// How a bot did on a track.
struct Run {
    id: LevelId,
    class: EngineClass,
    path: u8,
    /// Its lap's time, if it finished it.
    lap_ms: Option<i32>,
    /// How many times it was grabbed by the mask (off the track, into a kill plane), and eaten
    /// by a plant (which goes through the same state, but is the hazard's doing).
    grabs: u32,
    eaten: u32,
    relocations: u16,
    /// Where on its path it was at the end.
    frame: u16,
    /// Each time the mask grabbed it.
    falls: Vec<Fall>,
}

/// A time the mask grabbed a bot.
#[derive(Clone)]
struct Fall {
    tick: u32,
    /// The frame of its path it was at.
    frame: u16,
    /// How many ticks it had been in the air (0: it drove off, or into a kill plane on the
    /// ground).
    air: u32,
    /// How it left the ground, if it was in the air.
    takeoff: Option<Moment>,
    /// Where it was the tick before the mask had it.
    last: Moment,
    /// The frames of the path about there (from 3 before the take-off's to 2 after the last
    /// one's), as (number, frame).
    around: Vec<(usize, loader::lev::NavFrame)>,
    /// The gap it was jumping, if there was one ahead of where it took off.
    gap: Option<Gap>,
}

/// A gap of a path: a ramp's lip (a frame that launches retail's bots), the frames under the gap
/// in a kill plane, and the first frame past them.
#[derive(Clone)]
struct Gap {
    lip: (usize, [i32; 3]),
    landing: (usize, [i32; 3]),
    /// The lip's `rampPhys`: what retail launches its bots with there.
    ramp_xz: i16,
    ramp_y: i16,
}

impl Gap {
    /// The gap ahead of `frame` of `path`, if the lip is a few frames on.
    fn ahead(path: &loader::lev::NavPath, frame: usize) -> Option<Gap> {
        let frames = &path.frames;
        let n = frames.len();
        let at = |i: usize| {
            let p = frames[i % n].pos;
            [p.x as i32, p.y as i32, p.z as i32]
        };
        // (The bots' own idea of a gap's landing.)
        let (lip, landing) = (frame..frame + 8)
            .find_map(|i| race::bot::gap_landing(frames, i % n).map(|j| (i, j)))?;
        let index = (frames[lip % n].special_bits & 0xf) as usize;
        Some(Gap {
            lip: (lip % n, at(lip)),
            landing: (landing % n, at(landing)),
            ramp_xz: path.ramp_phys_xz[index],
            ramp_y: path.ramp_phys_y[index],
        })
    }
}

impl std::fmt::Display for Gap {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let ((i, a), (j, b)) = (self.lip, self.landing);
        let (dx, dz) = ((b[0] - a[0]) as f64, (b[2] - a[2]) as f64);
        write!(
            f,
            "gap from frame {i} at {a:?} to frame {j} at {b:?}: {:.0} on the ground, {} up; \
             rampPhys xz {}, y {}",
            (dx * dx + dz * dz).sqrt(),
            b[1] - a[1],
            self.ramp_xz,
            self.ramp_y
        )
    }
}

/// Where a bot was and how it was going, against its path.
#[derive(Clone, Copy)]
struct Moment {
    tick: u32,
    frame: u16,
    /// `speed_approx`, and the class's top speed (`speed_class_stat`).
    speed: i16,
    top: i16,
    /// `velocity.y`.
    up: i32,
    /// Where it was (level units).
    pos: [i32; 3],
    /// How far it was from the line of its path, to the left (level units, on the ground plane).
    off_line: i32,
    /// Its heading against the path's there, in degrees: positive to the left.
    heading: i32,
    /// How high it was above the frame (level units).
    above: i32,
    /// `NavFrame::flags` and `special_bits` of the frame (a jump is 0x400).
    flags: u16,
    special: u8,
}

impl std::fmt::Display for Moment {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let t = self;
        write!(
            f,
            "at tick {}, frame {}, at {:?}: speed {} of {}, up {}, {} off the line, {} above \
             it, heading {} deg, flags {:#06x}, special {:#04x}",
            t.tick,
            t.frame,
            t.pos,
            t.speed,
            t.top,
            t.up,
            t.off_line,
            t.above,
            t.heading,
            t.flags,
            t.special
        )
    }
}

impl Moment {
    fn of(kart: &race::Kart, path: &[loader::lev::NavFrame], tick: u32) -> Moment {
        let d = &kart.driver;
        let frame = kart.bot.as_ref().unwrap().tracker.frame;
        let i = frame as usize;
        let (a, b) = (path[i].pos, path[(i + 1) % path.len()].pos);
        let (dx, dz) = (b.x as i32 - a.x as i32, b.z as i32 - a.z as i32);
        let (px, pz) = ((d.pos_curr.x >> 8) - a.x as i32, (d.pos_curr.z >> 8) - a.z as i32);
        let len = ((dx * dx + dz * dz) as f64).sqrt().max(1.0);
        // Left raises the angle, which runs from +z towards +x.
        let off_line = ((dx * pz - dz * px) as f64 / len) as i32;
        let along = math::ratan2(dx, dz);
        let heading = (((d.angle as i32 - along + 0x800) & 0xfff) - 0x800) * 360 / 0x1000;
        Moment {
            tick,
            frame,
            speed: d.speed_approx,
            top: d.stats.speed_class_stat,
            up: d.velocity.y,
            pos: [d.pos_curr.x >> 8, d.pos_curr.y >> 8, d.pos_curr.z >> 8],
            above: (d.pos_curr.y >> 8) - path[i].pos.y as i32,
            off_line,
            heading,
            flags: path[i].flags,
            special: path[i].special_bits,
        }
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let (id, class, path) = (self.id, self.class, self.path);
        write!(f, "{id:?} {class:?} (path {path}): ")?;
        match self.lap_ms {
            Some(ms) => write!(f, "lap {}.{:03} s", ms / 1000, ms % 1000)?,
            None => write!(f, "no lap, at frame {}", self.frame)?,
        }
        write!(
            f,
            ", mask grabs {}, eaten {}, relocations {}",
            self.grabs, self.eaten, self.relocations
        )?;
        // The first few, where it goes wrong.
        for fall in self.falls.iter().take(3) {
            let (tick, frame, air) = (fall.tick, fall.frame, fall.air);
            write!(f, "\n    fell at tick {tick}, frame {frame}, {air} ticks in the air")?;
            if let Some(t) = fall.takeoff {
                write!(f, "\n      took off {t}")?;
            }
            write!(f, "\n      last seen {}", fall.last)?;
            if let Some(gap) = &fall.gap {
                write!(f, "\n      {gap}")?;
            }
            for (i, frame) in &fall.around {
                let p = frame.pos;
                write!(
                    f,
                    "\n        frame {i}: at {:?}, {} to the next ({} on the ground), flags \
                     {:#06x}, special {:#04x}",
                    [p.x, p.y, p.z],
                    frame.dist_to_next_xyz,
                    frame.dist_to_next_xz,
                    frame.flags,
                    frame.special_bits
                )?;
            }
        }
        Ok(())
    }
}

/// The race tracks, or None without the disc.
fn tracks() -> Option<Vec<(LevelId, Level, Objects)>> {
    (0..=17)
        .map(LevelId)
        .map(|id| level(id).map(|(level, objects)| (id, level, objects)))
        .collect()
}

/// A lap of each track by [`BOTS`] bots that steer by `steering`.
fn drive(tracks: &[(LevelId, Level, Objects)], steering: Steering) -> Vec<Run> {
    let mut runs = Vec::new();
    for (turn, (id, level, objects)) in tracks.iter().enumerate() {
        // No karts bumping: each bot is on its own.
        let specs = bots(turn);
        let mut race = Race::new(level, objects.clone(), &specs, false);
        race.laps = 1;
        for kart in &mut race.karts {
            kart.bot.as_mut().unwrap().steering = steering;
        }
        let pads = vec![Pad::default(); BOTS];
        let mut grabs = [0u32; BOTS];
        let mut eaten = [0u32; BOTS];
        let mut falls = vec![Vec::new(); BOTS];
        // How many ticks each has been in the air for, and how it took off.
        let mut air = [0u32; BOTS];
        let mut takeoffs: [Option<Moment>; BOTS] = [None; BOTS];
        let path_of = |kart: &race::Kart| {
            &level.nav_paths[kart.bot.as_ref().unwrap().path as usize].frames[..]
        };
        while race.tick < TICKS && (race.finished as usize) < BOTS {
            let before: Vec<u8> = race.karts.iter().map(|k| k.driver.kart_state).collect();
            let seen: Vec<Moment> = race
                .karts
                .iter()
                .map(|k| Moment::of(k, path_of(k), race.tick))
                .collect();
            race.tick(level, &pads);
            for (slot, kart) in race.karts.iter().enumerate() {
                let grabbed = kart.driver.kart_state == kart_state::MASK_GRABBED;
                if grabbed && before[slot] != kart_state::MASK_GRABBED {
                    if kart.driver.phys_state == PhysState::PlantEaten {
                        eaten[slot] += 1;
                    } else {
                        grabs[slot] += 1;
                        falls[slot].push(Fall {
                            tick: race.tick,
                            frame: kart.bot.as_ref().unwrap().tracker.frame,
                            air: air[slot],
                            takeoff: takeoffs[slot].filter(|_| air[slot] > 0),
                            last: seen[slot],
                            around: {
                                let path = path_of(kart);
                                let n = path.len();
                                let from = takeoffs[slot]
                                    .filter(|_| air[slot] > 0)
                                    .map_or(seen[slot].frame, |t| t.frame)
                                    as usize;
                                let to = seen[slot].frame as usize;
                                // (Forward over the end of the loop, and at most a dozen.)
                                let count = ((to + n - from) % n + 6).min(12);
                                (0..count)
                                    .map(|k| (from + n - 3 + k) % n)
                                    .map(|i| (i, path[i]))
                                    .collect()
                            },
                            gap: takeoffs[slot].filter(|_| air[slot] > 0).and_then(|t| {
                                let path = &level.nav_paths
                                    [kart.bot.as_ref().unwrap().path as usize];
                                Gap::ahead(path, t.frame as usize)
                            }),
                        });
                    }
                }
                if kart.driver.actions_flag_set & actions::TOUCH_GROUND == 0 {
                    if air[slot] == 0 {
                        takeoffs[slot] = Some(Moment::of(kart, path_of(kart), race.tick));
                    }
                    air[slot] += 1;
                } else {
                    air[slot] = 0;
                }
            }
        }
        for (slot, kart) in race.karts.iter().enumerate() {
            let bot = kart.bot.as_ref().unwrap();
            let done = kart.driver.actions_flag_set & actions::RACE_FINISHED != 0;
            runs.push(Run {
                id: *id,
                class: CHARACTER_ENGINES[specs[slot].character],
                path: bot.path,
                lap_ms: race.lap_times[slot].first().copied().filter(|_| done),
                grabs: grabs[slot],
                eaten: eaten[slot],
                relocations: bot.tracker.relocations,
                frame: bot.tracker.frame,
                falls: std::mem::take(&mut falls[slot]),
            });
        }
    }
    runs
}

#[test]
fn needs_disc_bots_drive_a_lap_of_every_track() {
    let Some(tracks) = tracks() else {
        return;
    };
    let runs = drive(&tracks, Steering::DEFAULT);
    for run in &runs {
        println!("{run}");
    }
    let no_lap: Vec<_> = runs
        .iter()
        .filter(|r| r.lap_ms.is_none())
        .map(|r| (r.id, r.class))
        .collect();
    let fell: Vec<_> = runs
        .iter()
        .filter(|r| r.grabs > 0)
        .map(|r| (r.id, r.class, r.grabs))
        .collect();
    assert!(
        no_lap.is_empty() && fell.is_empty(),
        "no lap in time: {no_lap:?}\ngrabbed by the mask (track, class, times): {fell:?}"
    );
}

/// Not a test: tries steering settings against each other, a line for each. Run with
/// `cargo test -p race --test bots tune -- --ignored --nocapture`; `BOT_TUNE=<text>` runs only
/// the tries with that in their names (and the default).
#[test]
#[ignore]
fn needs_disc_tune_steering() {
    let Some(tracks) = tracks() else {
        return;
    };
    let d = Steering::DEFAULT;
    let tries = [
        ("default", d),
        (
            "the first (stick 0x100, no prediction)",
            Steering {
                full_error: 0x100,
                predict_ticks: 0,
                even_strength: false,
                ..d
            },
        ),
        ("far 1/4", Steering { far_weight: 64, ..d }),
        ("predict 8", Steering { predict_ticks: 8, ..d }),
        ("predict 10", Steering { predict_ticks: 10, ..d }),
        ("0xa0", Steering { full_error: 0xa0, ..d }),
        ("0xa0, far 1/4", Steering { full_error: 0xa0, far_weight: 64, ..d }),
        ("lookahead 9", Steering { lookahead_ticks: 9, ..d }),
        ("lookahead 15", Steering { lookahead_ticks: 15, ..d }),
        ("lookahead 6", Steering { lookahead_ticks: 6, ..d }),
        ("lookahead min 150", Steering { lookahead_min: 150, ..d }),
        ("lookahead min 450", Steering { lookahead_min: 450, ..d }),
        ("lookahead min 150, 9", Steering { lookahead_min: 150, lookahead_ticks: 9, ..d }),
        ("lookahead min 450, 9", Steering { lookahead_min: 450, lookahead_ticks: 9, ..d }),
        ("no straight gaps", Steering { straight_gaps: false, ..d }),
        ("no firm gaps", Steering { gap_full_error: 0, ..d }),
        ("gap 0x30", Steering { gap_full_error: 0x30, ..d }),
        ("gap 0x60", Steering { gap_full_error: 0x60, ..d }),
        ("no air turn", Steering { air_turn: 0, ..d }),
        ("air turn 0x20", Steering { air_turn: 0x20, ..d }),
        ("air turn 0x100", Steering { air_turn: 0x100, ..d }),
        // A d-pad: the stick centred or all the way, just past the dead zone (0x10) and on the
        // run-up to a gap at any error.
        ("d-pad", Steering { full_error: 0x11, gap_full_error: 1, ..d }),
    ];
    // `BOT_TUNE=lookahead`: only the tries whose names have that in them (and the default, to
    // compare with).
    let only = std::env::var("BOT_TUNE").ok();
    for (name, steering) in tries {
        if only
            .as_deref()
            .is_some_and(|only| name != "default" && !name.contains(only))
        {
            continue;
        }
        let runs = drive(&tracks, steering);
        let laps: Vec<i32> = runs.iter().filter_map(|r| r.lap_ms).collect();
        let grabs: u32 = runs.iter().map(|r| r.grabs).sum();
        let mut bad: Vec<String> = runs
            .iter()
            .filter(|r| r.lap_ms.is_none() || r.grabs > 0)
            .map(|r| format!("{}", r.id.0))
            .collect();
        bad.dedup();
        // The mean lap on the walled tracks, which the bots all got round before (so that the
        // settings compare on speed, too).
        let walled = [2, 3, 4, 5, 6, 14, 15, 16];
        let walled_laps: Vec<i64> = runs
            .iter()
            .filter(|r| walled.contains(&r.id.0))
            .filter_map(|r| r.lap_ms.map(i64::from))
            .collect();
        let mean = walled_laps.iter().sum::<i64>() / walled_laps.len().max(1) as i64;
        println!(
            "{name:28} laps {:2}/{}, mask grabs {grabs:3}, mean walled lap {}.{:03} s ({} laps), \
             tracks with trouble: {}",
            laps.len(),
            runs.len(),
            mean / 1000,
            mean % 1000,
            walled_laps.len(),
            bad.join(" "),
        );
    }
}

/// The bots are part of the race: the same race plays the same, and a snapshot of it plays on
/// as the original did.
#[test]
fn needs_disc_bots_play_the_same() {
    let Some((level, objects)) = level(LevelId::DINGO_CANYON) else {
        return;
    };
    let pads = vec![Pad::default(); BOTS];
    let mut a = Race::new(&level, objects, &bots(0), true);
    for _ in 0..300 {
        a.tick(&level, &pads);
    }
    let mut b = a.clone();
    for _ in 0..300 {
        a.tick(&level, &pads);
        b.tick(&level, &pads);
    }
    assert_eq!(a.checksum(), b.checksum());
    for (ka, kb) in a.karts.iter().zip(&b.karts) {
        assert_eq!(ka.bot, kb.bot);
    }
}

/// Not a test: a tick-by-tick trace of one bot over a stretch of its path, to see what it does
/// there. Hot Air Skyway's last gap for the Accel bot of path 2 (the lip is frame 384), which
/// overshoots the turn onto the gap's line. (Its second gap: Balanced, path 1, frames 136..=175.) Run
/// with `cargo test -p race --test bots trace -- --ignored --nocapture`.
#[test]
#[ignore]
fn needs_disc_trace_gap() {
    const ID: LevelId = LevelId::HOT_AIR_SKYWAY;
    const CLASS: EngineClass = EngineClass::Accel;
    // Path 2 is the slot 2's.
    const SLOT: usize = 2;
    const FRAMES: std::ops::RangeInclusive<usize> = 370..=400;
    let Some((level, objects)) = level(ID) else {
        return;
    };
    let specs: Vec<KartSpec> = (0..=SLOT)
        .map(|_| KartSpec {
            character: character_of(CLASS),
            anim_num_frames: [0; 8],
            bot: true,
            line: None,
        })
        .collect();
    let mut race = Race::new(&level, objects, &specs, false);
    race.laps = 1;
    let path = &level.nav_paths[race.karts[SLOT].bot.as_ref().unwrap().path as usize].frames;
    println!("path {}:", race.karts[SLOT].bot.as_ref().unwrap().path);
    for i in FRAMES {
        let f = &path[i];
        println!(
            "  frame {i}: at {:?}, rot {:?}, {} to the next ({} on the ground), flags {:#06x}, \
             special {:#04x}",
            [f.pos.x, f.pos.y, f.pos.z],
            f.rot,
            f.dist_to_next_xyz,
            f.dist_to_next_xz,
            f.flags,
            f.special_bits
        );
    }
    let pads = vec![Pad::default(); specs.len()];
    let mut tracing = false;
    while race.tick < TICKS {
        let kart = &race.karts[SLOT];
        let d = &kart.driver;
        let frame = kart.bot.as_ref().unwrap().tracker.frame as usize;
        let ground = d.actions_flag_set & actions::TOUCH_GROUND != 0;
        if FRAMES.contains(&frame) {
            tracing = true;
        } else if tracing && ground {
            break;
        }
        if tracing {
            // The pad it is about to make (on a copy: `pad` moves the tracker).
            let pad = kart.bot.clone().unwrap().pad(&level, d, true);
            let v = d.velocity;
            // Which way it goes on the ground plane, against which way it faces (0x1000 a turn).
            let going = math::ratan2(v.x >> 8, v.z >> 8);
            let slip = (((going - d.angle as i32 + 0x800) & 0xfff) - 0x800) as i32;
            // Which way it goes against the line from the lip of the gap ahead to its landing
            // (degrees, + to the left).
            let bot_path = &level.nav_paths[kart.bot.as_ref().unwrap().path as usize];
            let line = Gap::ahead(bot_path, frame).map_or(String::new(), |gap| {
                let (a, b) = (gap.lip.1, gap.landing.1);
                let along = math::ratan2(b[0] - a[0], b[2] - a[2]);
                let off = ((going - along + 0x800) & 0xfff) - 0x800;
                format!(", line {:+5.1} deg (lip {})", off as f64 * 360.0 / 4096.0, gap.lip.0)
            });
            println!(
                "tick {:4} frame {frame:3} {} at {:?}: angle {:#05x}, slip {:+5}{line}, velocity {:?}, \
                 speed {:5} (base {:5}, fire {:5}), turn {:+5}, spin {:+5}, steps {:#x}, \
                 state {}, stick {:#04x}, held {:#06x}",
                race.tick,
                if ground { "ground" } else { "air   " },
                [d.pos_curr.x >> 8, d.pos_curr.y >> 8, d.pos_curr.z >> 8],
                d.angle & 0xfff,
                slip,
                [v.x, v.y, v.z],
                d.speed_approx,
                d.base_speed,
                d.fire_speed,
                d.amp_turn_state,
                d.rotation_spin_rate,
                d.step_flag_set,
                d.kart_state,
                pad.lx,
                pad.held,
            );
            if d.kart_state == kart_state::MASK_GRABBED {
                println!("grabbed");
                break;
            }
        }
        race.tick(&level, &pads);
    }
}

/// A bot leaving the ground before a gap, in [`needs_disc_gap_takeoffs`].
struct Takeoff {
    class: EngineClass,
    path: u8,
    tick: u32,
    lip: usize,
    /// Which way it flies against the line from the lip to the landing (degrees, + to the left),
    /// and how far it is from that line (level units, + to the left, as [`Moment`]'s).
    heading: f64,
    off_line: i32,
    speed: i16,
    /// The most the stick was off the centre over the ground ticks before.
    stick: i32,
    /// How it came down: landed, or grabbed by the mask.
    landed: bool,
}

/// How each class's bots leave the lips of the gaps of a track, on each path, steering by
/// `steering`: a lap each.
fn gap_takeoffs(level: &Level, objects: &Objects, steering: Steering) -> Vec<Takeoff> {
    const STICK_TICKS: usize = 12;
    let paths = level.nav_paths.iter().filter(|p| p.frames.len() >= 2).count();
    let mut out = Vec::new();
    for class in CLASSES {
        let specs: Vec<KartSpec> = (0..paths)
            .map(|_| KartSpec {
                character: character_of(class),
                anim_num_frames: [0; 8],
                bot: true,
                line: None,
            })
            .collect();
        let n = specs.len();
        let mut race = Race::new(level, objects.clone(), &specs, false);
        race.laps = 1;
        for kart in &mut race.karts {
            kart.bot.as_mut().unwrap().steering = steering;
        }
        let pads = vec![Pad::default(); n];
        let mut sticks: Vec<std::collections::VecDeque<i32>> = vec![Default::default(); n];
        // The takeoff of each in the air before a gap, until it comes down.
        let mut flying: Vec<Option<Takeoff>> = (0..n).map(|_| None).collect();
        while race.tick < TICKS && (race.finished as usize) < n {
            for (slot, kart) in race.karts.iter().enumerate() {
                if kart.driver.actions_flag_set & actions::TOUCH_GROUND != 0 {
                    let pad = kart.bot.clone().unwrap().pad(level, &kart.driver, true);
                    let q = &mut sticks[slot];
                    q.push_back((pad.lx as i32 - 0x80).abs());
                    if q.len() > STICK_TICKS {
                        q.pop_front();
                    }
                }
            }
            let before: Vec<u8> = race.karts.iter().map(|k| k.driver.kart_state).collect();
            race.tick(level, &pads);
            for (slot, kart) in race.karts.iter().enumerate() {
                let d = &kart.driver;
                let bot = kart.bot.as_ref().unwrap();
                let ground = d.actions_flag_set & actions::TOUCH_GROUND != 0;
                let grabbed = d.kart_state == kart_state::MASK_GRABBED
                    && before[slot] != kart_state::MASK_GRABBED;
                if let Some(mut t) = flying[slot].take() {
                    if ground || grabbed {
                        t.landed = ground && !grabbed;
                        out.push(t);
                    } else {
                        flying[slot] = Some(t);
                    }
                    continue;
                }
                // (Nor the drop of a kart the mask puts back.)
                if ground || grabbed || d.kart_state != kart_state::NORMAL {
                    continue;
                }
                let path = &level.nav_paths[bot.path as usize];
                let Some(gap) = Gap::ahead(path, bot.tracker.frame as usize) else {
                    continue;
                };
                let (a, b) = (gap.lip.1, gap.landing.1);
                let (dx, dz) = ((b[0] - a[0]) as f64, (b[2] - a[2]) as f64);
                let len = (dx * dx + dz * dz).sqrt().max(1.0);
                let (px, pz) = (
                    ((d.pos_curr.x >> 8) - a[0]) as f64,
                    ((d.pos_curr.z >> 8) - a[2]) as f64,
                );
                let line = math::ratan2(dx as i32, dz as i32);
                let going = math::ratan2(d.velocity.x >> 8, d.velocity.z >> 8);
                let heading = (((going - line + 0x800) & 0xfff) - 0x800) as f64 * 360.0 / 4096.0;
                flying[slot] = Some(Takeoff {
                    class,
                    path: bot.path,
                    tick: race.tick,
                    lip: gap.lip.0,
                    heading,
                    off_line: ((dx * pz - dz * px) / len) as i32,
                    speed: d.speed_approx,
                    stick: sticks[slot].iter().copied().max().unwrap_or(0),
                    landed: false,
                });
            }
        }
    }
    out
}

/// Not a test: how the bots leave the lips of Hot Air Skyway's gaps, with and without the firm
/// steering on the run-up. Run with
/// `cargo test -p race --test bots gap_takeoffs -- --ignored --nocapture`.
#[test]
#[ignore]
fn needs_disc_gap_takeoffs() {
    let Some((level, objects)) = level(LevelId::HOT_AIR_SKYWAY) else {
        return;
    };
    let d = Steering::DEFAULT;
    for (name, steering) in [
        ("no firm gaps", Steering { gap_full_error: 0, ..d }),
        ("default", d),
    ] {
        println!("{name}:");
        let takeoffs = gap_takeoffs(&level, &objects, steering);
        for t in &takeoffs {
            println!(
                "  {:8} path {} tick {:4} lip {:3}: heading {:+5.1} deg, {:+4} off the line, \
                 speed {:6}, stick up to {:#04x} off, {}",
                format!("{:?}", t.class),
                t.path,
                t.tick,
                t.lip,
                t.heading,
                t.off_line,
                t.speed,
                t.stick,
                if t.landed { "landed" } else { "FELL" }
            );
        }
        let fell = takeoffs.iter().filter(|t| !t.landed).count();
        let worst = takeoffs.iter().map(|t| t.heading.abs()).fold(0.0, f64::max);
        println!("  {} takeoffs, {fell} fell, worst heading {worst:.1} deg", takeoffs.len());
    }
}

/// A race of bots that the client printed with `/debug dump-bots` (offline, spectating): the
/// `BOT_DUMP ...` line rebuilds it.
struct Dump {
    level: LevelId,
    collisions: bool,
    laps: u8,
    tick: u32,
    watched: usize,
    checksum: u64,
    specs: Vec<KartSpec>,
}

impl Dump {
    fn parse(line: &str) -> Dump {
        let line = line.trim();
        let line = line.strip_prefix("BOT_DUMP").unwrap_or(line);
        let field = |name: &str| {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(name)?.strip_prefix('='))
                .unwrap_or_else(|| panic!("the dump has no {name}="))
        };
        let number = |name: &str| field(name).parse::<u32>().unwrap();
        let specs = field("specs")
            .split(';')
            .map(|spec| {
                let parts: Vec<&str> = spec.split('/').collect();
                let (character, anim, bot, line) = match parts[..] {
                    [character, anim, bot] => (character, anim, bot, None),
                    [character, anim, bot, line] => (character, anim, bot, Some(line)),
                    _ => panic!("a spec is character/anim,frames/bot[/line]: {spec}"),
                };
                let anim: Vec<u16> = anim.split(',').map(|n| n.parse().unwrap()).collect();
                KartSpec {
                    character: character.parse().unwrap(),
                    anim_num_frames: anim.try_into().unwrap(),
                    bot: bot == "1",
                    line: line.map(|l| l.parse().unwrap()),
                }
            })
            .collect();
        let checksum = field("checksum");
        Dump {
            level: LevelId(number("level")),
            collisions: number("collisions") != 0,
            laps: number("laps") as u8,
            tick: number("tick"),
            watched: number("watched") as usize,
            checksum: u64::from_str_radix(checksum.trim_start_matches("0x"), 16).unwrap(),
            specs,
        }
    }
}

/// Replays a race of bots from the client (`/debug dump-bots`), and traces the kart that was
/// watched, tick by tick, from `BOT_TRACE_BEFORE` ticks (90) before the dump to
/// `BOT_TRACE_AFTER` (300) after it. `BOT_DUMP` is the dump's `BOT_DUMP ...` line, or a file
/// with it. Run with
/// `BOT_DUMP=dump.txt cargo test -p race --test bots replay_dump -- --ignored --nocapture`.
#[test]
#[ignore]
fn needs_disc_replay_dump() {
    let Some(text) = std::env::var_os("BOT_DUMP") else {
        eprintln!("skipping: no BOT_DUMP");
        return;
    };
    let text = text.into_string().expect("BOT_DUMP is not UTF-8");
    // A file (the test runs in `crates/race`, so a relative path is tried from the workspace's
    // root too), or the line itself.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(&text)
        .or_else(|_| std::fs::read_to_string(root.join(&text)))
        .unwrap_or_else(|_| {
            assert!(
                text.contains("BOT_DUMP") || text.contains("specs="),
                "BOT_DUMP: no file {text} (here or in the workspace's root), nor a dump line"
            );
            text
        });
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("BOT_DUMP"))
        .unwrap_or(&text);
    let dump = Dump::parse(line);
    let ticks = |name: &str, default: u32| {
        std::env::var(name).map_or(default, |v| v.parse().expect("a number of ticks"))
    };
    let before = ticks("BOT_TRACE_BEFORE", 90);
    let after = ticks("BOT_TRACE_AFTER", 300);
    let Some((level, objects)) = level(dump.level) else {
        return;
    };
    // As the client's `Sim::rollback` builds an offline race.
    let mut race = Race::new(&level, objects, &dump.specs, dump.collisions);
    race.laps = dump.laps;
    let pads = vec![Pad::default(); dump.specs.len()];
    let slot = dump.watched;
    let path_of = |kart: &race::Kart| {
        &level.nav_paths[kart.bot.as_ref().unwrap().path as usize].frames[..]
    };
    println!(
        "{:?}, {} karts, watching {slot} (path {})",
        dump.level,
        dump.specs.len(),
        race.karts[slot].bot.as_ref().unwrap().path
    );
    let mut was_ground = true;
    // The frames the trace has been on or aimed at, to list after it.
    let mut seen_frames = std::collections::BTreeSet::new();
    while race.tick < dump.tick + after {
        if race.tick == dump.tick {
            // Not the same race: the karts as the replay has them, in the dump's own format, to
            // tell which of them went another way (one kart: something done to it alone, such as
            // `.` / `,` changing the character of the watched one; all: the level or the sim).
            // `BOT_NO_CHECK=1`: the race of the dump's karts as the bots drive it now, which
            // is not the client's once the bots' code has changed (to try a fix on the dump).
            let check = std::env::var_os("BOT_NO_CHECK").is_none();
            if check && race.checksum() != dump.checksum {
                for (slot, kart) in race.karts.iter().enumerate() {
                    let d = &kart.driver;
                    println!(
                        "  kart {slot}: pos {:?} vel {:?} speed {} angle {} kart_state {} phys {:?} \
                         character {} frame {}",
                        [d.pos_curr.x >> 8, d.pos_curr.y >> 8, d.pos_curr.z >> 8],
                        [d.velocity.x, d.velocity.y, d.velocity.z],
                        d.speed,
                        d.angle,
                        d.kart_state,
                        d.phys_state,
                        d.character,
                        kart.bot.as_ref().map_or(0, |b| b.tracker.frame),
                    );
                }
            }
            assert!(
                !check || race.checksum() == dump.checksum,
                "not the client's race at tick {}: another level file, sim, or a debug command \
                 that changed the race",
                dump.tick
            );
            if check {
                println!("---- tick {}: the dump (checksum matches) ----", dump.tick);
            } else {
                println!("---- tick {}: the dump's tick (not checked) ----", dump.tick);
            }
        }
        let kart = &race.karts[slot];
        let d = &kart.driver;
        let ground = d.actions_flag_set & actions::TOUCH_GROUND != 0;
        if race.tick + before >= dump.tick {
            // The pad it is about to make (on a copy: `pad` moves the tracker).
            let mut bot = kart.bot.clone().unwrap();
            let pad = bot.pad(&level, d, race.lights_ms <= 0);
            // Where it aims: the point `Bot::pad` steers for on the ground (without the straight
            // to a gap's landing), from where the kart will be, and how far it has to turn to
            // face it (degrees, + left). The stick's side should match the error's.
            let st = bot.steering;
            let speed = (d.speed_approx as i32).abs();
            let travel = (speed >> 8) * st.predict_ticks;
            let lookahead = race::bot::lookahead(st, d);
            let (aim, aim_frame) = bot.tracker.ahead(path_of(kart), d.pos_curr, travel + lookahead);
            {
                let n = path_of(kart).len();
                let from = bot.tracker.frame as usize;
                let to = from + (aim_frame + n - from) % n;
                seen_frames.extend((from..=to + 2).map(|i| i % n));
            }
            let then = [
                d.pos_curr.x.wrapping_add(d.velocity.x.wrapping_mul(st.predict_ticks)),
                d.pos_curr.z.wrapping_add(d.velocity.z.wrapping_mul(st.predict_ticks)),
            ];
            let mut angle = d.angle as i32;
            if d.kart_state == kart_state::NORMAL {
                angle += (d.amp_turn_state as i32 * st.predict_ticks) >> 8;
            }
            let want = math::ratan2((aim.x - then[0]) >> 8, (aim.z - then[1]) >> 8);
            let error = ((want - angle + 0x800) & 0xfff) - 0x800;
            let going = math::ratan2(d.velocity.x, d.velocity.z);
            let slip = ((going - d.angle as i32 + 0x800) & 0xfff) - 0x800;
            println!(
                "{} {}, aim frame {aim_frame} at {:?} error {:+}°, slip {:+}°, stick {:#04x} ({}), held {:#06x}, \
                 state {}, stuck {}, reversing {}, relocations {}",
                if ground { "ground" } else { "air   " },
                Moment::of(kart, path_of(kart), race.tick),
                [aim.x >> 8, aim.y >> 8, aim.z >> 8],
                error * 360 / 0x1000,
                slip * 360 / 0x1000,
                pad.lx,
                match pad.lx.cmp(&0x80) {
                    std::cmp::Ordering::Less => "left",
                    std::cmp::Ordering::Equal => "centre",
                    std::cmp::Ordering::Greater => "right",
                },
                pad.held,
                d.kart_state,
                bot.stuck,
                bot.reversing,
                bot.tracker.relocations,
            );
            if !ground {
                // What `Bot::fly` turns its way of travel toward (after this tick's `pad`, as the
                // race runs it), and how far off its way of travel that is (degrees, + left; past
                // 90 it does not turn).
                let (target, landing) = bot.air_target(path_of(kart), d);
                let going = math::ratan2(d.velocity.x, d.velocity.z);
                let want = math::ratan2(
                    (target.x - d.pos_curr.x) >> 8,
                    (target.z - d.pos_curr.z) >> 8,
                );
                let off = ((want - going + 0x800) & 0xfff) - 0x800;
                let (dx, dz) = (
                    ((target.x - d.pos_curr.x) >> 8) as i64,
                    ((target.z - d.pos_curr.z) >> 8) as i64,
                );
                println!(
                    "  fly: toward {} at {:?}, {} away on the ground, {:+}° off its way",
                    match landing {
                        Some(j) => format!("the gap's landing, frame {j},"),
                        None => "the path ahead".to_string(),
                    },
                    [target.x >> 8, target.y >> 8, target.z >> 8],
                    (dx * dx + dz * dz).isqrt(),
                    off * 360 / 0x1000,
                );
            }
            if ground != was_ground {
                println!("  {}", if ground { "landed" } else { "took off" });
            }
            // A relocation: where the tracker was searched for, and the frames on from there.
            let old = kart.bot.as_ref().unwrap().tracker;
            if bot.tracker.relocations != old.relocations {
                println!("  relocated: frame {} -> {}", old.frame, bot.tracker.frame);
                let path = path_of(kart);
                for k in 0..=4 {
                    let i = (bot.tracker.frame as usize + k) % path.len();
                    let f = &path[i];
                    println!(
                        "    frame {i}: at {:?}, flags {:#06x}, special {:#04x}",
                        [f.pos.x, f.pos.y, f.pos.z],
                        f.flags,
                        f.special_bits,
                    );
                }
            }
        }
        was_ground = ground;
        let grabbed_before = race.karts[slot].driver.kart_state == kart_state::MASK_GRABBED;
        race.tick(&level, &pads);
        if !grabbed_before
            && race.karts[slot].driver.kart_state == kart_state::MASK_GRABBED
            && race.tick + before > dump.tick
        {
            println!("  grabbed by the mask");
        }
    }
    // The path where it went: what `Bot::pad` and `Bot::fly` steer by. A gap (flown straight from
    // its lip to its landing) is a frame with special 0x10 followed by a kill plane frame.
    let path = &level.nav_paths[race.karts[slot].bot.as_ref().unwrap().path as usize].frames;
    println!("---- the frames of path {} it was on or aimed at ----", race.karts[slot].bot.as_ref().unwrap().path);
    let names = [
        (loader::lev::NavFrame::TURBO, "turbo"),
        (loader::lev::NavFrame::SKID_BACK, "skid-back"),
        (loader::lev::NavFrame::SKID_FRONT, "skid-front"),
        (loader::lev::NavFrame::SUPER_TURBO, "super-turbo"),
        (loader::lev::NavFrame::KILL_PLANE, "KILL-PLANE"),
        (loader::lev::NavFrame::JUMP, "jump"),
        (loader::lev::NavFrame::DRIFT_LEFT, "drift-left"),
        (loader::lev::NavFrame::DRIFT_RIGHT, "drift-right"),
        (loader::lev::NavFrame::ECHO, "echo"),
        (0x4000, "0x4000"),
        (loader::lev::NavFrame::SPLIT_LINE, "split-line"),
    ];
    for i in seen_frames {
        let f = &path[i];
        let flags: Vec<&str> = names
            .iter()
            .filter(|(bit, _)| f.flags & bit != 0)
            .map(|(_, n)| *n)
            .collect();
        println!(
            "  frame {i}: at {:?}, rot {:?}, {} to the next ({} on the ground), flags {:#06x} [{}] \
             terrain {}, special {:#04x}{}",
            [f.pos.x, f.pos.y, f.pos.z],
            f.rot,
            f.dist_to_next_xyz,
            f.dist_to_next_xz,
            f.flags,
            flags.join(" "),
            f.terrain(),
            f.special_bits,
            match race::bot::gap_landing(path, i) {
                Some(j) => format!(", LIP of a gap landing at frame {j}"),
                None => String::new(),
            },
        );
    }
}
