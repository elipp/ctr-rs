//! The command line: Return opens it, typing and pasting (Cmd or Ctrl V) fill it in, Return again
//! sends what is typed and Escape closes it without. A line that starts with `/` is a command
//! (an [`Action`], which the keys can ask for as well); any other is a chat message.
//!
//! While it is open it takes the keyboard: the keys are cleared from the game's input every frame,
//! so that typing does not drive the kart.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use net::Ticket;

use crate::lobby::Lobby;
use crate::toast::Toast;

/// Something a player can ask for: a key does it, and so does its `/command`. Both only send it;
/// what it does is up to whoever handles it (the lobby's are in `lobby.rs`).
#[derive(Message, Clone, Debug)]
pub enum Action {
    CreateLobby,
    JoinLobby(Ticket),
    CloseLobby,
    StartRace,
    RestartRace,
}

impl Action {
    /// The action a line without its `/` asks for, or what is wrong with it.
    fn parse(line: &str) -> Result<Action, String> {
        let (name, argument) = line.split_once(' ').unwrap_or((line, ""));
        match name {
            "create-lobby" => Ok(Action::CreateLobby),
            "join-lobby" => argument
                .trim()
                .parse()
                .map(Action::JoinLobby)
                .map_err(|e| format!("Not a lobby address: {e}")),
            "close-lobby" => Ok(Action::CloseLobby),
            "start-race" => Ok(Action::StartRace),
            "restart-race" => Ok(Action::RestartRace),
            _ => Err(format!("Unknown command: /{name}")),
        }
    }
}

const FONT_SIZE: f32 = 20.0;

/// The seconds a cursor stays on, and off.
const BLINK_SECONDS: f32 = 0.5;

/// What has been typed so far, if the line is open.
#[derive(Resource, Default)]
struct CommandLine {
    open: bool,
    text: String,
    /// Return was pressed: `submit` sends the line.
    entered: bool,
}

#[derive(Component)]
struct CommandText;

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandLine>()
            .add_message::<Action>()
            .add_systems(PreUpdate, capture.after(InputSystems))
            .add_systems(Update, (open, bindings, submit, show).chain());
    }
}

/// Reads the typing while the line is open, and keeps it from the game.
fn capture(
    mut line: ResMut<CommandLine>,
    mut events: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
) {
    if !line.open {
        // (What was typed before it opened is not for it.)
        events.clear();
        return;
    }
    let command = keys.any_pressed([
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
    ]);
    for event in events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        match &event.logical_key {
            Key::Enter => line.entered = true,
            Key::Escape => {
                line.open = false;
                line.text.clear();
            }
            // A named key, so it is not matched as typed text.
            Key::Space => line.text.push(' '),
            Key::Backspace => {
                line.text.pop();
            }
            _ if command => {
                if event.key_code == KeyCode::KeyV {
                    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        Ok(text) => line
                            .text
                            .extend(text.chars().filter(|c| !c.is_whitespace())),
                        Err(e) => error!("could not read the clipboard: {e}"),
                    }
                }
            }
            _ => {
                if let Some(text) = &event.text {
                    line.text.extend(
                        text.chars()
                            .filter(|c| !c.is_control() && !c.is_whitespace()),
                    );
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

/// The keys that ask for an action: L opens a lobby, or closes the one we are in, G starts the
/// race and F9 starts it over.
fn bindings(
    keys: Res<ButtonInput<KeyCode>>,
    lobby: Res<Lobby>,
    mut actions: MessageWriter<Action>,
) {
    if keys.just_pressed(KeyCode::KeyL) {
        actions.write(if lobby.is_open() {
            Action::CloseLobby
        } else {
            Action::CreateLobby
        });
    }
    if keys.just_pressed(KeyCode::KeyG) {
        actions.write(Action::StartRace);
    }
    if keys.just_pressed(KeyCode::F9) {
        actions.write(Action::RestartRace);
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
