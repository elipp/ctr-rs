//! The command line: Return opens it, typing and pasting (Cmd or Ctrl V) fill it in, Return again
//! sends what is typed and Escape closes it without. A line that starts with `/` is a command
//! (an [`Action`]); any other is a chat message.
//!
//! While a command is typed, what can come next is listed above the line (`suggest.rs`): Up and
//! Down pick one, Tab puts it in the line, and Return does too, sending the line if nothing can
//! follow it. Otherwise Up and Down go through the lines sent before.
//!
//! `/lobby create public` shows a name for the lobby in grey: Up and Down change it, Tab or Return
//! takes it, and typing replaces it. `/character`, `/promote` and `/track` offer the characters,
//! the players and the tracks the same way, with a portrait after the name or the track's picture
//! over the line; typing narrows them down to the names that start with what is typed.
//!
//! `/cup create <name> ` offers the tracks too, by name, and puts in the short name of the one
//! taken: Tab or Right at the end of the line takes it and goes on to the next, Return takes it as
//! the last, and Space after a comma ends the list without it. The laps are typed after that.
//! `/cup start` and `/cup delete` list the cups made, offering them the same way, the latest at
//! first. Return on a name offered sends `/cup start` but only fills in `/cup delete`.
//!
//! The line is edited as in a shell: the arrows, Home and End move the cursor, and Ctrl A, E, B, F
//! (Alt B, F by a word), Ctrl W, U, K, D, H and Alt D delete. Page
//! Up and Down scroll the console (`console.rs`), which the line is the bottom row of.
//!
//! While it is open it takes the keyboard: the keys are cleared from the game's input every frame,
//! so that typing does not drive the kart.
//!
//! On a touch screen a tap on the console opens it, and the page's text field is what the
//! on-screen keyboard types in: the line is replaced with what the field holds (`web/play.html`).

use std::collections::VecDeque;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::text::TextBackgroundColor;
use net::Ticket;

use crate::console::{Console, Log, PAGE, SmallPortrait, small_portrait};
use crate::suggest::{self, Menu, Offer, OfferPicture};
use crate::track_picture::TrackPictures;

/// Something a player can ask for with a `/command`. The command only sends it; what it does is
/// up to whoever handles it (the lobby's are in `lobby.rs`).
#[derive(Message, Clone, Debug)]
pub enum Action {
    /// Open a lobby, and list it in the realm for anyone to find if `public` has the name to list
    /// it under (empty for the default).
    CreateLobby {
        public: Option<String>,
    },
    JoinLobby(Ticket),
    /// Join a lobby to watch its races, not to drive in them.
    SpectateLobby(Ticket),
    /// Watch instead of driving, or drive again, in the lobby we are in (a guest's).
    ToggleSpectator,
    CloseLobby,
    /// Hand the lobby over to the player called that (a nickname or the short key): they host it
    /// from now on, and we are a guest of theirs (the host only, between races).
    Promote(String),
    /// Drive that character (`CHARACTERS` order), as the keys `.` and `,` go to the next.
    Character(u32),
    /// Be on that track (a `LevelId`), as `[` and `]` go to the next. In a lobby it is ours until
    /// a race goes to the host's, and the host's is the one its next race is on.
    Track(u32),
    /// Delete the maps other players sent us (`maps::purge`).
    PurgeCache,
    /// Lists the tracks there are (`/track list`).
    ListTracks,
    StartRace,
    /// Fill the grid of our races with bots (`lobby::BotSettings`): how many, and how hard, or
    /// say what it is with neither.
    Bots {
        count: Option<u8>,
        difficulty: Option<u16>,
    },
    RestartRace,
    /// Define a cup: the tracks it goes through and the laps of each race (`lobby/cup.rs`).
    CupCreate {
        name: String,
        tracks: Vec<u32>,
        laps: u8,
    },
    /// Drive the cup of that name in the lobby (the host only).
    CupStart(String),
    /// Forget the cup of that name.
    CupDelete(String),
    /// End the race in progress for everyone, with no winner (the host only).
    StopRace,
    /// (Debug) Roll that item into our hand: one of `vehicle::item::ENABLED`.
    DebugAddWeapon(u8),
    /// (Debug) Set our wumpa count, clamped to what a kart can hold.
    DebugSetWumpa(u8),
    /// (Debug) Freeze this client for that long, online only: the others wait for it, and the
    /// host removes it after `net::KICK_AFTER`.
    DebugStall(std::time::Duration),
    /// (Debug) Hold back the pads of the others that come to this client that long (`None` to
    /// stop): see `lobby/lag.rs`.
    DebugLag(Option<crate::lobby::lag::Lag>),
    /// (Debug) End the race now, as if the karts had finished in a random order (the host's;
    /// a cup scores it).
    DebugRanking,
    /// (Debug) Switch between the race and the fly-through viewer with the collision probe
    /// (`main.rs`).
    DebugViewer,
    /// (Debug) Show or hide the development text of the race (position, speed, reserves), or
    /// toggle it (`None`). The frame rate stays.
    DebugHud(Option<bool>),
    /// (Debug) Show that text as a toast, to see how it is written.
    DebugToast(String),
    /// (Debug) Change how the level is built, or toast how it is (`None`).
    DebugLevel(Option<MeshSetting>),
    /// (Debug) Set the depth bias of the crates, or toast it (`None`).
    DebugCrateBias(Option<f32>),
    /// (Debug) Try a visual effect, or open the panel of them (`fx.rs`).
    DebugFx(crate::fx::Fx),
    /// (Debug) Put that many dummy karts on the grid with ours, of random characters, offline
    /// (`remote.rs`); 0 takes them away.
    DebugSpawnKarts(u8),
    /// Toast how each peer is connected: direct or by relay, the round trip time, losses.
    NetInfo,
    /// Set how the drawing eases the karts of the others after a rollback.
    NetSmoothing(crate::kart::Smoothing),
    /// Show a toast for each rollback, or stop.
    NetRollbacks(bool),
    /// Change a setting (`config.rs` does it).
    SetConfig(crate::config::Setting),
    /// Toast the version of this build (`version.rs`).
    Version,
    /// Empty the console (`console.rs`). What was typed before stays in the history.
    Clear,
    /// Say that to everyone in the lobby: a line that is not a `/command`.
    Say(String),
}

/// How the level's mesh is built (`/debug level`).
#[derive(Clone, Copy, Debug)]
pub enum MeshSetting {
    /// Keep the blocks without textures and the faces with a pure red corner.
    Hidden(bool),
    /// Cull the back faces, as the game does.
    Cull(bool),
    /// The `color_lo` vertex colors instead of `color_hi`.
    LowColors(bool),
    /// Draw the crates that the level places in the ground where `props::crate_fixup` puts them.
    CrateFixup(bool),
}

/// The first word of `text` and what follows it.
pub(crate) fn word(text: &str) -> (&str, &str) {
    let text = text.trim();
    text.split_once(char::is_whitespace)
        .map_or((text, ""), |(w, rest)| (w, rest.trim()))
}

const LOBBY_USAGE: &str =
    "Usage: /lobby create [private | public [name]] | join <address> | spectate [address] | close";
const CONFIG_USAGE: &str = "Usage: /config set <setting> <value>, like /config set nickname Penta";
const PROMOTE_USAGE: &str = "Usage: /promote <player>, to make them the host of the lobby";
const CHARACTER_USAGE: &str = "Usage: /character <name>";
const TRACK_USAGE: &str = "Usage: /track <name> | list";
const CACHE_USAGE: &str = "Usage: /cache purge";
const RACE_USAGE: &str = "Usage: /race start | restart | stop";
const BOTS_USAGE: &str = "Usage: /bots [0 to 7] [easy | medium | hard], or /bots off";
const CUP_USAGE: &str = "Usage: /cup create <name> <track,track,...> <laps> | start <name> | delete <name>\nA track is its name, its short name (dingo_canyon), or its number (/track list)";
const DEBUG_USAGE: &str = "Usage: /debug set wumpa <0 to 10> | set weapon <0 to 4 or 6 to 11> | stall <duration, like 3s> | lag <delay> [jitter] | lag off | viewer | ranking | hud [on|off] | toast <text> | level hidden|cull|colors|crate-fixup | crate bias <number> | fx [...] | spawn-karts <0 to 7>";
const NET_USAGE: &str =
    "Usage: /net info | smoothing ping|replay|<duration, like 120ms> | rollbacks on|off | relay add <url> | relay clear | relay only on|off";

/// The most dummy karts there can be: the grid has 8 places, and ours is one.
const DUMMY_KARTS_MAX: u8 = 7;

/// The longest stall that can be set.
const STALL_MAX_MS: f64 = 60_000.0;

/// The longest smoothing that can be set.
const SMOOTHING_MAX_MS: f64 = 1000.0;

/// A time as milliseconds: `120ms`, `0.12s`, or a bare number of milliseconds.
fn milliseconds(text: &str) -> Option<f64> {
    let (number, to_ms) = match (text.strip_suffix("ms"), text.strip_suffix('s')) {
        (Some(n), _) => (n, 1.0),
        (None, Some(n)) => (n, 1000.0),
        _ => (text, 1.0),
    };
    number.trim().parse::<f64>().ok().map(|n| n * to_ms)
}

impl Action {
    /// The action a line without its `/` asks for, or what is wrong with it. Commands are a name
    /// and a subcommand (`/lobby join <address>`), which may have subcommands of its own
    /// (`/debug set wumpa 10`).
    pub(crate) fn parse(line: &str) -> Result<Action, String> {
        let (name, rest) = word(line);
        match name {
            "lobby" => Action::lobby(rest),
            "race" => Action::race(rest),
            "bots" => Action::bots(rest),
            "cup" => Action::cup(rest),
            "promote" if rest.is_empty() => Err(PROMOTE_USAGE.into()),
            "promote" => Ok(Action::Promote(rest.into())),
            "character" if rest.is_empty() => Err(CHARACTER_USAGE.into()),
            "character" => crate::CHARACTERS
                .iter()
                .position(|name| name.eq_ignore_ascii_case(rest))
                .map(|character| Action::Character(character as u32))
                .ok_or_else(|| {
                    format!(
                        "No character is called {rest}: {}",
                        crate::CHARACTERS.join(", ")
                    )
                }),
            "track" => Action::track(rest),
            "cache" => Action::cache(rest),
            "config" => Action::config(rest),
            "version" if rest.is_empty() => Ok(Action::Version),
            "version" => Err("Usage: /version".into()),
            "clear" if rest.is_empty() => Ok(Action::Clear),
            "clear" => Err("Usage: /clear".into()),
            "net" => Action::net(rest),
            "debug" => Action::debug(rest),
            _ => Err(format!("Unknown command: /{name}")),
        }
    }

