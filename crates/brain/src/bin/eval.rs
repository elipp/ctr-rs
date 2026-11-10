//! Races a trained policy from the grid on each track, against the scripted bot
//! (`race::bot::Bot`) in the same kart, each alone: the laps' times, the mask grabs and the
//! boosts; and for both, the time stuck (getting no further along the path, past the first
//! second of it: `race::nn::Step::stalled`) and the share of the ticks slow (under a quarter of
//! the top speed, the mask's carry aside).
//!
//! `cargo run --release -p brain --bin eval -- [key=value ...]`, the keys being:
//!
//! - `policy` (`target/brain/policy.safetensors`): the checkpoint.
//! - `tracks` (all): the `LevelId`s, as `0,3,7`.
//! - `paths` (the first of each): the bot path to race by on some tracks, as `7:middle,11:0`
//!   (`brain::tracks::keep_paths`; the first one kept); or `all`: each track on each of its
//!   paths, the best of them (finished soonest) said, and at the end the list of the best for
//!   `train`'s `paths=`.
//! - `laps` (3).
//! - `character` (0): in `GAME_CHARACTER_IDS` order.
//! - `greedy` (true): the likeliest choice of each head, but drawn (as in training) while stuck
//!   or just put back by the mask, as the client does (`race::nn::draw_choices`); false draws
//!   them all the time.
//! - `escapes` (true): with `greedy`, whether to draw them while stuck and after a respawn; false
//!   takes the likeliest always (a stuck kart stays stuck, a fall at a spot repeats).
//! - `seed` (1): for `greedy=false`.

use std::collections::HashMap;
use std::path::PathBuf;

use brain::policy::Policy;
use brain::rng::Rng;
use brain::tracks::Track;
use race::bot::Tracker;
use race::nn::{self, Latch};
use race::{KartSpec, Pad, Race};
use vehicle::kart_state;

type Error = Box<dyn std::error::Error>;

/// A race is given up after this many ticks a lap (three minutes).
const MAX_TICKS_PER_LAP: u32 = 30 * 180;

/// The choices are held as long as in training (`brain::env::Rules::repeat`).
const REPEAT: u32 = 2;

/// How a race went.
struct Run {
    /// Each lap's, in milliseconds.
    laps: Vec<i32>,
    finished: bool,
    grabs: u32,
    boosts: u32,
    ticks: u32,
    stuck: u32,
    slow: u32,
    /// The stuck ticks by the path frame the kart was at, and where it was and what it was
    /// doing the last of them (for the spots, [`print_spots`]).
    stuck_at: std::collections::HashMap<usize, Spot>,
}

/// What [`Run::stuck_at`] keeps of a frame: the stuck ticks there, and of the last of them the
/// kart's place, state and speed, and how it was against the path: off it sideways and up
/// (level units), and its heading against the path's (degrees, 180 the wrong way).
#[derive(Clone, Copy, Default)]
struct Spot {
    ticks: u32,
    pos: [i32; 3],
    state: u8,
    speed: i32,
    side: i64,
    up: i32,
    facing: i32,
}

impl Run {
    fn total(&self) -> Option<i32> {
        self.finished.then(|| self.laps.iter().sum())
    }

    /// The time stuck, as `12.3s`.
    fn stuck(&self) -> String {
        format!("{:.1}s", self.stuck as f32 / 30.0)
    }

    /// The share of the ticks slow, as `4%`.
    fn slow(&self) -> String {
        format!("{:.0}%", self.slow as f32 / self.ticks.max(1) as f32 * 100.0)
    }
}

fn spec(character: usize, bot: bool) -> KartSpec {
    KartSpec {
        character,
        anim_num_frames: [0; 8],
        bot,
        line: None,
    }
}

