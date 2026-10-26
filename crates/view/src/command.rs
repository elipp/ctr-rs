//! The command line: Return opens it, typing and pasting (Cmd or Ctrl V) fill it in, Return again
//! sends what is typed and Escape closes it without. A line that starts with `/` is a command
//! (an [`Action`]); any other is a chat message.
//!
//! While it is open it takes the keyboard: the keys are cleared from the game's input every frame,
//! so that typing does not drive the kart.

use std::collections::VecDeque;

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use net::Ticket;

use crate::toast::Toast;

/// Something a player can ask for with a `/command`. The command only sends it; what it does is
/// up to whoever handles it (the lobby's are in `lobby.rs`).
#[derive(Message, Clone, Debug)]
pub enum Action {
    CreateLobby,
    JoinLobby(Ticket),
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
    /// (Debug) Toast how each peer is connected: direct or by relay, the round trip time, losses.
    DebugNetInfo,
}

/// The first word of `text` and what follows it.
fn word(text: &str) -> (&str, &str) {
    let text = text.trim();
    text.split_once(char::is_whitespace)
        .map_or((text, ""), |(w, rest)| (w, rest.trim()))
}

const LOBBY_USAGE: &str = "Usage: /lobby create | join <address> | close";
const CACHE_USAGE: &str = "Usage: /cache purge";
const RACE_USAGE: &str = "Usage: /race start | restart | stop";
const DEBUG_USAGE: &str =
    "Usage: /debug set wumpa <0 to 10> | set weapon <0 to 4, 6, 7, 10 or 11> | netinfo";

impl Action {
    /// The action a line without its `/` asks for, or what is wrong with it. Commands are a name
    /// and a subcommand (`/lobby join <address>`), which may have subcommands of its own
    /// (`/debug set wumpa 10`).
    fn parse(line: &str) -> Result<Action, String> {
        let (name, rest) = word(line);
        match name {
            "lobby" => Action::lobby(rest),
            "race" => Action::race(rest),
            "cache" => Action::cache(rest),
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
            "close" => Ok(Action::CloseLobby),
            "" => Err(LOBBY_USAGE.into()),
            _ => Err(format!("Unknown command: /lobby {sub}\n{LOBBY_USAGE}")),
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
            "netinfo" => Ok(Action::DebugNetInfo),
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
struct CommandLine {
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
        f(&mut self.text);
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
fn capture(
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
            Key::Enter => line.entered = true,
            Key::Escape => {
                line.open = false;
                line.text.clear();
                line.browsing = None;
                line.draft.clear();
            }
            Key::ArrowUp => line.older(),
            Key::ArrowDown => line.newer(),
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
    info!("(chat, not sent) {message}");
    toasts.write(Toast::new("Chat is not implemented yet", 2000));
}

/// The line on screen while it is open.
fn show(
    line: Res<CommandLine>,
    time: Res<Time<Real>>,
    mut commands: Commands,
    mut texts: Query<(Entity, &mut Text), With<CommandText>>,
) {
    let existing = texts.iter_mut().next();
    if !line.open {
        if let Some((entity, _)) = existing {
            commands.entity(entity).despawn();
        }
        return;
    }
    let cursor = if ((time.elapsed_secs() / BLINK_SECONDS) as u32).is_multiple_of(2) {
        "_"
    } else {
        " "
    };
    let line = format!("> {}{cursor}", line.text);
    match existing {
        Some((_, mut text)) => text.0 = line,
        None => {
            commands.spawn((
                CommandText,
                Text::new(line),
                TextFont {
                    font_size: FONT_SIZE.into(),
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(40.0),
                    left: Val::Px(20.0),
                    right: Val::Px(20.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::BLACK),
                // Over every other piece of the HUD.
                GlobalZIndex(i32::MAX),
            ));
        }
    }
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
        assert!(matches!(
            Action::parse("debug netinfo"),
            Ok(Action::DebugNetInfo)
        ));
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
