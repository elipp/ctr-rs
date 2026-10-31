//! The command line: Return opens it, typing and pasting (Cmd or Ctrl V) fill it in, Return again
//! sends what is typed and Escape closes it without. A line that starts with `/` is a command
//! (an [`Action`]); any other is a chat message.
//!
//! While a command is typed, what can come next is listed above the line (`suggest.rs`): Up and
//! Down pick one, Tab puts it in the line, and Return does too, sending the line if nothing can
//! follow it. Otherwise Up and Down go through the lines sent before.
//!
//! The line is edited as in a shell: the arrows, Home and End move the cursor, and Ctrl A, E, B, F
//! (Alt B, F by a word), Ctrl W, U, K, D, H and Alt D delete. Page
//! Up and Down scroll the console (`console.rs`), which the line is the bottom row of.
//!
//! While it is open it takes the keyboard: the keys are cleared from the game's input every frame,
//! so that typing does not drive the kart.

use std::collections::VecDeque;

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::text::TextBackgroundColor;
use net::Ticket;

use crate::console::{Console, Log, PAGE};
use crate::suggest::Menu;

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
    /// Delete the maps other players sent us (`maps::purge`).
    PurgeCache,
    StartRace,
    RestartRace,
    /// End the race in progress for everyone, with no winner (the host only).
    StopRace,
    /// (Debug) Roll that item into our hand: one of `vehicle::item::ENABLED`.
    DebugAddWeapon(u8),
    /// (Debug) Set our wumpa count, clamped to what a kart can hold.
    DebugSetWumpa(u8),
    /// (Debug) Freeze this client for that long, online only: the others wait for it, and the
    /// host removes it after `net::KICK_AFTER`.
    DebugStall(std::time::Duration),
    /// (Debug) Switch between the race and the fly-through viewer with the collision probe
    /// (`main.rs`).
    DebugViewer,
    /// (Debug) Show or hide the development text of the race (position, speed, reserves), or
    /// toggle it (`None`). The frame rate stays.
    DebugHud(Option<bool>),
    /// (Debug) Change how the level is built, or toast how it is (`None`).
    DebugLevel(Option<MeshSetting>),
    /// (Debug) Set the depth bias of the crates, or toast it (`None`).
    DebugCrateBias(Option<f32>),
    /// (Debug) Try a visual effect, or open the panel of them (`fx.rs`).
    DebugFx(crate::fx::Fx),
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