    fn lobby(line: &str) -> Result<Action, String> {
        let (sub, rest) = word(line);
        match sub {
            "create" => match word(rest) {
                // (A private lobby is never listed: the realm is not contacted at all.)
                ("" | "private", "") => Ok(Action::CreateLobby { public: None }),
                ("public", name) => Ok(Action::CreateLobby {
                    public: Some(net::lobby_name(name)?),
                }),
                _ => Err(LOBBY_USAGE.into()),
            },
            "join" => rest
                .parse()
                .map(Action::JoinLobby)
                .map_err(|e| format!("Not a lobby address: {e}")),
            "spectate" if rest.is_empty() => Ok(Action::ToggleSpectator),
            "spectate" => rest
                .parse()
                .map(Action::SpectateLobby)
                .map_err(|e| format!("Not a lobby address: {e}")),
            "close" => Ok(Action::CloseLobby),
            "" => Err(LOBBY_USAGE.into()),
            _ => Err(format!("Unknown command: /lobby {sub}\n{LOBBY_USAGE}")),
        }
    }

    fn net(line: &str) -> Result<Action, String> {
        let (sub, rest) = word(line);
        match sub {
            "info" => Ok(Action::NetInfo),
            "smoothing" => Action::smoothing(rest),
            "rollbacks" => match rest {
                "on" => Ok(Action::NetRollbacks(true)),
                "off" => Ok(Action::NetRollbacks(false)),
                _ => Err("Usage: /net rollbacks on|off".into()),
            },
            "relay" => match word(rest) {
                ("add", url) if !url.is_empty() => net::parse_relay(url)
                    .map(|url| Action::SetConfig(crate::config::Setting::AddRelay(url))),
                ("clear", "") => Ok(Action::SetConfig(crate::config::Setting::ClearRelays)),
                ("only", _) if cfg!(target_arch = "wasm32") => {
                    Err("In a browser, every connection goes through the relay".into())
                }
                ("only", "on") => Ok(Action::SetConfig(crate::config::Setting::RelayOnly(true))),
                ("only", "off") => Ok(Action::SetConfig(crate::config::Setting::RelayOnly(false))),
                _ => Err("Usage: /net relay add <url, like relay.example.com> | clear | only on|off".into()),
            },
            "" => Err(NET_USAGE.into()),
            _ => Err(format!("Unknown command: /net {sub}\n{NET_USAGE}")),
        }
    }

    /// `dynamic`, or a time: `120ms`, `0.12s`, or a number of milliseconds.
    fn smoothing(text: &str) -> Result<Action, String> {
        use crate::kart::Smoothing;
        // (`dynamic` was the name of `ping`.)
        match text {
            "ping" | "dynamic" => return Ok(Action::NetSmoothing(Smoothing::Ping)),
            "replay" => return Ok(Action::NetSmoothing(Smoothing::Replay)),
            _ => {}
        }
        match milliseconds(text) {
            Some(ms) if (0.0..=SMOOTHING_MAX_MS).contains(&ms) => Ok(Action::NetSmoothing(
                crate::kart::Smoothing::Fixed(std::time::Duration::from_secs_f64(ms / 1000.0)),
            )),
            _ => Err(format!(
                "Smoothing is ping, replay or 0 to {SMOOTHING_MAX_MS:.0} ms\n{NET_USAGE}"
            )),
        }
    }

    fn config(line: &str) -> Result<Action, String> {
        let (sub, rest) = word(line);
        match sub {
            "set" => crate::config::Setting::parse(rest).map(Action::SetConfig),
            "" => Err(CONFIG_USAGE.into()),
            _ => Err(format!("Unknown command: /config {sub}\n{CONFIG_USAGE}")),
        }
    }

    /// `list`, or the track to be on. (So a custom track called "list" goes by its number.)
    fn track(line: &str) -> Result<Action, String> {
        match line {
            "" => Err(TRACK_USAGE.into()),
            "list" => Ok(Action::ListTracks),
            track => crate::maps::resolve(track).map(Action::Track),
        }
    }

    fn cache(line: &str) -> Result<Action, String> {
        let (sub, _) = word(line);
        match sub {
            "purge" => Ok(Action::PurgeCache),
            "" => Err(CACHE_USAGE.into()),
            _ => Err(format!("Unknown command: /cache {sub}\n{CACHE_USAGE}")),
        }
    }

    fn bots(line: &str) -> Result<Action, String> {
        use vehicle::bot::difficulty_level::{EASY, HARD, MEDIUM};
        let (mut count, mut difficulty) = (None, None);
        for word in line.split_whitespace() {
            match word {
                "off" => count = Some(0),
                "easy" => difficulty = Some(EASY as u16),
                "medium" => difficulty = Some(MEDIUM as u16),
                "hard" => difficulty = Some(HARD as u16),
                _ => match word.parse::<u8>() {
                    Ok(n) if n < race::MAX_KARTS as u8 => count = Some(n),
                    _ => return Err(BOTS_USAGE.into()),
                },
            }
        }
        Ok(Action::Bots { count, difficulty })
    }

    fn race(line: &str) -> Result<Action, String> {
        let (sub, _) = word(line);
        match sub {
            "start" => Ok(Action::StartRace),
            "restart" => Ok(Action::RestartRace),
            "stop" => Ok(Action::StopRace),
            "" => Err(RACE_USAGE.into()),
            _ => Err(format!("Unknown command: /race {sub}\n{RACE_USAGE}")),
        }
    }

    fn cup(line: &str) -> Result<Action, String> {
        use crate::lobby::cup::{MAX_LAPS, MAX_NAME, MAX_TRACKS, valid_name};
        let (sub, rest) = word(line);
        match sub {
            "create" => {
                // (A track's name can have spaces in it, so the laps are found from the end.)
                let (name, rest) = word(rest);
                let Some((tracks, laps)) = rest.trim_end().rsplit_once(char::is_whitespace) else {
                    return Err(CUP_USAGE.into());
                };
                if !valid_name(name) {
                    return Err(format!(
                        "A cup's name is letters, digits, - and _, up to {MAX_NAME}"
                    ));
                }
                // (The comma left by taking a track, if the laps were typed over the next.)
                let tracks = tracks
                    .trim_end()
                    .strip_suffix(',')
                    .unwrap_or(tracks)
                    .split(',')
                    .map(crate::maps::resolve)
                    .collect::<Result<Vec<u32>, String>>()?;
                if tracks.len() > MAX_TRACKS {
                    return Err(format!("A cup has {MAX_TRACKS} tracks at most"));
                }
                let laps = laps
                    .parse::<u8>()
                    .ok()
                    .filter(|n| (1..=MAX_LAPS).contains(n))
                    .ok_or_else(|| format!("The laps are 1 to {MAX_LAPS}"))?;
                Ok(Action::CupCreate {
                    name: name.into(),
                    tracks,
                    laps,
                })
            }
            "start" if rest.split_whitespace().count() == 1 => Ok(Action::CupStart(rest.into())),
            "delete" if rest.split_whitespace().count() == 1 => Ok(Action::CupDelete(rest.into())),
            "" | "start" | "delete" => Err(CUP_USAGE.into()),
            _ => Err(format!("Unknown command: /cup {sub}\n{CUP_USAGE}")),
        }
    }

    /// `hidden on|off`, `cull on|off`, `colors hi|lo`, `crate-fixup on|off`, or nothing.
    fn level(line: &str) -> Result<Action, String> {
        const USAGE: &str =
            "Usage: /debug level hidden on|off | cull on|off | colors hi|lo | crate-fixup on|off";
        let (what, value) = word(line);
        let on_off = || match value {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => Err(USAGE.to_string()),
        };
        Ok(Action::DebugLevel(match what {
            "" => None,
            "hidden" => Some(MeshSetting::Hidden(on_off()?)),
            "cull" => Some(MeshSetting::Cull(on_off()?)),
            "crate-fixup" => Some(MeshSetting::CrateFixup(on_off()?)),
            "colors" => Some(MeshSetting::LowColors(match value {
                "hi" => false,
                "lo" => true,
                _ => return Err(USAGE.into()),
            })),
            _ => return Err(format!("Unknown command: /debug level {what}\n{USAGE}")),
        }))
    }

    /// Nothing (the panel), `confetti auto|on|off`, `flash`, `finish`, `speed <factor>`,
    /// `freeze on|off`, `step`, `weather on|off` or `rain on|off`.
    fn fx(line: &str) -> Result<crate::fx::Fx, String> {
        use crate::fx::{Force, Fx, SPEED_RANGE};
        const USAGE: &str = "Usage: /debug fx [confetti auto|on|off | flash | finish | speed <factor> | freeze on|off | step | weather on|off | rain on|off]";
        let (what, value) = word(line);
        let on_off = || match value {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => Err(USAGE.to_string()),
        };
        Ok(match what {
            "" => Fx::Panel,
            "confetti" => Fx::Confetti(match value {
                "auto" => Force::Auto,
                "on" => Force::On,
                "off" => Force::Off,
                _ => return Err(USAGE.into()),
            }),
            "flash" => Fx::Flash,
            "finish" => Fx::Finish,
            "speed" => match value.parse::<f32>() {
                Ok(speed) if (SPEED_RANGE.0..=SPEED_RANGE.1).contains(&speed) => Fx::Speed(speed),
                _ => {
                    return Err(format!(
                        "A speed is {} to {}, 1 being the game's\n{USAGE}",
                        SPEED_RANGE.0, SPEED_RANGE.1
                    ));
                }
            },
            "freeze" => Fx::Freeze(on_off()?),
            "step" => Fx::Step,
            "weather" => Fx::Weather(on_off()?),
            "rain" => Fx::BeakerRain(on_off()?),
            _ => return Err(format!("Unknown command: /debug fx {what}\n{USAGE}")),
        })
    }

    /// `/debug lag 75ms 15ms`: the delay, and the jitter (none if left out); `off` or 0 for none.
    fn lag(line: &str) -> Result<Action, String> {
        use crate::lobby::lag::{Lag, MAX_DELAY_MS, MAX_JITTER_MS};
        let usage = || {
            format!(
                "Usage: /debug lag <delay> [jitter] | off: a delay of 0 to {MAX_DELAY_MS} ms and a jitter of 0 to {MAX_JITTER_MS} ms, like 75ms 15ms"
            )
        };
        let (delay, jitter) = word(line);
        if delay == "off" && jitter.is_empty() {
            return Ok(Action::DebugLag(None));
        }
        let delay = milliseconds(delay).filter(|ms| (0.0..=MAX_DELAY_MS).contains(ms));
        let jitter = match jitter {
            "" => Some(0.0),
            j => milliseconds(j).filter(|ms| (0.0..=MAX_JITTER_MS).contains(ms)),
        };
        let (Some(delay), Some(jitter)) = (delay, jitter) else {
            return Err(usage());
        };
        if delay == 0.0 && jitter == 0.0 {
            return Ok(Action::DebugLag(None));
        }
        Ok(Action::DebugLag(Some(Lag {
            delay: std::time::Duration::from_secs_f64(delay / 1000.0),
            jitter: std::time::Duration::from_secs_f64(jitter / 1000.0),
        })))
    }

