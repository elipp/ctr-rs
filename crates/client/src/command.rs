//! The command line: Return opens it, typing and pasting (Cmd or Ctrl V) fill it in, Return again
//! sends what is typed and Escape closes it without. A line that starts with `/` is a command
//! (an [`Action`]); any other is a chat message.
//!
//! While a command is typed, what can come next is listed above the line (`suggest.rs`): Up and
//! Down pick one, Tab puts it in the line, and Return does too, sending the line if nothing can
//! follow it. Otherwise Up and Down go through the lines sent before.
//!
//! While it is open it takes the keyboard: the keys are cleared from the game's input every frame,
//! so that typing does not drive the kart.

use std::collections::VecDeque;

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use net::Ticket;

use crate::suggest::Menu;
use crate::toast::Toast;

/// Something a player can ask for with a `/command`. The command only sends it; what it does is
/// up to whoever handles it (the lobby's are in `lobby.rs`).
#[derive(Message, Clone, Debug)]
pub enum Action {
    CreateLobby,
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

const LOBBY_USAGE: &str = "Usage: /lobby create | join <address> | spectate [address] | close";
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
            "net" => Action::net(rest),
            "debug" => Action::debug(rest),
            _ => Err(format!("Unknown command: /{name}")),
        }
    }

    fn lobby(line: &str) -> Result<Action, String> {
        let (sub, rest) = word(line);
        match sub {
            "create" => Ok(Action::CreateLobby),
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
}

impl CommandLine {
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
        self.browsing = Some(index);
        self.text = self.history[index].clone();
    }

    /// Down: the line sent after the one shown, and past the last, what was being typed.
    fn newer(&mut self) {
        let Some(i) = self.browsing else { return };
        if i + 1 < self.history.len() {
            self.browsing = Some(i + 1);
            self.text = self.history[i + 1].clone();
        } else {
            self.browsing = None;
            self.text = std::mem::take(&mut self.draft);
        }
    }

    /// Typing edits what is shown, which is the text from then on.
    fn edit(&mut self, f: impl FnOnce(&mut String)) {
        self.browsing = None;
        self.selected = 0;
        f(&mut self.text);
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
        self.edit(|t| *t = line);
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

#[derive(Component)]
struct CommandText;

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandLine>()
            .add_message::<Action>()
            .add_systems(PreUpdate, capture.after(InputSystems))
            .add_systems(Update, (open, submit, show).chain());
    }
}

/// Reads the typing while the line is open, and keeps it from the game.
pub(crate) fn capture(
    mut line: ResMut<CommandLine>,
    mut events: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut command: Local<bool>,
) {
    // The modifier is followed here: `keys` is wiped below, so it cannot say whether Cmd is held.
    let mut typed = Vec::new();
    for event in events.read() {
        if matches!(event.logical_key, Key::Super | Key::Control) {
            *command = event.state == ButtonState::Pressed;
        }
        typed.push(event.clone());
    }
    if !line.open {
        // (What was typed before it opened is not for it.)
        return;
    }
    for event in &typed {
        if event.state != ButtonState::Pressed {
            continue;
        }
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
            Key::Backspace => line.edit(|t| {
                t.pop();
            }),
            _ if *command => {
                if event.key_code == KeyCode::KeyV {
                    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        Ok(text) => {
                            line.edit(|t| t.extend(text.chars().filter(|c| !c.is_control())))
                        }
                        Err(e) => error!("could not read the clipboard: {e}"),
                    }
                }
            }
            _ => {
                if let Some(text) = &event.text {
                    line.edit(|t| t.extend(text.chars().filter(|c| !c.is_control())));
                }
            }
        }
    }
    keys.reset_all();
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
    mut toasts: MessageWriter<Toast>,
) {
    if !std::mem::take(&mut line.entered) {
        return;
    }
    line.open = false;
    let text = std::mem::take(&mut line.text);
    if text.is_empty() {
        return;
    }
    line.remember(&text);
    let Some(command) = text.strip_prefix('/') else {
        chat(&text, &mut toasts);
        return;
    };
    match Action::parse(command) {
        Ok(action) => {
            actions.write(action);
        }
        Err(why) => {
            toasts.write(Toast::new(why, 3000).color(Color::srgb(1.0, 0.4, 0.4)));
        }
    }
}