const LOBBY_USAGE: &str = "Usage: /lobby create [public [name]] | join <address> | spectate [address] | close";
const CONFIG_USAGE: &str = "Usage: /config set <setting> <value>, like /config set nickname Penta";
const CACHE_USAGE: &str = "Usage: /cache purge";
const RACE_USAGE: &str = "Usage: /race start | restart | stop";
const DEBUG_USAGE: &str = "Usage: /debug set wumpa <0 to 10> | set weapon <0 to 4, 6, 7, 10 or 11> | stall <duration, like 3s> | viewer | hud [on|off] | level hidden|cull|colors|crate-fixup | crate bias <number> | fx [...]";
const NET_USAGE: &str =
    "Usage: /net info | smoothing ping|replay|<duration, like 120ms> | rollbacks on|off";

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
            "cache" => Action::cache(rest),
            "config" => Action::config(rest),
            "version" if rest.is_empty() => Ok(Action::Version),
            "version" => Err("Usage: /version".into()),
            "net" => Action::net(rest),
            "debug" => Action::debug(rest),
            _ => Err(format!("Unknown command: /{name}")),
        }
    }

    fn lobby(line: &str) -> Result<Action, String> {
        let (sub, rest) = word(line);
        match sub {
            "create" => match word(rest) {
                ("", _) => Ok(Action::CreateLobby { public: None }),
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

    fn cache(line: &str) -> Result<Action, String> {
        let (sub, _) = word(line);
        match sub {
            "purge" => Ok(Action::PurgeCache),
            "" => Err(CACHE_USAGE.into()),
            _ => Err(format!("Unknown command: /cache {sub}\n{CACHE_USAGE}")),
        }
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
                            "Usage: /debug set weapon <0 to 4, 6, 7, 10 or 11>".to_string()
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
            "viewer" => Ok(Action::DebugViewer),
            "hud" => match rest {
                "" => Ok(Action::DebugHud(None)),
                "on" => Ok(Action::DebugHud(Some(true))),
                "off" => Ok(Action::DebugHud(Some(false))),
                _ => Err(DEBUG_USAGE.into()),
            },
            "level" => Action::level(rest),
            "fx" => Action::fx(rest).map(Action::DebugFx),
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
    /// Where the cursor is in the text: a byte offset, always at a character.
    cursor: usize,
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
    }

    /// Replaces the text, with the cursor after it.
    fn set_text(&mut self, text: String) {
        self.touch();
        self.cursor = text.len();
        self.text = text;
    }

    /// Types at the cursor.
    fn insert(&mut self, typed: &str) {
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
        head.rfind(char::is_whitespace)
            .map_or(0, |i| i + head[i..].chars().next().map_or(1, char::len_utf8))
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
        true
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

/// The line and the suggestions over it, which are shown only while the line is open.
#[derive(Component)]
struct InputRow;

/// The suggestions over the line.
#[derive(Component)]
struct Suggestions;

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandLine>()
            .add_message::<Action>()
            .add_systems(PreUpdate, capture.after(InputSystems))
            .add_systems(Update, (open, submit, show).chain());
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
    for (event, Modifiers { ctrl, alt, sup }) in &typed {
        if event.state != ButtonState::Pressed {
            continue;
        }
        let (ctrl, alt, sup) = (*ctrl, *alt, *sup);
        match &event.logical_key {
            Key::Enter => {
                // A suggestion picked goes in first; the line is sent if that ends it.
                if line.selection().is_none() || line.complete() {
                    line.entered = true;
                }
            }
            Key::Tab => {
                line.complete();
            }
            Key::Escape => {
                line.open = false;
                line.text.clear();
                line.cursor = 0;
                line.browsing = None;
                line.draft.clear();
                line.selected = 0;
            }
            Key::ArrowUp => {
                if !line.step(false) {
                    line.older();
                }
            }
            Key::ArrowDown => {
                if !line.step(true) {
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
fn open(keys: Res<ButtonInput<KeyCode>>, mut line: ResMut<CommandLine>) {
    if !line.open && keys.just_pressed(KeyCode::Enter) {
        line.open = true;
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
                Suggestions,
                // Over the log, right above the line.
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Percent(100.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    // Two columns, the names and what they do: a row's two texts line up with the
                    // others', and what they do wraps in its own column.
                    display: Display::Grid,
                    grid_template_columns: vec![GridTrack::auto(), GridTrack::flex(1.0)],
                    column_gap: Val::Px(16.0),
                    row_gap: Val::Px(6.0),
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::BLACK),
                Visibility::Hidden,
            ),
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
                BackgroundColor(Color::BLACK),
                children![
                    (
                        CursorSpan,
                        TextSpan::new(" "),
                        font.clone(),
                        TextBackgroundColor(Color::NONE),
                    ),
                    (AfterSpan, TextSpan::new(""), font),
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
    suggestions: Query<Entity, With<Suggestions>>,
    mut prompt: Query<&mut Text, With<CommandText>>,
    mut cursor: Query<
        (&mut TextSpan, &mut TextColor, &mut TextBackgroundColor),
        With<CursorSpan>,
    >,
    mut after: Query<&mut TextSpan, (With<AfterSpan>, Without<CursorSpan>)>,
    // What the suggestions were made for: the text and the one picked.
    mut shown: Local<Option<(String, usize)>>,
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
    let under = rest.chars().next().map_or(" ".to_string(), String::from);
    let blink_on = ((time.elapsed_secs() / BLINK_SECONDS) as u32).is_multiple_of(2);
    for mut text in &mut prompt {
        assign(&mut text.0, &format!("> {before}"));
    }
    for (mut span, mut color, mut background) in &mut cursor {
        assign(&mut span.0, &under);
        let (ink, back) = if blink_on {
            (Color::BLACK, Color::WHITE)
        } else {
            (Color::WHITE, Color::NONE)
        };
        if color.0 != ink {
            color.0 = ink;
        }
        if background.0 != back {
            background.0 = back;
        }
    }
    for mut span in &mut after {
        assign(&mut span.0, rest.chars().next().map_or("", |c| &rest[c.len_utf8()..]));
    }
    let Ok(list) = suggestions.single() else {
        return;
    };
    let key = (line.text.clone(), line.selected);
    if shown.as_ref() == Some(&key) {
        return;
    }
    *shown = Some(key);
    commands.entity(list).despawn_children();
    let rows = suggestion_rows(&line);
    commands.entity(list).insert(if rows.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
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

/// The rows of suggestions for the line: each option, the picked one in its own colour, and
/// what else can be typed there; and what they do.
fn suggestion_rows(line: &CommandLine) -> Vec<(String, String, Color)> {
    let Some(menu) = line.menu() else {
        return Vec::new();
    };
    let picked = line.selection().map(|(_, index)| index);
    let row = |name: &str, about: &str, color| (name.to_string(), about.to_string(), color);
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
        assert!(Action::parse("lobby create private").is_err());
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
        assert!(matches!(Action::parse("race start"), Ok(Action::StartRace)));
        assert!(matches!(
            Action::parse("race restart"),
            Ok(Action::RestartRace)
        ));
        assert!(matches!(Action::parse("race stop"), Ok(Action::StopRace)));
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
        assert!(Action::parse("debug stall 5m").is_err());
        assert!(Action::parse("debug stall 61s").is_err());
        assert!(Action::parse("debug netinfo").is_err());
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
            Action::parse("config set relay-only on"),
            Ok(Action::SetConfig(crate::config::Setting::RelayOnly(true)))
        ));
        assert!(Action::parse("config set relay-only maybe").is_err());
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
        assert!(matches!(Action::parse("version"), Ok(Action::Version)));
        assert!(Action::parse("version 1").is_err());
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
        assert!(error("debug set weapon 12").contains("0 to 4, 6, 7, 10 or 11"));
        assert!(error("debug set wumpa x").contains("<number>"));
        assert!(error("join-lobby abc").contains("Unknown command: /join-lobby"));
        assert!(error("lobby join nonsense").starts_with("Not a lobby address"));
    }
}