    fn debug(line: &str) -> Result<Action, String> {
        let (sub, rest) = word(line);
        match sub {
            "set" => {
                let (what, value) = word(rest);
                match what {
                    "wumpa" => {
                        value
                            .parse::<i64>()
                            .map(|n| {
                                Action::DebugSetWumpa(
                                    n.clamp(0, vehicle::wumpa::MAX_COUNT as i64) as u8
                                )
                            })
                            .map_err(|_| "Usage: /debug set wumpa <number>".to_string())
                    }
                    "weapon" => value
                        .parse()
                        .ok()
                        .filter(|n| vehicle::item::ENABLED.contains(n))
                        .map(Action::DebugAddWeapon)
                        .ok_or_else(|| {
                            "Usage: /debug set weapon <0 to 4 or 6 to 11>".to_string()
                        }),
                    "" => Err(DEBUG_USAGE.into()),
                    _ => Err(format!("Unknown command: /debug set {what}\n{DEBUG_USAGE}")),
                }
            }
            "stall" => match milliseconds(rest) {
                Some(ms) if (0.0..=STALL_MAX_MS).contains(&ms) => Ok(Action::DebugStall(
                    std::time::Duration::from_secs_f64(ms / 1000.0),
                )),
                _ => Err(format!(
                    "A stall is 0 to {} s, like 3s or 500ms\n{DEBUG_USAGE}",
                    STALL_MAX_MS / 1000.0
                )),
            },
            "lag" => Action::lag(rest),
            "viewer" => Ok(Action::DebugViewer),
            "ranking" => Ok(Action::DebugRanking),
            "hud" => match rest {
                "" => Ok(Action::DebugHud(None)),
                "on" => Ok(Action::DebugHud(Some(true))),
                "off" => Ok(Action::DebugHud(Some(false))),
                _ => Err(DEBUG_USAGE.into()),
            },
            "toast" if !rest.is_empty() => Ok(Action::DebugToast(rest.to_string())),
            "toast" => Err("Usage: /debug toast <text>".into()),
            "level" => Action::level(rest),
            "fx" => Action::fx(rest).map(Action::DebugFx),
            "spawn-karts" => rest
                .parse::<u8>()
                .ok()
                .filter(|&n| n <= DUMMY_KARTS_MAX)
                .map(Action::DebugSpawnKarts)
                .ok_or_else(|| format!("Usage: /debug spawn-karts <0 to {DUMMY_KARTS_MAX}>")),
            "crate" => {
                const USAGE: &str = "Usage: /debug crate bias [number]";
                let (what, value) = word(rest);
                match (what, value) {
                    ("bias", "") => Ok(Action::DebugCrateBias(None)),
                    ("bias", v) => v
                        .parse::<f32>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .map(|v| Action::DebugCrateBias(Some(v)))
                        .ok_or_else(|| USAGE.to_string()),
                    _ => Err(USAGE.to_string()),
                }
            }
            "" => Err(DEBUG_USAGE.into()),
            _ => Err(format!("Unknown command: /debug {sub}\n{DEBUG_USAGE}")),
        }
    }
}

const FONT_SIZE: f32 = 20.0;

/// The seconds a cursor stays on, and off.
const BLINK_SECONDS: f32 = 0.5;

/// How many sent lines the history keeps.
const HISTORY: usize = 100;

/// What the line offers names of (`CommandLine::wanted`).
#[derive(Clone, Copy)]
enum Offering {
    Character,
    Player,
    Track,
    Cup,
}

/// A track the line offers.
struct OfferedTrack {
    name: String,
    /// What a cup's line puts in for it (`maps::stable_name`).
    command_name: String,
    id: u32,
}

/// A cup the line offers.
struct OfferedCup {
    name: String,
    /// The `LevelId` it starts on, none if that track is gone.
    first_track: Option<u32>,
    /// What its row says of it (`cup_about`).
    about: String,
}

/// What has been typed so far, if the line is open.
#[derive(Resource, Default)]
pub(crate) struct CommandLine {
    open: bool,
    text: String,
    /// Return was pressed: `submit` sends the line.
    entered: bool,
    /// The lines sent before, the latest last.
    history: VecDeque<String>,
    /// The line of `history` that the text is, while the arrows are browsing it.
    browsing: Option<usize>,
    /// What was typed before the browsing started: the newest option after the history's.
    draft: String,
    /// The suggestion picked, of those listed for the text.
    selected: usize,
    /// Up or Down went through the suggestions since the text changed.
    moved_selection: bool,
    /// Which name a public lobby is offered (`suggest::lobby_name_at`), new each time the line
    /// opens.
    lobby_name_index: u64,
    /// The name to offer if the line can come to it (`suggest::pick`), of each [`Offering`]: at
    /// first, the character we drive, the track we are on and the cup made last.
    wanted: [String; 4],
    /// Who `/promote` offers: each one's name and character.
    players: Vec<(String, usize)>,
    /// What `/track` and `/cup create` offer.
    tracks: Vec<OfferedTrack>,
    /// What `/cup start` and `/cup delete` offer.
    cups: Vec<OfferedCup>,
    /// Where the cursor is in the text: a byte offset, always at a character.
    cursor: usize,
}

impl OfferedTrack {
    fn offer(&self) -> Offer<'_> {
        Offer {
            name: &self.name,
            picture: OfferPicture::Track(self.id),
        }
    }
}

impl CommandLine {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Up: the line sent before the one shown.
    fn older(&mut self) {
        let index = match self.browsing {
            Some(0) => return,
            Some(i) => i - 1,
            None if self.history.is_empty() => return,
            None => {
                self.draft = std::mem::take(&mut self.text);
                self.history.len() - 1
            }
        };
        self.set_text(self.history[index].clone());
        self.browsing = Some(index);
    }

    /// Down: the line sent after the one shown, and past the last, what was being typed.
    fn newer(&mut self) {
        let Some(i) = self.browsing else { return };
        if i + 1 < self.history.len() {
            self.set_text(self.history[i + 1].clone());
            self.browsing = Some(i + 1);
        } else {
            let draft = std::mem::take(&mut self.draft);
            self.set_text(draft);
        }
    }

    /// Typing edits what is shown, which is the text from then on.
    fn touch(&mut self) {
        self.browsing = None;
        self.selected = 0;
        self.moved_selection = false;
    }

    /// Replaces the text, with the cursor after it.
    fn set_text(&mut self, text: String) {
        self.touch();
        self.cursor = text.len();
        self.text = text;
    }

    /// Types at the cursor.
    fn insert(&mut self, typed: &str) {
        if typed == " " && self.end_cup_tracks() {
            return;
        }
        self.touch();
        self.text.insert_str(self.cursor, typed);
        self.cursor += typed.len();
    }

    /// Takes out that part of the text, leaving the cursor where it began.
    fn remove(&mut self, range: std::ops::Range<usize>) {
        self.touch();
        self.cursor = range.start;
        self.text.drain(range);
    }

    /// The offset of the character before the cursor.
    fn before(&self) -> usize {
        self.text[..self.cursor]
            .chars()
            .next_back()
            .map_or(0, |c| self.cursor - c.len_utf8())
    }