/// A chat message, for the others in the lobby to read. (Stub: nothing is sent yet.)
fn chat(message: &str, toasts: &mut MessageWriter<Toast>) {
    debug!("(chat, not sent) {message}");
    toasts.write(Toast::new("Chat is not implemented yet", 2000));
}

/// The colours of the suggestions: the one picked, the others, and what they do.
const PICKED: Color = Color::srgb(1.0, 0.75, 0.3);
const OPTION: Color = Color::srgb(0.9, 0.9, 0.9);
const ABOUT: Color = Color::srgb(0.6, 0.6, 0.6);

/// The line and its suggestions, at the bottom of the screen.
#[derive(Component)]
struct CommandBox;

/// The suggestions over the line.
#[derive(Component)]
struct Suggestions;

/// The line on screen while it is open, and over it what can be typed next.
fn show(
    line: Res<CommandLine>,
    time: Res<Time<Real>>,
    mut commands: Commands,
    boxes: Query<Entity, With<CommandBox>>,
    suggestions: Query<Entity, With<Suggestions>>,
    mut texts: Query<&mut Text, With<CommandText>>,
    // What the suggestions were made for: the text and the one picked.
    mut shown: Local<Option<(String, usize)>>,
) {
    if !line.open {
        for entity in &boxes {
            commands.entity(entity).despawn();
        }
        *shown = None;
        return;
    }
    let cursor = if ((time.elapsed_secs() / BLINK_SECONDS) as u32).is_multiple_of(2) {
        "_"
    } else {
        " "
    };
    let text = format!("> {}{cursor}", line.text);
    let Ok(list) = suggestions.single() else {
        commands.spawn((
            CommandBox,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(40.0),
                left: Val::Px(20.0),
                right: Val::Px(20.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            // Over every other piece of the HUD.
            GlobalZIndex(i32::MAX),
            children![
                (
                    Suggestions,
                    Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::BLACK.with_alpha(0.75)),
                    Visibility::Hidden,
                ),
                (
                    CommandText,
                    Text::new(text),
                    TextFont {
                        font_size: FONT_SIZE.into(),
                        ..default()
                    },
                    Node {
                        padding: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(Color::BLACK),
                ),
            ],
        ));
        return;
    };
    for mut shown_text in &mut texts {
        shown_text.0 = text.clone();
    }
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
    for (name, about, color) in rows {
        commands.spawn((
            Text::new(name),
            TextFont {
                font_size: FONT_SIZE.into(),
                ..default()
            },
            TextColor(color),
            ChildOf(list),
            children![(
                TextSpan::new(about),
                TextFont {
                    font_size: FONT_SIZE.into(),
                    ..default()
                },
                TextColor(ABOUT),
            )],
        ));
    }
}

/// The rows of suggestions for the line: each option, the picked one in its own colour, and
/// what else can be typed there; the names padded to line up what they do.
fn suggestion_rows(line: &CommandLine) -> Vec<(String, String, Color)> {
    let Some(menu) = line.menu() else {
        return Vec::new();
    };
    let picked = line.selection().map(|(_, index)| index);
    let width = menu
        .options
        .iter()
        .map(|o| o.name.chars().count())
        .chain(menu.arg.map(|(arg, _)| arg.chars().count()))
        .max()
        .unwrap_or(0);
    let row = |name: &str, about: &str, color| (format!("{name:width$}   "), about.into(), color);
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
            Ok(Action::CreateLobby)
        ));
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
            Action::parse("config set volume music 40"),
            Ok(Action::SetConfig(crate::config::Setting::VolumeMusic(40)))
        ));
        assert!(matches!(
            Action::parse("config set volume fx 0"),
            Ok(Action::SetConfig(crate::config::Setting::VolumeFx(0)))
        ));
        assert!(Action::parse("config set volume fx 101").is_err());
        assert!(Action::parse("config set volume fx").is_err());
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