/// Plays `race` until its kart finishes or the time is up, its pad of each tick from `pad`. How
/// far along it gets is measured on bot path `path`, as in training.
fn drive(
    track: &Track,
    path: usize,
    mut race: Race,
    laps: u8,
    mut pad: impl FnMut(&Race) -> Pad,
) -> Run {
    race.laps = laps;
    let mut run = Run {
        laps: Vec::new(),
        finished: false,
        grabs: 0,
        boosts: 0,
        ticks: 0,
        stuck: 0,
        slow: 0,
        stuck_at: Default::default(),
    };
    let (frames, lengths) = (&track.level.nav_paths[path].frames, &track.lengths[path]);
    let mut tracker = Tracker::find(frames, race.karts[0].driver.pos_curr);
    let mut progress = nn::Progress::new(frames, lengths, &tracker, &race.karts[0].driver);
    while race.tick < MAX_TICKS_PER_LAP * laps as u32 && race.finished == 0 {
        let p = pad(&race);
        let (state, boosts) = {
            let d = &race.karts[0].driver;
            (d.kart_state, d.num_boosts_success)
        };
        race.tick(&track.level, &[p]);
        let d = &race.karts[0].driver;
        run.grabs += (d.kart_state == kart_state::MASK_GRABBED
            && state != kart_state::MASK_GRABBED) as u32;
        // (Each of a drift's; a drift that ends puts the count back to 0.)
        run.boosts += (d.num_boosts_success as i32 - boosts as i32).max(0) as u32;
        let relocated = nn::track(&mut tracker, frames, d);
        let step = progress.step(frames, lengths, &tracker, relocated, d);
        run.ticks += 1;
        run.stuck += step.stalled as u32;
        if step.stalled {
            let p = d.pos_curr;
            let frame = tracker.frame as usize;
            let (side, _) = tracker.offset(frames, p);
            let turn = (d.angle as i32 - nn::path_heading(frames, &tracker, p)).rem_euclid(0x1000);
            let spot = run.stuck_at.entry(frame).or_default();
            spot.ticks += 1;
            spot.pos = [p.x >> 8, p.y >> 8, p.z >> 8];
            spot.state = d.kart_state;
            spot.speed = d.speed_approx as i32;
            spot.side = side;
            spot.up = (p.y >> 8) - frames[frame].pos.y as i32;
            spot.facing = (if turn > 0x800 { turn - 0x1000 } else { turn }) * 360 / 0x1000;
        }
        run.slow += (d.kart_state != kart_state::MASK_GRABBED
            && (d.speed_approx as i32) * 4 < d.stats.speed_class_stat as i32)
            as u32;
    }
    run.laps = race.lap_times[0].clone();
    run.finished = race.finished > 0;
    run
}

/// Above this much time stuck (ticks), [`print_spots`] says where.
const SPOTS_AFTER: u32 = 30 * 5;

/// Where the kart of `run` was stuck the most on `track` (on bot path `path`): the frames, the
/// last place and state there, and the nearest jump's launch (frames ahead, or behind).
fn print_spots(track: &Track, path: usize, run: &Run) {
    if run.stuck <= SPOTS_AFTER {
        return;
    }
    let n = track.level.nav_paths[path].frames.len();
    let mut spots: Vec<_> = run.stuck_at.iter().collect();
    spots.sort_by_key(|(_, s)| std::cmp::Reverse(s.ticks));
    for &(&frame, s) in spots.iter().take(3) {
        let jump = track
            .jumps
            .iter()
            .filter(|&&(p, _)| p == path)
            .map(|&(_, lip)| {
                let ahead = (lip + n - frame) % n;
                if ahead <= n / 2 { ahead as i64 } else { ahead as i64 - n as i64 }
            })
            .min_by_key(|d| d.abs());
        let jump = match jump {
            Some(d) if d >= 0 => format!("a jump {d} frames ahead"),
            Some(d) => format!("a jump {} frames behind", -d),
            None => "no jump on the path".into(),
        };
        println!(
            "        stuck {:5.1}s at frame {frame:4} {:?}, kart_state {}, speed {}, {} off the path \
             sideways and {} up, facing {}° from it; {jump}",
            s.ticks as f32 / 30.0,
            s.pos,
            s.state,
            s.speed,
            s.side,
            s.up,
            s.facing,
        );
    }
}