    /// The offset of the character after the cursor.
    fn after(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    /// The start of the word before the cursor (words are what spaces separate).
    fn word_start(&self) -> usize {
        let head = self.text[..self.cursor].trim_end();
        head.rfind(char::is_whitespace).map_or(0, |i| {
            i + head[i..].chars().next().map_or(1, char::len_utf8)
        })
    }

    /// The end of the word after the cursor.
    fn word_end(&self) -> usize {
        let tail = &self.text[self.cursor..];
        let skipped = tail.len() - tail.trim_start().len();
        let word = &tail[skipped..];
        self.cursor + skipped + word.find(char::is_whitespace).unwrap_or(word.len())
    }

    /// Backspace.
    fn backspace(&mut self) {
        self.remove(self.before()..self.cursor);
    }

    /// Delete.
    fn delete(&mut self) {
        self.remove(self.cursor..self.after());
    }

    /// Shell keys with Ctrl or Alt held: true if `code` was one of them.
    fn shortcut(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> bool {
        let end = self.text.len();
        match code {
            KeyCode::KeyA if ctrl => self.cursor = 0,
            KeyCode::KeyE if ctrl => self.cursor = end,
            KeyCode::KeyB if ctrl => self.cursor = self.before(),
            KeyCode::KeyF if ctrl => self.cursor = self.after(),
            KeyCode::KeyB if alt => self.cursor = self.word_start(),
            KeyCode::KeyF if alt => self.cursor = self.word_end(),
            KeyCode::KeyH if ctrl => self.backspace(),
            KeyCode::KeyD if ctrl => self.delete(),
            KeyCode::KeyD if alt => self.remove(self.cursor..self.word_end()),
            KeyCode::KeyW if ctrl => self.remove(self.word_start()..self.cursor),
            KeyCode::KeyU if ctrl => self.remove(0..self.cursor),
            KeyCode::KeyK if ctrl => self.remove(self.cursor..end),
            _ => return false,
        }
        true
    }

    /// The suggestions for the text, unless it is a line of the history being browsed.
    fn menu(&self) -> Option<Menu> {
        self.browsing
            .is_none()
            .then(|| Menu::of(&self.text))
            .flatten()
    }

    /// The suggestion picked, if there are any to pick.
    fn selection(&self) -> Option<(Menu, usize)> {
        let menu = self.menu().filter(|m| !m.options.is_empty())?;
        let index = self.selected.min(menu.options.len() - 1);
        Some((menu, index))
    }

    /// Up or Down through the suggestions, round from the last to the first: false if there are
    /// none.
    fn step(&mut self, down: bool) -> bool {
        let Some((menu, index)) = self.selection() else {
            return false;
        };
        let count = menu.options.len();
        let next = if down { index + 1 } else { index + count - 1 };
        self.selected = next % count;
        self.moved_selection = true;
        true
    }

    /// Whether the line is a command that is understood as it is.
    fn is_command(&self) -> bool {
        self.text
            .strip_prefix('/')
            .is_some_and(|command| Action::parse(command).is_ok())
    }

    /// Puts the suggestion picked in the line: true if nothing can follow it.
    fn complete(&mut self) -> bool {
        let Some((line, done)) = self
            .selection()
            .and_then(|(menu, index)| menu.pick(&self.text, index))
        else {
            return false;
        };
        self.set_text(line);
        done
    }

    /// The name of a public lobby to show in grey, with a space if the line needs one.
    fn lobby_ghost(&self) -> Option<String> {
        suggest::name_to_add(&self.text, &suggest::lobby_name_at(self.lobby_name_index))
    }

    /// Up or Down through the names offered, so that Up goes back to the last: false if none is
    /// shown.
    fn cycle_lobby_name(&mut self, down: bool) -> bool {
        if self.lobby_ghost().is_none() {
            return false;
        }
        let step = if down {
            1
        } else {
            suggest::LOBBY_NAME_COUNT - 1
        };
        self.lobby_name_index = (self.lobby_name_index + step) % suggest::LOBBY_NAME_COUNT;
        true
    }

    /// What the line offers, what it has typed of the name, and the offers it can still come to.
    fn offers(&self) -> Option<(Offering, &str, Vec<Offer<'_>>)> {
        if let Some(typed) = suggest::typed_after(&self.text, "/character") {
            let found = suggest::matching(typed, suggest::characters());
            return Some((Offering::Character, typed, found));
        }
        if let Some(typed) = suggest::typed_after(&self.text, "/promote") {
            let players = self.players.iter().map(|(name, character)| Offer {
                name,
                picture: OfferPicture::Portrait(*character),
            });
            return Some((Offering::Player, typed, suggest::matching(typed, players)));
        }
        if let Some(typed) = suggest::cup_name_typed(&self.text) {
            // (Its picture is of the race it starts with.)
            let cups = self.cups.iter().map(|cup| Offer {
                name: &cup.name,
                picture: cup.first_track.map_or(OfferPicture::Nothing, OfferPicture::Track),
            });
            return Some((Offering::Cup, typed, suggest::matching(typed, cups)));
        }
        if let Some(typed) = suggest::cup_track_typed(&self.text) {
            let tracks = self
                .tracks
                .iter()
                .filter(|t| {
                    suggest::starts_with(&t.name, typed) || suggest::starts_with(&t.command_name, typed)
                })
                .map(OfferedTrack::offer);
            return Some((Offering::Track, typed, suggest::each_name_once(tracks)));
        }
        let typed = suggest::typed_after(&self.text, "/track")?;
        let tracks = self.tracks.iter().map(OfferedTrack::offer);
        Some((Offering::Track, typed, suggest::matching(typed, tracks)))
    }

    /// The one the line is about to name, or names.
    fn offer(&self) -> Option<Offer<'_>> {
        let (offering, typed, offers) = self.offers()?;
        suggest::pick(&offers, &self.wanted[offering as usize], typed)
    }

    /// What to show in grey after the line, with a space if the line needs one.
    fn ghost(&self) -> Option<String> {
        self.lobby_ghost().or_else(|| {
            let offer = self.offer()?;
            let (_, typed, _) = self.offers()?;
            let spelled = if suggest::starts_with(offer.name, typed) {
                offer.name
            } else {
                &self.offered_track(offer)?.command_name
            };
            suggest::rest_of_name(&self.text, typed, spelled)
        })
    }

    /// Up or Down through the offers the line can come to: false if there is no other, or the line
    /// is one of the history's.
    fn cycle_offer(&mut self, down: bool) -> bool {
        let next = self.offers().and_then(|(offering, typed, offers)| {
            let current = suggest::pick(&offers, &self.wanted[offering as usize], typed)?;
            let at = offers.iter().position(|o| o.name == current.name)?;
            let step = if down { 1 } else { offers.len() - 1 };
            let name = offers[(at + step) % offers.len()].name.to_string();
            (offers.len() > 1).then_some((offering, name))
        });
        match next.filter(|_| self.browsing.is_none()) {
            Some((offering, name)) => {
                self.wanted[offering as usize] = name;
                true
            }
            None => false,
        }
    }

    /// Takes the name shown in grey: false if there is none.
    fn accept_ghost(&mut self) -> bool {
        let Some(ghost) = self.ghost() else {
            return false;
        };
        self.set_text(format!("{}{ghost}", self.text));
        true
    }

    /// Whether the line is choosing a track of a cup, with one offered or named.
    fn on_cup_track(&self) -> bool {
        self.browsing.is_none()
            && suggest::cup_track_typed(&self.text).is_some()
            && self.offer().is_some()
    }

    fn offered_track(&self, offer: Offer) -> Option<&OfferedTrack> {
        let OfferPicture::Track(id) = offer.picture else {
            return None;
        };
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Replaces what is typed of the track of a cup with the command name of the one offered, and
    /// `end` after it. Returns where that track is in the list: none if the line is not choosing
    /// one.
    fn take_cup_track(&mut self, end: &str) -> Option<usize> {
        if !self.on_cup_track() {
            return None;
        }
        let track = self.offered_track(self.offer()?)?;
        let id = track.id;
        let command_name = track.command_name.clone();
        let typed = suggest::cup_track_typed(&self.text)?.len();
        let kept = self.text.len() - typed;
        self.set_text(format!("{}{command_name}{end}", &self.text[..kept]));
        self.tracks.iter().position(|t| t.id == id)
    }

    /// Takes the track of a cup that is offered and goes on to the next, offering the one after it
    /// in the list: false if the line is not choosing one.
    fn next_cup_track(&mut self) -> bool {
        let Some(at) = self.take_cup_track(",") else {
            return false;
        };
        let next = (at + 1) % self.tracks.len();
        self.wanted[Offering::Track as usize] = self.tracks[next].name.clone();
        true
    }

    /// Space after the comma of a cup's list, at the end of the line: the list ends without the
    /// track offered, and the laps come next. False if the line is not there.
    fn end_cup_tracks(&mut self) -> bool {
        let at_next_track = self.browsing.is_none()
            && self.cursor == self.text.len()
            && self.text.ends_with(',')
            && suggest::cup_track_typed(&self.text) == Some("");
        if at_next_track {
            let text = format!("{} ", self.text.trim_end_matches(','));
            self.set_text(text);
        }
        at_next_track
    }

    /// Return. A line that is a whole command is sent as it is (`/lobby create`, which an option
    /// could follow but does not have to), unless Up or Down picked an option; otherwise a
    /// suggestion picked goes in first, and the line is sent if that ends it.
    fn enter(&mut self) {
        if self.take_cup_track(" ").is_some() {
            return;
        }
        // Deleting cannot be undone: Return shows the whole name, and a second one sends it.
        if suggest::typed_after(&self.text, "/cup delete").is_some() && self.accept_ghost() {
            return;
        }
        if self.ghost_goes_first() {
            self.accept_ghost();
            self.entered = true;
            return;
        }
        if self.selection().is_none()
            || (!self.moved_selection && self.is_command())
            || self.complete()
        {
            self.accept_ghost();
            self.entered = true;
        }
    }

    /// A name offered in grey is what Return takes, not an option that can go there too (`list`
    /// of `/track `), unless Up or Down picked that. A command still without its space stops at
    /// it first, to show the name.
    fn ghost_goes_first(&self) -> bool {
        !self.moved_selection
            && self.text.contains(char::is_whitespace)
            && self.offer().is_some()
            && self.ghost().is_some()
    }

    /// Keeps the line that was sent, unless it is the same as the last.
    fn remember(&mut self, sent: &str) {
        self.browsing = None;
        self.draft.clear();
        if self.history.back().is_none_or(|last| last != sent) {
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(sent.to_string());
        }
    }
}

/// The line on screen: `> `, the text before the cursor, ...
#[derive(Component)]
pub(crate) struct CommandText;

/// ... the character under the cursor (a blank at the end), blinking ...
#[derive(Component)]
struct CursorSpan;

/// ... and the rest.
#[derive(Component)]
struct AfterSpan;

/// ... and the name offered for a public lobby.
#[derive(Component)]
struct GhostSpan;

/// The line and the suggestions over it, which are shown only while the line is open.
#[derive(Component)]
struct InputRow;

/// The suggestions over the line.
#[derive(Component)]
struct Suggestions;

/// The panel over them with the picture of the track the line offers ...
#[derive(Component)]
struct TrackPanel;

/// ... and the picture.
#[derive(Component)]
struct TrackImage;

/// The portrait of who the line offers, after the name.
#[derive(Component)]
struct OfferIcon;

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandLine>()
            .add_message::<Action>()
            .add_systems(PreUpdate, capture.after(InputSystems))
            .add_systems(Update, (open, submit, show, show_offer_picture).chain());
        #[cfg(target_arch = "wasm32")]
        app.add_systems(Update, tell_page.after(submit));
    }
}

/// Which of the modifier keys are held.
#[derive(Clone, Copy, Default)]
pub(crate) struct Modifiers {
    ctrl: bool,
    alt: bool,
    /// Cmd on a Mac, the Windows key elsewhere.
    sup: bool,
}

