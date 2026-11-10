//! Races a trained policy from the grid on each track, against the scripted bot
//! (`race::bot::Bot`) in the same kart, each alone: the laps' times, the mask grabs and the
//! boosts.
//!
//! `cargo run --release -p brain --bin eval -- [key=value ...]`, the keys being:
//!
//! - `policy` (`target/brain/policy.safetensors`): the checkpoint.
//! - `tracks` (all): the `LevelId`s, as `0,3,7`.
//! - `laps` (3).
//! - `character` (0): in `GAME_CHARACTER_IDS` order.
//! - `greedy` (true): the likeliest choice of each head; false draws them, as in training.
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
}

impl Run {
    fn total(&self) -> Option<i32> {
        self.finished.then(|| self.laps.iter().sum())
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

/// Plays `race` until its kart finishes or the time is up, its pad of each tick from `pad`.
fn drive(track: &Track, mut race: Race, laps: u8, mut pad: impl FnMut(&Race) -> Pad) -> Run {
    race.laps = laps;
    let mut run = Run {
        laps: Vec::new(),
        finished: false,
        grabs: 0,
        boosts: 0,
    };
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
    }
    run.laps = race.lap_times[0].clone();
    run.finished = race.finished > 0;
    run
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
    let seed: u64 = take("seed").map_or(Ok(1), |v| v.parse())?;
    let ids: Vec<u32> = match take("tracks") {
        Some(list) => list
            .split(',')
            .map(|s| s.trim().parse())
            .collect::<Result<_, _>>()?,
        None => brain::tracks::RACE_TRACKS.collect(),
    };
    if let Some(k) = args.keys().next() {
        return Err(format!("unknown key {k:?}").into());
    }

    let disc = brain::tracks::disc_path();
    let tracks = brain::tracks::load(&disc, ids)
        .map_err(|e| format!("reading the tracks off {}: {e}", disc.display()))?;
    let device = brain::ppo::device();
    let policy = Policy::load(&policy_path, &device)?;
    let mut rng = Rng::new(seed);
    println!(
        "{} ({}), {laps} laps, character {character}\n",
        policy_path.display(),
        if greedy { "greedy" } else { "sampled" },
    );
    println!(
        "{:>5} | {:>9} {:>9} {:>6} {:>6} | {:>9} {:>6} | {:>7}",
        "track", "policy", "best lap", "grabs", "boosts", "scripted", "grabs", "diff"
    );

    let (mut wins, mut both) = (0, 0);
    for track in &tracks {
        let Some(&path) = track.paths.first() else {
            println!("{:>5} | no bot path", track.id.0);
            continue;
        };
        let frames = &track.level.nav_paths[path].frames;

        // The policy, steering by the level's first path.
        let race = Race::new(&track.level, track.objects.clone(), &[spec(character, false)], false);
        let lengths = &track.lengths[path];
        let mut tracker = Tracker::find(frames, race.karts[0].driver.pos_curr);
        let mut progress = nn::Progress::new(frames, lengths, &tracker, &race.karts[0].driver);
        let mut latch = Latch::default();
        let mut action = nn::Action::default();
        let mut failed = None;
        let learned = drive(track, race, laps, |race| {
            let kart = &race.karts[0];
            let relocated = nn::track(&mut tracker, frames, &kart.driver);
            progress.step(frames, lengths, &tracker, relocated, &kart.driver);
            if race.tick % REPEAT == 0 {
                let obs = nn::observe(frames, &tracker, &progress, kart, &latch);
                match policy.act(&[obs], greedy, &mut rng) {
                    Ok(c) => action = c[0].action,
                    Err(e) => failed = Some(e),
                }
            }
            latch.pad(action)
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
        let scripted = drive(track, race, laps, |_| Pad::default());

        let diff = match (learned.total(), scripted.total()) {
            (Some(a), Some(b)) => {
                both += 1;
                wins += (a < b) as u32;
                format!("{:+.2}s", (a - b) as f32 / 1000.0)
            }
            _ => "-".into(),
        };
        println!(
            "{:>5} | {:>9} {:>9} {:>6} {:>6} | {:>9} {:>6} | {:>7}",
            track.id.0,
            fmt_ms(learned.total()),
            fmt_ms(learned.laps.iter().copied().min()),
            learned.grabs,
            learned.boosts,
            fmt_ms(scripted.total()),
            scripted.grabs,
            diff,
        );
    }
    println!("\nfaster than the scripted bot on {wins} of the {both} tracks both finished");
    Ok(())
}