fn fmt_ms(ms: Option<i32>) -> String {
    match ms {
        Some(ms) => format!("{}:{:02}.{:02}", ms / 60_000, ms / 1000 % 60, ms % 1000 / 10),
        None => "DNF".into(),
    }
}

fn main() -> Result<(), Error> {
    let mut args: HashMap<String, String> = HashMap::new();
    for arg in std::env::args().skip(1) {
        let (k, v) = arg
            .split_once('=')
            .ok_or_else(|| format!("expected key=value, got {arg:?}"))?;
        args.insert(k.to_string(), v.to_string());
    }
    let mut take = |k: &str| args.remove(k);
    let policy_path = PathBuf::from(
        take("policy").unwrap_or_else(|| "target/brain/policy.safetensors".into()),
    );
    let laps: u8 = take("laps").map_or(Ok(3), |v| v.parse())?;
    let character: usize = take("character").map_or(Ok(0), |v| v.parse())?;
    let greedy: bool = take("greedy").map_or(Ok(true), |v| v.parse())?;
    let escapes: bool = take("escapes").map_or(Ok(true), |v| v.parse())?;
    let seed: u64 = take("seed").map_or(Ok(1), |v| v.parse())?;
    let ids: Vec<u32> = match take("tracks") {
        Some(list) => list
            .split(',')
            .map(|s| s.trim().parse())
            .collect::<Result<_, _>>()?,
        None => brain::tracks::RACE_TRACKS.collect(),
    };
    let paths = take("paths");
    let all_paths = paths.as_deref() == Some("all");
    if let Some(k) = args.keys().next() {
        return Err(format!("unknown key {k:?}").into());
    }

    let disc = brain::tracks::disc_path();
    let mut tracks = brain::tracks::load(&disc, ids)
        .map_err(|e| format!("reading the tracks off {}: {e}", disc.display()))?;
    if let Some(list) = paths.as_ref().filter(|_| !all_paths) {
        brain::tracks::keep_paths(&mut tracks, list)?;
    }
    let device = brain::ppo::device();
    let policy = Policy::load(&policy_path, &device)?;
    let mut rng = Rng::new(seed);
    println!(
        "{} ({}), {laps} laps, character {character}\n",
        policy_path.display(),
        match (greedy, escapes) {
            (true, true) => "greedy, drawn when stuck or put back",
            (true, false) => "greedy",
            (false, _) => "sampled",
        },
    );
    println!(
        "{:>5} {:>4} | {:>9} {:>9} {:>6} {:>6} {:>7} {:>5} | {:>9} {:>6} {:>7} {:>5} | {:>7}",
        "track",
        "path",
        "policy",
        "best lap",
        "grabs",
        "boosts",
        "stuck",
        "slow",
        "scripted",
        "grabs",
        "stuck",
        "slow",
        "diff"
    );

    let (mut wins, mut both) = (0, 0);
    // With `paths=all`, the path each track went best on, for `train`'s `paths=`.
    let mut best_paths = Vec::new();
    for track in &tracks {
        let race_paths: Vec<usize> = if all_paths {
            track.paths.clone()
        } else {
            track.paths.first().copied().into_iter().collect()
        };
        if race_paths.is_empty() {
            println!("{:>5} | no bot path", track.id.0);
            continue;
        }
        // Each path's runs, the best kept: finished soonest, or else the fewest grabs, the least
        // time stuck.
        let mut best: Option<(usize, Run, Run)> = None;
        for &path in &race_paths {
            // The policy, steering by this path (another while it is stuck, as in training:
            // `Progress::switch_due`). The time and the stats are measured on this one all the
            // same.
            let race = Race::new(&track.level, track.objects.clone(), &[spec(character, false)], false);
            let mut steer_path = path;
            let frames0 = &track.level.nav_paths[path].frames;
            let mut tracker = Tracker::find(frames0, race.karts[0].driver.pos_curr);
            let mut progress =
                nn::Progress::new(frames0, &track.lengths[path], &tracker, &race.karts[0].driver);
            let mut latch = Latch::default();
            let mut action = nn::Action::default();
            let mut failed = None;
            let learned = drive(track, path, race, laps, |race| {
                let kart = &race.karts[0];
                let frames = &track.level.nav_paths[steer_path].frames;
                let lengths = &track.lengths[steer_path];
                let relocated = nn::track(&mut tracker, frames, &kart.driver);
                progress.step(frames, lengths, &tracker, relocated, &kart.driver);
                if progress.switch_due(&kart.driver)
                    && let Some((other, t)) =
                        nn::next_path(&track.level, steer_path, kart.driver.pos_curr)
                {
                    steer_path = other;
                    tracker = t;
                    let frames = &track.level.nav_paths[other].frames;
                    progress.switched(frames, &track.lengths[other], &tracker, &kart.driver);
                }
                let frames = &track.level.nav_paths[steer_path].frames;
                if race.tick % REPEAT == 0 {
                    let obs = nn::observe(frames, &tracker, &progress, kart, &latch);
                    let likeliest = greedy && !(escapes && nn::draw_choices(&progress));
                    match policy.act(&[obs], likeliest, &mut rng) {
                        Ok(c) => action = c[0].action,
                        Err(e) => failed = Some(e),
                    }
                }
                let d = &kart.driver;
                latch.pad(action, d.kart_state == kart_state::DRIFTING, d.num_boosts_attempted > 0)
            });
            if let Some(e) = failed {
                return Err(e.into());
            }

            // The scripted bot, in the same kart on the same path.
            let scripted_spec = KartSpec {
                line: Some(path as u8),
                ..spec(character, true)
            };
            let race = Race::new(&track.level, track.objects.clone(), &[scripted_spec], false);
            let scripted = drive(track, path, race, laps, |_| Pad::default());

            let diff = match (learned.total(), scripted.total()) {
                (Some(a), Some(b)) => format!("{:+.2}s", (a - b) as f32 / 1000.0),
                _ => "-".into(),
            };
            println!(
                "{:>5} {:>4} | {:>9} {:>9} {:>6} {:>6} {:>7} {:>5} | {:>9} {:>6} {:>7} {:>5} | {:>7}",
                track.id.0,
                path,
                fmt_ms(learned.total()),
                fmt_ms(learned.laps.iter().copied().min()),
                learned.grabs,
                learned.boosts,
                learned.stuck(),
                learned.slow(),
                fmt_ms(scripted.total()),
                scripted.grabs,
                scripted.stuck(),
                scripted.slow(),
                diff,
            );
            print_spots(track, path, &learned);
            let key = |r: &Run| (r.total().is_none(), r.total().unwrap_or(0), r.grabs, r.stuck);
            if best.as_ref().is_none_or(|(_, b, _)| key(&learned) < key(b)) {
                best = Some((path, learned, scripted));
            }
        }
        let Some((path, learned, scripted)) = best else {
            continue;
        };
        if let (Some(a), Some(b)) = (learned.total(), scripted.total()) {
            both += 1;
            wins += (a < b) as u32;
        }
        if race_paths.len() > 1 {
            let diff = match (learned.total(), scripted.total()) {
                (Some(a), Some(b)) => format!(" ({:+.2}s against the scripted bot)", (a - b) as f32 / 1000.0),
                _ => String::new(),
            };
            println!("{:>5} {:>4}  best: {}{diff}\n", track.id.0, path, fmt_ms(learned.total()));
            best_paths.push(format!("{}:{path}", track.id.0));
        }
    }
    if !best_paths.is_empty() {
        println!("the best paths: paths={}", best_paths.join(","));
    }
    println!("\nfaster than the scripted bot on {wins} of the {both} tracks both finished");
    Ok(())
}