/// Reads the typing while the line is open, and keeps it from the game.
pub(crate) fn capture(
    mut line: ResMut<CommandLine>,
    mut console: ResMut<Console>,
    time: Res<Time<Real>>,
    mut events: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut held: Local<Modifiers>,
) {
    // The modifiers are followed here: `keys` is wiped below, so it cannot say what is held.
    let mut typed = Vec::new();
    for event in events.read() {
        let down = event.state == ButtonState::Pressed;
        match event.logical_key {
            Key::Control => held.ctrl = down,
            Key::Alt => held.alt = down,
            Key::Super => held.sup = down,
            _ => {}
        }
        typed.push((event.clone(), *held));
    }
    if !line.open {
        // (What was typed before it opened is not for it.)
        return;
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(text) = crate::web::take_pasted() {
        line.insert(&text.replace(|c: char| c.is_control(), ""));
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(text) = crate::web::take_typed() {
        let text = text.replace(|c: char| c.is_control(), "");
        let space_typed = text.strip_prefix(line.text.as_str()) == Some(" ");
        if text != line.text && !(space_typed && line.end_cup_tracks()) {
            line.set_text(text);
        }
    }
    for (event, Modifiers { ctrl, alt, sup }) in &typed {
        if event.state != ButtonState::Pressed {
            continue;
        }
        let (ctrl, alt, sup) = (*ctrl, *alt, *sup);
        match &event.logical_key {
            Key::Enter => line.enter(),
            Key::Tab => {
                if !line.next_cup_track() && !line.accept_ghost() {
                    line.complete();
                }
            }
            Key::Escape => {
                line.open = false;
                line.text.clear();
                line.cursor = 0;
                line.browsing = None;
                line.draft.clear();
                line.selected = 0;
                line.moved_selection = false;
            }
            Key::ArrowUp => {
                if !line.cycle_lobby_name(false)
                    && !line.cycle_offer(false)
                    && !line.step(false)
                {
                    line.older();
                }
            }
            Key::ArrowDown => {
                if !line.cycle_lobby_name(true)
                    && !line.cycle_offer(true)
                    && !line.step(true)
                {
                    line.newer();
                }
            }
            Key::ArrowLeft => {
                line.cursor = if sup {
                    0
                } else if ctrl || alt {
                    line.word_start()
                } else {
                    line.before()
                }
            }
            Key::ArrowRight => {
                if line.cursor == line.text.len() && line.next_cup_track() {
                    continue;
                }
                line.cursor = if sup {
                    line.text.len()
                } else if ctrl || alt {
                    line.word_end()
                } else {
                    line.after()
                }
            }
            Key::Home => line.cursor = 0,
            Key::End => line.cursor = line.text.len(),
            Key::PageUp => console.scroll_by(PAGE, time.elapsed_secs()),
            Key::PageDown => console.scroll_by(-PAGE, time.elapsed_secs()),
            Key::Backspace if alt => {
                let range = line.word_start()..line.cursor;
                line.remove(range);
            }
            Key::Backspace => line.backspace(),
            Key::Delete => line.delete(),
            _ => {
                if (ctrl || sup) && event.key_code == KeyCode::KeyV {
                    paste(&mut line);
                } else if (ctrl || alt) && line.shortcut(event.key_code, ctrl, alt) {
                    // (Done.)
                } else if let Some(text) = event.text.as_ref().filter(|_| !sup) {
                    // (Ctrl A and the like are control characters, which are dropped.)
                    line.insert(&text.replace(|c: char| c.is_control(), ""));
                }
            }
        }
    }
    keys.reset_all();
}

/// Cmd/Ctrl+V: the text on the system clipboard goes into the line.
#[cfg(not(target_arch = "wasm32"))]
fn paste(line: &mut CommandLine) {
    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
        Ok(text) => line.insert(&text.replace(|c: char| c.is_control(), "")),
        Err(e) => error!("could not read the clipboard: {e}"),
    }
}

/// In a browser the clipboard is read in the background: the text comes in a later frame, which
/// `capture` takes it in.
#[cfg(target_arch = "wasm32")]
fn paste(_: &mut CommandLine) {
    crate::web::request_paste();
}

/// Return opens the line, unless we are typing in it already.
fn open(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    options: Res<crate::Options>,
    level: Res<crate::LevelNumber>,
    lobby: Res<crate::lobby::Lobby>,
    config: Res<crate::config::Config>,
    mut line: ResMut<CommandLine>,
) {
    if line.open || keys.just_pressed(KeyCode::Enter) {
        line.players = lobby
            .promotable()
            .map(|(character, name)| (name.to_string(), character as usize))
            .collect();
    }
    if !line.open && keys.just_pressed(KeyCode::Enter) {
        line.open = true;
        line.wanted[Offering::Character as usize] =
            crate::CHARACTERS[options.racer as usize].to_string();
        line.wanted[Offering::Player as usize].clear();
        line.wanted[Offering::Track as usize] = crate::maps::name(level.0);
        line.tracks = crate::maps::cycle()
            .into_iter()
            .map(|id| OfferedTrack {
                name: crate::maps::name(id),
                command_name: crate::maps::stable_name(id),
                id,
            })
            .collect();
        line.cups = config
            .cups
            .all()
            .iter()
            .map(|cup| OfferedCup {
                name: cup.name.clone(),
                first_track: cup.tracks.first().and_then(|t| crate::maps::resolve(t).ok()),
                about: cup_about(cup.tracks.len(), cup.laps),
            })
            .collect();
        line.wanted[Offering::Cup as usize] =
            line.cups.last().map(|cup| cup.name.clone()).unwrap_or_default();
        // (A browser's std hasher has fixed keys: the time is what differs there.)
        let seed = RandomState::new().hash_one(time.elapsed().as_nanos());
        line.lobby_name_index = seed % suggest::LOBBY_NAME_COUNT;
    }
}

/// Keeps the page's touch buttons and text field up with the line and the menu.
#[cfg(target_arch = "wasm32")]
fn tell_page(
    line: Res<CommandLine>,
    menu: Res<crate::pause::PauseMenu>,
    mut told: Local<Option<(bool, String, bool)>>,
) {
    let now = (line.open, line.text.clone(), menu.open);
    if told.as_ref() != Some(&now) {
        crate::web::show_line(now.0, &now.1, now.2);
        *told = Some(now);
    }
}

/// Sends the line: a command becomes an action, anything else is said in the chat.
fn submit(
    mut line: ResMut<CommandLine>,
    mut actions: MessageWriter<Action>,
    mut logs: MessageWriter<Log>,
    mut console: ResMut<Console>,
) {
    if !std::mem::take(&mut line.entered) {
        return;
    }
    line.open = false;
    line.cursor = 0;
    console.scroll_to_end();
    let text = std::mem::take(&mut line.text);
    if text.is_empty() {
        return;
    }
    line.remember(&text);
    let Some(command) = text.strip_prefix('/') else {
        actions.write(Action::Say(text));
        return;
    };
    match Action::parse(command) {
        Ok(action) => {
            actions.write(action);
        }
        Err(why) => {
            logs.write(Log::error(why));
        }
    }
}

/// The colours of the suggestions: the one picked, the others, and what they do.
const PICKED: Color = Color::srgb(1.0, 0.75, 0.3);
const OPTION: Color = Color::srgb(0.9, 0.9, 0.9);
const ABOUT: Color = Color::srgb(0.6, 0.6, 0.6);

/// The line and its suggestions, for the console to put at the bottom of its window.
pub(crate) fn input_row() -> impl Bundle {
    let font = TextFont {
        font_size: FONT_SIZE.into(),
        ..default()
    };
    (
        InputRow,
        Node {
            flex_direction: FlexDirection::Column,
            ..default()
        },
        Visibility::Hidden,
        children![
            (
                // Over the log, right above the line.
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Percent(100.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                children![
                    (
                        TrackPanel,
                        Node {
                            display: Display::None,
                            padding: UiRect::all(Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(Color::BLACK),
                        children![(
                            TrackImage,
                            ImageNode::default().with_mode(NodeImageMode::Stretch),
                            Node::default(),
                        )],
                    ),
                    (
                        Suggestions,
                        Node {
                            display: Display::None,
                            // Two columns, the names and what they do: a row's two texts line up
                            // with the others', and what they do wraps in its own column.
                            grid_template_columns: vec![GridTrack::auto(), GridTrack::flex(1.0)],
                            column_gap: Val::Px(16.0),
                            row_gap: Val::Px(6.0),
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::BLACK),
                    ),
                ],
            ),
            (
                Node {
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::BLACK),
                children![
                    (
                        CommandText,
                        Text::new("> "),
                        // (As in the console: what is typed wraps too.)
                        TextLayout::linebreak(LineBreak::WordOrCharacter),
                        font.clone(),
                        Node {
                            padding: UiRect::all(Val::Px(8.0)),
                            ..default()
                        },
                        children![
                            (
                                CursorSpan,
                                TextSpan::new(" "),
                                font.clone(),
                                TextBackgroundColor(Color::NONE),
                            ),
                            (AfterSpan, TextSpan::new(""), font.clone()),
                            (GhostSpan, TextSpan::new(""), font, TextColor(ABOUT)),
                        ],
                    ),
                    (
                        OfferIcon,
                        small_portrait(None, 1.0),
                    ),
                ],
            ),
        ],
    )
}

/// Sets a string, leaving it (and so what watches it for changes) alone if it is the same.
fn assign(to: &mut String, from: &str) {
    if to != from {
        to.clear();
        to.push_str(from);
    }
}

/// The line on screen while it is open, and over it what can be typed next.
fn show(
    line: Res<CommandLine>,
    time: Res<Time<Real>>,
    mut commands: Commands,
    mut rows: Query<&mut Visibility, With<InputRow>>,
    mut suggestions: Query<(Entity, &mut Node), With<Suggestions>>,
    mut prompt: Query<&mut Text, With<CommandText>>,
    mut cursor: Query<(&mut TextSpan, &mut TextColor, &mut TextBackgroundColor), With<CursorSpan>>,
    mut after: Query<&mut TextSpan, (With<AfterSpan>, Without<CursorSpan>)>,
    mut ghost_span: Query<
        &mut TextSpan,
        (With<GhostSpan>, Without<CursorSpan>, Without<AfterSpan>),
    >,
    // What the suggestions were made for: the text, the one picked and the name offered.
    mut shown: Local<Option<(String, usize, Option<String>)>>,
) {
    for mut row in &mut rows {
        let want = if line.open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *row != want {
            *row = want;
        }
    }
    if !line.open {
        *shown = None;
        return;
    }
    let (before, rest) = line.text.split_at(line.cursor);
    let ghost = line.ghost().unwrap_or_default();
    // At the end of the line the cursor sits on the first char of the name in grey.
    let cursor_on_ghost = rest.is_empty() && !ghost.is_empty();
    let (under, ghost_rest) = match (rest.chars().next(), ghost.chars().next()) {
        (Some(c), _) => (c.to_string(), ghost.as_str()),
        (None, Some(c)) => (c.to_string(), &ghost[c.len_utf8()..]),
        (None, None) => (" ".to_string(), ""),
    };
    let blink_on = ((time.elapsed_secs() / BLINK_SECONDS) as u32).is_multiple_of(2);
    for mut text in &mut prompt {
        assign(&mut text.0, &format!("> {before}"));
    }
    for (mut span, mut color, mut background) in &mut cursor {
        assign(&mut span.0, &under);
        let (ink, back) = if blink_on {
            (Color::BLACK, Color::WHITE)
        } else {
            (
                if cursor_on_ghost { ABOUT } else { Color::WHITE },
                Color::NONE,
            )
        };
        if color.0 != ink {
            color.0 = ink;
        }
        if background.0 != back {
            background.0 = back;
        }
    }
    for mut span in &mut after {
        assign(
            &mut span.0,
            rest.chars().next().map_or("", |c| &rest[c.len_utf8()..]),
        );
    }
    for mut span in &mut ghost_span {
        assign(&mut span.0, ghost_rest);
    }
    let Ok((list, mut node)) = suggestions.single_mut() else {
        return;
    };
    let offered = line.offer().map(|offer| offer.name.to_string());
    let key = (line.text.clone(), line.selected, offered);
    if shown.as_ref() == Some(&key) {
        return;
    }
    *shown = Some(key);
    commands.entity(list).despawn_children();
    let rows = suggestion_rows(&line);
    node.display = if rows.is_empty() {
        Display::None
    } else {
        Display::Grid
    };
    let font = TextFont {
        font_size: FONT_SIZE.into(),
        ..default()
    };
    for (name, about, color) in rows {
        commands.spawn((
            Text::new(name),
            font.clone(),
            TextColor(color),
            ChildOf(list),
        ));
        commands.spawn((
            Text::new(about),
            font.clone(),
            TextColor(ABOUT),
            ChildOf(list),
        ));
    }
}

/// What goes with what the line offers or names: the portrait after the line, or the picture of
/// the track over it.
fn show_offer_picture(
    line: Res<CommandLine>,
    time: Res<Time<Real>>,
    mut pictures: ResMut<TrackPictures>,
    mut images: ResMut<Assets<Image>>,
    mut icons: Query<&mut SmallPortrait, With<OfferIcon>>,
    mut panels: Query<&mut Node, With<TrackPanel>>,
    mut track_images: Query<(&mut ImageNode, &mut Node), (With<TrackImage>, Without<TrackPanel>)>,
) {
    let picture = line
        .open
        .then(|| line.offer().map(|offer| offer.picture))
        .flatten();
    let character = match picture {
        Some(OfferPicture::Portrait(character)) => Some(character),
        _ => None,
    };
    for mut icon in &mut icons {
        if icon.0 != character {
            icon.0 = character;
        }
    }
    let offered = match picture {
        Some(OfferPicture::Track(id)) => Some(id),
        _ => None,
    };
    pictures.set_offered(offered, time.elapsed_secs(), &mut images);
    let track = offered.and_then(|id| pictures.of(id, &mut images));
    let display = if track.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    for mut panel in &mut panels {
        if panel.display != display {
            panel.display = display;
        }
    }
    let Some(track) = track else {
        return;
    };
    for (mut image, mut node) in &mut track_images {
        if image.rect != Some(track.rect) || image.image != track.image {
            image.rect = Some(track.rect);
            image.image = track.image.clone();
            node.width = Val::Px(track.size.x);
            node.height = Val::Px(track.size.y);
        }
    }
}

/// What a row of `/cup start` says of a cup.
fn cup_about(tracks: usize, laps: u8) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    format!("{tracks} track{}, {laps} lap{}", plural(tracks), plural(laps as usize))
}

/// The rows of suggestions for the line: each option, the picked one in its own colour, and
/// what else can be typed there; and what they do.
fn suggestion_rows(line: &CommandLine) -> Vec<(String, String, Color)> {
    let row = |name: &str, about: &str, color| (name.to_string(), about.to_string(), color);
    if line.on_cup_track() {
        return vec![row("<tracks>", "Up/Down, Tab/Right adds, Space: laps", ABOUT)];
    }
    if line.browsing.is_none() && suggest::cup_track_typed(&line.text).is_some() {
        return vec![row("<laps>", "The laps of each race", ABOUT)];
    }
    if line.browsing.is_none() && suggest::cup_name_typed(&line.text).is_some() {
        if line.cups.is_empty() {
            return vec![row("<name>", "No cups yet: /cup create", ABOUT)];
        }
        if let Some((_, _, offers)) = line.offers().filter(|(_, _, offers)| !offers.is_empty()) {
            let offered = line.offer().map(|offer| offer.name);
            return offers
                .iter()
                .filter_map(|offer| line.cups.iter().find(|cup| cup.name == offer.name))
                .map(|cup| {
                    let color = if offered == Some(cup.name.as_str()) {
                        PICKED
                    } else {
                        OPTION
                    };
                    row(&cup.name, &cup.about, color)
                })
                .collect();
        }
    }
    let Some(menu) = line.menu() else {
        return Vec::new();
    };
    let picked = line
        .selection()
        .filter(|_| !line.ghost_goes_first())
        .map(|(_, index)| index);
    let mut rows: Vec<_> = menu
        .options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let color = if picked == Some(i) { PICKED } else { OPTION };
            row(o.name, o.about, color)
        })
        .collect();
    if let Some((arg, about)) = menu.arg {
        rows.push(row(arg, about, ABOUT));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bots_take_a_count_and_a_difficulty_in_any_order() {
        let bots = |line| match Action::parse(line) {
            Ok(Action::Bots { count, difficulty }) => Some((count, difficulty)),
            _ => None,
        };
        assert_eq!(bots("bots"), Some((None, None)));
        assert_eq!(bots("bots 7"), Some((Some(7), None)));
        assert_eq!(bots("bots off"), Some((Some(0), None)));
        assert_eq!(bots("bots hard 3"), Some((Some(3), Some(0xf0))));
        assert_eq!(bots("bots 5 easy"), Some((Some(5), Some(0x50))));
        assert_eq!(bots("bots medium"), Some((None, Some(0xa0))));
        assert_eq!(bots("bots 8"), None);
        assert_eq!(bots("bots lots"), None);
    }

    #[test]
    fn commands_have_subcommands() {
        assert!(matches!(
            Action::parse("lobby create"),
            Ok(Action::CreateLobby { public: None })
        ));
        assert!(matches!(
            Action::parse("lobby create public"),
            Ok(Action::CreateLobby { public: Some(name) }) if name.is_empty()
        ));
        assert!(matches!(
            Action::parse("lobby create public Fast lane"),
            Ok(Action::CreateLobby { public: Some(name) }) if name == "Fast lane"
        ));
        assert!(matches!(
            Action::parse("lobby create private"),
            Ok(Action::CreateLobby { public: None })
        ));
        assert!(Action::parse("lobby create private Fast").is_err());
        assert!(Action::parse("lobby create public Fäst").is_err());
        assert!(Action::parse(&format!("lobby create public {}", "a".repeat(33))).is_err());
        assert!(matches!(
            Action::parse("lobby close"),
            Ok(Action::CloseLobby)
        ));
        assert!(matches!(
            Action::parse("cache purge"),
            Ok(Action::PurgeCache)
        ));
        assert!(matches!(
            Action::parse("track list"),
            Ok(Action::ListTracks)
        ));
        assert!(matches!(Action::parse("race start"), Ok(Action::StartRace)));
        assert!(matches!(
            Action::parse("race restart"),
            Ok(Action::RestartRace)
        ));
        assert!(matches!(Action::parse("race stop"), Ok(Action::StopRace)));
        assert!(matches!(
            Action::parse("cup create fast dingo_canyon,crash_cove 5"),
            Ok(Action::CupCreate { name, tracks, laps: 5 }) if name == "fast" && tracks.len() == 2
        ));
        assert!(Action::parse("cup create fast nowhere 5").is_err());
        assert!(Action::parse("cup create fast dingo_canyon 0").is_err());
        assert!(matches!(
            Action::parse("cup start fast"),
            Ok(Action::CupStart(name)) if name == "fast"
        ));
        assert!(matches!(
            Action::parse("debug set wumpa 10"),
            Ok(Action::DebugSetWumpa(10))
        ));
        assert!(matches!(Action::parse("net info"), Ok(Action::NetInfo)));
        assert!(matches!(
            Action::parse("debug stall 3s"),
            Ok(Action::DebugStall(d)) if d == std::time::Duration::from_secs(3)
        ));
        assert!(Action::parse("debug stall").is_err());
        let lag = |line| match Action::parse(line) {
            Ok(Action::DebugLag(lag)) => Some(lag.map(|l| (l.delay.as_millis(), l.jitter.as_millis()))),
            _ => None,
        };
        assert_eq!(lag("debug lag 75 15"), Some(Some((75, 15))));
        assert_eq!(lag("debug lag 75ms"), Some(Some((75, 0))));
        assert_eq!(lag("debug lag 0.1s 20ms"), Some(Some((100, 20))));
        assert_eq!(lag("debug lag off"), Some(None));
        assert_eq!(lag("debug lag 0"), Some(None));
        assert_eq!(lag("debug lag"), None);
        assert_eq!(lag("debug lag 2s"), None);
        assert_eq!(lag("debug lag 75 600"), None);
        assert_eq!(lag("debug lag 75 15 3"), None);
        assert!(Action::parse("debug stall 5m").is_err());
        assert!(Action::parse("debug stall 61s").is_err());
        assert!(Action::parse("debug netinfo").is_err());
        assert!(matches!(
            Action::parse("debug spawn-karts 7"),
            Ok(Action::DebugSpawnKarts(7))
        ));
        assert!(Action::parse("debug spawn-karts 8").is_err());
        assert!(Action::parse("debug spawn-karts").is_err());
        let fx = |line| match Action::parse(line) {
            Ok(Action::DebugFx(fx)) => Some(fx),
            _ => None,
        };
        use crate::fx::{Force, Fx};
        assert_eq!(fx("debug fx"), Some(Fx::Panel));
        assert_eq!(fx("debug fx confetti on"), Some(Fx::Confetti(Force::On)));
        assert_eq!(
            fx("debug fx confetti auto"),
            Some(Fx::Confetti(Force::Auto))
        );
        assert_eq!(fx("debug fx flash"), Some(Fx::Flash));
        assert_eq!(fx("debug fx finish"), Some(Fx::Finish));
        assert_eq!(fx("debug fx speed 0.25"), Some(Fx::Speed(0.25)));
        assert_eq!(fx("debug fx freeze on"), Some(Fx::Freeze(true)));
        assert_eq!(fx("debug fx step"), Some(Fx::Step));
        assert_eq!(fx("debug fx weather off"), Some(Fx::Weather(false)));
        assert_eq!(fx("debug fx rain off"), Some(Fx::BeakerRain(false)));
        assert!(Action::parse("debug fx speed 0").is_err());
        assert!(Action::parse("debug fx speed fast").is_err());
        assert!(Action::parse("debug fx confetti").is_err());
        assert!(Action::parse("debug fx sparkles").is_err());
        assert!(matches!(
            Action::parse("config set nickname  Big Bob "),
            Ok(Action::SetConfig(crate::config::Setting::Nickname(n))) if n == "Big Bob"
        ));
        assert!(matches!(
            Action::parse("config set nickname"),
            Ok(Action::SetConfig(crate::config::Setting::Nickname(n))) if n.is_empty()
        ));
        assert!(Action::parse("config set nickname 0123456789abc").is_err());
        assert!(Action::parse("config set colour red").is_err());
        assert!(Action::parse("config get nickname").is_err());
        assert!(matches!(
            Action::parse("config set vsync off"),
            Ok(Action::SetConfig(crate::config::Setting::Vsync(false)))
        ));
        assert!(Action::parse("config set vsync maybe").is_err());
        assert!(matches!(
            Action::parse("net relay only on"),
            Ok(Action::SetConfig(crate::config::Setting::RelayOnly(true)))
        ));
        assert!(Action::parse("net relay only maybe").is_err());
        assert!(matches!(
            Action::parse("config set fps-max 144"),
            Ok(Action::SetConfig(crate::config::Setting::FpsMax(144)))
        ));
        assert!(matches!(
            Action::parse("config set fps-max 0"),
            Ok(Action::SetConfig(crate::config::Setting::FpsMax(0)))
        ));
        assert!(Action::parse("config set fps-max fast").is_err());
        assert!(Action::parse("config set fps-max -1").is_err());
        assert!(matches!(
            Action::parse("config set msaa off"),
            Ok(Action::SetConfig(crate::config::Setting::Msaa(1)))
        ));
        assert!(matches!(
            Action::parse("config set msaa 4"),
            Ok(Action::SetConfig(crate::config::Setting::Msaa(4)))
        ));
        assert!(Action::parse("config set msaa 8").is_err());
        assert!(matches!(Action::parse("version"), Ok(Action::Version)));
        assert!(Action::parse("version 1").is_err());
        assert!(matches!(Action::parse("clear"), Ok(Action::Clear)));
        assert!(Action::parse("clear all").is_err());
        assert!(matches!(
            Action::parse("config set volume music 40"),
            Ok(Action::SetConfig(crate::config::Setting::VolumeMusic(40)))
        ));
        assert!(matches!(
            Action::parse("config set volume sfx 0"),
            Ok(Action::SetConfig(crate::config::Setting::VolumeSfx(0)))
        ));
        assert!(Action::parse("config set volume sfx 101").is_err());
        assert!(Action::parse("config set volume sfx").is_err());
        assert!(Action::parse("config set volume loud 50").is_err());
        let smoothing = |line| match Action::parse(line) {
            Ok(Action::NetSmoothing(how)) => Some(how),
            _ => None,
        };
        use crate::kart::Smoothing::{Fixed, Ping, Replay};
        use std::time::Duration;
        assert_eq!(smoothing("net smoothing ping"), Some(Ping));
        assert_eq!(smoothing("net smoothing dynamic"), Some(Ping));
        assert_eq!(smoothing("net smoothing replay"), Some(Replay));
        assert_eq!(
            smoothing("net smoothing 120ms"),
            Some(Fixed(Duration::from_millis(120)))
        );
        assert_eq!(
            smoothing("net smoothing 0.25s"),
            Some(Fixed(Duration::from_millis(250)))
        );
        assert_eq!(
            smoothing("net smoothing 80"),
            Some(Fixed(Duration::from_millis(80)))
        );
        assert!(matches!(
            Action::parse("net rollbacks on"),
            Ok(Action::NetRollbacks(true))
        ));
        assert!(matches!(
            Action::parse("net rollbacks off"),
            Ok(Action::NetRollbacks(false))
        ));
        assert!(Action::parse("net rollbacks").is_err());
        assert_eq!(smoothing("net smoothing"), None);
        assert_eq!(smoothing("net smoothing 5s"), None);
        assert_eq!(smoothing("net smoothing fast"), None);
        // Clamped to what a kart holds.
        assert!(matches!(
            Action::parse("debug  set   wumpa 99"),
            Ok(Action::DebugSetWumpa(10))
        ));
        assert!(matches!(
            Action::parse("debug set weapon 4"),
            Ok(Action::DebugAddWeapon(4))
        ));
    }

    fn typed(text: &str) -> CommandLine {
        let mut line = CommandLine::default();
        line.insert(text);
        line
    }

    #[test]
    fn return_takes_the_option_picked_over_a_whole_command() {
        let mut line = typed("/debug fx ");
        line.step(true);
        line.enter();
        assert_eq!(line.text, "/debug fx flash");
        assert!(line.entered);
        let mut line = typed("/debug set weapon 1");
        line.step(true);
        line.enter();
        assert_eq!(line.text, "/debug set weapon 10");
        assert!(line.entered);
        // Round to the first is still a pick.
        let mut line = typed("/debug hud ");
        line.step(true);
        line.step(true);
        line.enter();
        assert_eq!(line.text, "/debug hud on");
        // Nothing picked: the command as it is.
        let mut line = typed("/debug fx ");
        line.enter();
        assert_eq!(line.text, "/debug fx ");
        assert!(line.entered);
    }

    #[test]
    fn a_public_lobby_is_offered_a_name() {
        let name = suggest::lobby_name_at(0);
        let line_with_name = format!("/lobby create public {name}");
        // Return takes it, with or without the space.
        for text in ["/lobby create public", "/lobby create public "] {
            let mut line = typed(text);
            line.enter();
            assert_eq!(line.text, line_with_name);
            assert!(line.entered);
        }
        // Picking `public` stops to show it, and the next Return takes it.
        let mut line = typed("/lobby create ");
        line.step(true);
        line.enter();
        assert_eq!(line.text, "/lobby create public ");
        assert!(!line.entered);
        assert_eq!(line.ghost(), Some(name.clone()));
        // The arrows change it, and take back what they changed, past either end.
        assert!(line.cycle_lobby_name(true));
        assert_eq!(line.ghost(), Some(suggest::lobby_name_at(1)));
        assert!(line.cycle_lobby_name(false));
        assert!(line.cycle_lobby_name(false));
        assert_eq!(
            line.ghost(),
            Some(suggest::lobby_name_at(suggest::LOBBY_NAME_COUNT - 1))
        );
        assert!(line.cycle_lobby_name(true));
        line.enter();
        assert_eq!(line.text, line_with_name);
        assert!(line.entered);
        // With no name offered they do what they did.
        assert!(!typed("/lobby create public Fast").cycle_lobby_name(true));
        assert!(!typed("/race ").cycle_lobby_name(true));
        // A name typed is kept.
        let mut line = typed("/lobby create public Fast");
        assert_eq!(line.ghost(), None);
        line.enter();
        assert_eq!(line.text, "/lobby create public Fast");
        assert!(line.entered);
        // A private lobby has no name.
        let mut line = typed("/lobby create");
        assert_eq!(line.ghost(), None);
        line.enter();
        assert_eq!(line.text, "/lobby create");
    }

    #[test]
    fn a_character_is_offered() {
        let offered = |name: &str| {
            let mut line = typed("/character ");
            line.wanted[Offering::Character as usize] = name.to_string();
            line
        };
        // Return takes the one we drive.
        let mut line = offered("Penta");
        assert_eq!(line.ghost().as_deref(), Some("Penta"));
        line.enter();
        assert_eq!(line.text, "/character Penta");
        assert!(line.entered);
        assert!(matches!(
            Action::parse("character Penta"),
            Ok(Action::Character(13))
        ));
        // The arrows go round.
        let mut line = offered("Crash");
        assert!(line.cycle_offer(false));
        assert_eq!(line.ghost().as_deref(), Some("Oxide"));
        assert!(line.cycle_offer(true));
        assert_eq!(line.ghost().as_deref(), Some("Crash"));
        // Picking the command stops to show it.
        let mut line = typed("/charac");
        line.wanted[Offering::Character as usize] = "Penta".into();
        line.enter();
        assert_eq!(line.text, "/character ");
        assert!(!line.entered);
        assert_eq!(line.ghost().as_deref(), Some("Penta"));
        // Typing narrows the names down, and the arrows stay among those.
        let mut line = offered("Crash");
        line.insert("n");
        assert_eq!(line.ghost().as_deref(), Some(". Gin"));
        assert!(line.cycle_offer(true));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Portrait(12)));
        assert_eq!(line.ghost().as_deref(), Some(". Tropy"));
        line.enter();
        assert_eq!(line.text, "/character n. Tropy");
        assert!(matches!(
            Action::parse("character n. Tropy"),
            Ok(Action::Character(12))
        ));
        // A whole name has its portrait, and the arrows go to the history.
        let mut line = typed("/character Penta");
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Portrait(13)));
        assert_eq!(line.ghost(), None);
        assert!(!line.cycle_offer(false));
        // None by that name.
        let line = typed("/character nobody");
        assert_eq!(line.offer(), None);
        assert!(Action::parse("character nobody").unwrap_err().contains("Oxide"));
        assert!(Action::parse("character").unwrap_err().starts_with("Usage"));
        // Other lines have none.
        assert_eq!(typed("/race ").offer(), None);
    }

    #[test]
    fn a_player_is_offered_to_promote() {
        let players = |text: &str| {
            let mut line = typed(text);
            line.players = vec![("Penta".into(), 13), ("Bob".into(), 3)];
            line
        };
        let mut line = players("/promote ");
        assert_eq!(line.ghost().as_deref(), Some("Penta"));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Portrait(13)));
        assert!(line.cycle_offer(true));
        assert_eq!(line.ghost().as_deref(), Some("Bob"));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Portrait(3)));
        line.enter();
        assert_eq!(line.text, "/promote Bob");
        assert!(line.entered);
        // Typed part of the way.
        let mut line = players("/promote b");
        assert_eq!(line.ghost().as_deref(), Some("ob"));
        assert!(!line.cycle_offer(true));
        line.enter();
        assert_eq!(line.text, "/promote bob");
        // A key, or a guest's line with nobody to offer, goes as it was typed.
        let mut line = players("/promote abcde");
        assert_eq!(line.ghost(), None);
        line.enter();
        assert_eq!(line.text, "/promote abcde");
        assert_eq!(typed("/promote ").ghost(), None);
    }

    fn offered_tracks(tracks: &[(&str, &str, u32)]) -> Vec<OfferedTrack> {
        tracks
            .iter()
            .map(|&(name, command_name, id)| OfferedTrack {
                name: name.into(),
                command_name: command_name.into(),
                id,
            })
            .collect()
    }

    #[test]
    fn a_track_is_offered() {
        let tracks = |text: &str| {
            let mut line = typed(text);
            line.wanted[Offering::Track as usize] = "Crash Cove".into();
            line.wanted[Offering::Character as usize] = "Penta".into();
            line.tracks = offered_tracks(&[
                ("Dingo Canyon", "dingo_canyon", 0),
                ("Crash Cove", "crash_cove", 3),
                ("Papu's Pyramid", "papu_pyramid", 5),
            ]);
            line
        };
        // The one we are on first, whatever was picked of the others.
        let mut line = tracks("/track ");
        assert_eq!(line.ghost().as_deref(), Some("Crash Cove"));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Track(3)));
        assert!(line.cycle_offer(true));
        assert_eq!(line.ghost().as_deref(), Some("Papu's Pyramid"));
        line.enter();
        assert_eq!(line.text, "/track Papu's Pyramid");
        assert!(line.entered);
        assert_eq!(line.wanted[Offering::Character as usize], "Penta");
        // Typing narrows them down, across a space too.
        let line = tracks("/track dingo c");
        assert_eq!(line.ghost().as_deref(), Some("anyon"));
        // `list` goes to the list: typed, or picked past the names. Not over the name in grey.
        assert!(matches!(Action::parse("track list"), Ok(Action::ListTracks)));
        let mut line = tracks("/track li");
        line.enter();
        assert_eq!(line.text, "/track list");
        assert!(line.entered);
        let mut line = tracks("/track");
        line.enter();
        assert_eq!(line.text, "/track ");
        assert!(!line.entered);
        line.enter();
        assert_eq!(line.text, "/track Crash Cove");
        assert!(line.entered);
        // What a command can take goes as it was typed.
        let mut line = tracks("/track dingo_canyon");
        assert_eq!(line.ghost(), None);
        line.enter();
        assert_eq!(line.text, "/track dingo_canyon");

        for text in ["track Papu's Pyramid", "track papu_pyramid", "track 5"] {
            assert!(matches!(Action::parse(text), Ok(Action::Track(5))), "{text}");
        }
        assert!(matches!(Action::parse("track n. gin labs"), Ok(Action::Track(11))));
        assert!(Action::parse("track nowhere").unwrap_err().contains("Unknown track"));
        assert!(Action::parse("track").unwrap_err().starts_with("Usage"));
    }

    #[test]
    fn a_cups_tracks_are_offered_one_after_another() {
        let mut line = typed("/cup create fast");
        line.wanted[Offering::Track as usize] = "Crash Cove".into();
        line.tracks = offered_tracks(&[
            ("Dingo Canyon", "dingo_canyon", 0),
            ("Crash Cove", "crash_cove", 3),
            ("Odd, Name", "odd__name", 4),
            ("Papu's Pyramid", "papu_pyramid", 5),
        ]);
        // Still typing the cup's name.
        assert_eq!(line.offer(), None);
        line.insert(" ");
        assert_eq!(line.ghost().as_deref(), Some("Crash Cove"));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Track(3)));
        assert!(line.cycle_offer(false));
        assert_eq!(line.ghost().as_deref(), Some("Dingo Canyon"));

        // The full name is shown, the short one goes in, and the one after it is offered.
        assert!(line.next_cup_track());
        assert_eq!(line.text, "/cup create fast dingo_canyon,");
        assert_eq!(line.ghost().as_deref(), Some("Crash Cove"));
        assert!(line.cycle_offer(true));
        assert_eq!(line.ghost().as_deref(), Some("Odd, Name"));
        assert!(line.next_cup_track());
        assert_eq!(line.text, "/cup create fast dingo_canyon,odd__name,");

        // Typing narrows them down, by either name; each goes in as the short one.
        line.insert("Pa");
        assert_eq!(line.ghost().as_deref(), Some("pu's Pyramid"));
        assert!(line.next_cup_track());
        assert_eq!(line.text, "/cup create fast dingo_canyon,odd__name,papu_pyramid,");
        line.insert("crash_");
        assert_eq!(line.ghost().as_deref(), Some("cove"));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Track(3)));
        line.insert("cove");
        assert_eq!(line.ghost(), None);
        assert!(line.next_cup_track());
        let list = "/cup create fast dingo_canyon,odd__name,papu_pyramid,crash_cove";
        assert_eq!(line.text, format!("{list},"));
        line.insert("crash cove");
        assert_eq!(line.ghost(), None);
        assert!(line.next_cup_track());
        assert_eq!(line.text, format!("{list},crash_cove,"));

        // Return takes the last and leaves the laps to type, then sends the line.
        line.enter();
        assert_eq!(line.text, format!("{list},crash_cove,odd__name "));
        assert!(!line.entered);
        assert_eq!(line.offer(), None);
        assert_eq!(suggestion_rows(&line)[0].0, "<laps>");
        assert!(!line.next_cup_track());
        line.insert("3");
        line.enter();
        assert!(line.entered);
    }

    /// A line with two cups to offer, `slow` the one made last.
    fn typed_with_cups(text: &str) -> CommandLine {
        let cup = |name: &str, first_track, tracks, laps| OfferedCup {
            name: name.into(),
            first_track,
            about: cup_about(tracks, laps),
        };
        let mut line = typed(text);
        line.cups = vec![cup("fast", Some(3), 2, 3), cup("slow", Some(7), 1, 1)];
        line.wanted[Offering::Cup as usize] = "slow".into();
        line
    }

    #[test]
    fn return_only_fills_in_the_cup_to_delete() {
        let mut line = typed_with_cups("/cup delete");
        line.enter();
        assert_eq!(line.text, "/cup delete slow");
        assert!(!line.entered);
        line.enter();
        assert!(line.entered);
        assert!(matches!(Action::parse("cup delete slow"), Ok(Action::CupDelete(name)) if name == "slow"));
        assert!(Action::parse("cup delete").is_err());

        let mut line = typed_with_cups("/cup delete ");
        assert!(line.cycle_offer(true));
        line.enter();
        assert_eq!(line.text, "/cup delete fast");
        assert!(!line.entered);
        assert_eq!(suggestion_rows(&line)[0].0, "fast");
        // With nothing to offer it goes as it was typed, to say so.
        let mut line = typed_with_cups("/cup delete nope");
        line.enter();
        assert!(line.entered);
    }

    #[test]
    fn a_cup_whose_first_track_is_gone_is_still_offered() {
        let mut line = typed("/cup start ");
        line.cups = vec![OfferedCup {
            name: "old".into(),
            first_track: None,
            about: cup_about(1, 1),
        }];
        assert_eq!(line.ghost().as_deref(), Some("old"));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Nothing));
    }

    #[test]
    fn the_cups_are_offered_to_start() {
        // The latest first, and all of them listed with the one offered picked.
        let mut line = typed_with_cups("/cup start ");
        assert_eq!(line.ghost().as_deref(), Some("slow"));
        assert_eq!(line.offer().map(|o| o.picture), Some(OfferPicture::Track(7)));
        let rows = suggestion_rows(&line);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].0.as_str(), rows[0].1.as_str()), ("fast", "2 tracks, 3 laps"));
        assert_eq!((rows[1].0.as_str(), rows[1].1.as_str()), ("slow", "1 track, 1 lap"));
        assert_eq!((rows[0].2, rows[1].2), (OPTION, PICKED));
        assert!(line.cycle_offer(true));
        assert_eq!(line.ghost().as_deref(), Some("fast"));
        assert_eq!(suggestion_rows(&line)[0].2, PICKED);
        line.enter();
        assert_eq!(line.text, "/cup start fast");
        assert!(line.entered);
        assert!(matches!(Action::parse("cup start fast"), Ok(Action::CupStart(name)) if name == "fast"));

        // Typing narrows them down, in any case.
        let mut line = typed_with_cups("/cup start F");
        assert_eq!(line.ghost().as_deref(), Some("ast"));
        assert_eq!(suggestion_rows(&line).len(), 1);
        line.enter();
        assert_eq!(line.text, "/cup start Fast");
        assert!(line.entered);

        // Nothing to offer: the name is still what can go there.
        let line = typed_with_cups("/cup start nope");
        assert_eq!(line.ghost(), None);
        assert_eq!(suggestion_rows(&line)[0].0, "<name>");
        let line = typed("/cup start ");
        assert_eq!(line.ghost(), None);
        assert!(suggestion_rows(&line)[0].1.contains("/cup create"));
    }

    #[test]
    fn space_ends_a_cups_tracks() {
        let mut line = typed("/cup create fast dingo_canyon,");
        line.wanted[Offering::Track as usize] = "Crash Cove".into();
        line.tracks = offered_tracks(&[
            ("Dingo Canyon", "dingo_canyon", 0),
            ("Crash Cove", "crash_cove", 3),
        ]);
        line.insert(" ");
        assert_eq!(line.text, "/cup create fast dingo_canyon ");
        assert_eq!(line.cursor, line.text.len());
        assert_eq!(line.offer(), None);
        assert_eq!(suggestion_rows(&line)[0].0, "<laps>");
        line.insert("3");
        assert!(matches!(
            Action::parse(&line.text[1..]),
            Ok(Action::CupCreate { laps: 3, .. })
        ));

        // A space inside a name, or before any track was taken, is only a space.
        let mut line = typed("/cup create fast Crash");
        line.insert(" ");
        assert_eq!(line.text, "/cup create fast Crash ");
        let mut line = typed("/cup create fast ");
        line.insert(" ");
        assert_eq!(line.text, "/cup create fast  ");
        // As is one in the middle of the line.
        let mut line = typed("/cup create fast a,");
        line.cursor = 5;
        line.insert(" ");
        assert_eq!(line.text, "/cup  create fast a,");
    }

    #[test]
    fn a_cups_tracks_can_be_named_with_spaces() {
        let cup = |text| match Action::parse(text) {
            Ok(Action::CupCreate { name, tracks, laps }) => (name, tracks, laps),
            other => panic!("{text}: {other:?}"),
        };
        let wanted = ("fast".to_string(), vec![5, 11], 3);
        for text in [
            "cup create fast Papu's Pyramid,N. Gin Labs 3",
            "cup create fast Papu's Pyramid, N. Gin Labs 3",
            "cup create fast papu_pyramid,N. Gin Labs,  3",
        ] {
            assert_eq!(cup(text), wanted, "{text}");
        }
        assert!(Action::parse("cup create fast").is_err());
        assert!(Action::parse("cup create fast 3").is_err());
        assert!(Action::parse("cup create fast Papu's Pyramid").is_err());
    }

    #[test]
    fn typing_goes_in_at_the_cursor() {
        let mut line = typed("/lobby create");
        line.shortcut(KeyCode::KeyA, true, false);
        line.insert("x");
        assert_eq!(line.text, "x/lobby create");
        assert_eq!(line.cursor, 1);
        line.shortcut(KeyCode::KeyE, true, false);
        line.backspace();
        assert_eq!(line.text, "x/lobby creat");
        line.shortcut(KeyCode::KeyB, true, false);
        line.delete();
        assert_eq!(line.text, "x/lobby crea");
        // Characters of more than a byte are moved over whole.
        let mut line = typed("aäö");
        line.cursor = line.before();
        assert_eq!(line.cursor, 3);
        line.backspace();
        assert_eq!(line.text, "aö");
        line.delete();
        assert_eq!(line.text, "a");
    }

    #[test]
    fn words_are_deleted() {
        let mut line = typed("net smoothing  replay");
        line.shortcut(KeyCode::KeyW, true, false);
        assert_eq!(line.text, "net smoothing  ");
        line.shortcut(KeyCode::KeyW, true, false);
        assert_eq!(line.text, "net ");
        line.insert("smoothing  ");
        line.shortcut(KeyCode::KeyB, false, true);
        assert_eq!(line.cursor, 4);
        line.shortcut(KeyCode::KeyF, false, true);
        assert_eq!(line.cursor, 13);
        line.cursor = 4;
        line.shortcut(KeyCode::KeyD, false, true);
        assert_eq!(line.text, "net   ");
        line.shortcut(KeyCode::KeyA, true, false);
        line.shortcut(KeyCode::KeyK, true, false);
        assert_eq!(line.text, "");
        line.insert("abc");
        line.cursor = 1;
        line.shortcut(KeyCode::KeyU, true, false);
        assert_eq!(line.text, "bc");
        // Keys that are not for it are not taken.
        assert!(!line.shortcut(KeyCode::KeyQ, true, false));
        assert!(!line.shortcut(KeyCode::KeyA, false, true));
    }

    #[test]
    fn the_history_puts_the_cursor_at_the_end() {
        let mut line = typed("draft");
        line.remember("/version");
        line.older();
        assert_eq!((line.text.as_str(), line.cursor), ("/version", 8));
        line.newer();
        assert_eq!((line.text.as_str(), line.cursor), ("draft", 5));
    }

    #[test]
    fn a_missing_or_wrong_part_says_what_there_is() {
        let error = |line| Action::parse(line).unwrap_err();
        assert!(error("lobby").starts_with("Usage: /lobby"));
        assert!(error("lobby fly").contains("Unknown command: /lobby fly"));
        assert!(error("debug").starts_with("Usage: /debug"));
        assert!(error("debug set").starts_with("Usage: /debug"));
        assert!(error("debug set teapot 3").contains("/debug set teapot"));
        assert!(error("debug set weapon 12").contains("0 to 4 or 6 to 11"));
        assert!(error("debug set wumpa x").contains("<number>"));
        assert!(error("join-lobby abc").contains("Unknown command: /join-lobby"));
        assert!(error("lobby join nonsense").starts_with("Not a lobby address"));
    }

    #[test]
    fn promote_takes_a_player() {
        assert!(matches!(
            Action::parse("promote  Penta "),
            Ok(Action::Promote(name)) if name == "Penta"
        ));
        assert!(matches!(
            Action::parse("promote Pen ta"),
            Ok(Action::Promote(name)) if name == "Pen ta"
        ));
        assert!(Action::parse("promote").unwrap_err().starts_with("Usage: /promote"));
    }
}
