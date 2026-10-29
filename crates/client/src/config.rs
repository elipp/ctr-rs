//! The player's settings, kept in `config.json` (see `paths.rs`): read when the viewer starts, and
//! written whenever one changes.
//!
//! * `/config set nickname [name]`: what the others call us instead of our key (up to
//!   [`net::NICK_MAX`] characters; none clears it). It can change at any time, in a race too, but
//!   only every [`net::NICK_COOLDOWN`]
//!
//! The last character driven is kept too, and is the one the client starts with.
//!
//! To add a setting: a field of [`Config`], a variant of [`Setting`] with its parse, and its arm
//! in [`apply`].

use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command::{Action, word};
use crate::lobby::Lobby;
use crate::toast::Toast;

#[derive(Resource, Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// What we are called in a lobby, empty to be known by our key (see `net::clean_nick`).
    pub nickname: String,
    /// The last character driven (`CHARACTERS` order).
    pub character: Option<u32>,
    /// The disc image (see `disc.rs`), once a picked or written one has proved to be a disc.
    pub iso: Option<PathBuf>,
}

fn path() -> PathBuf {
    crate::paths::config_file()
}

impl Config {
    /// What the file holds; the defaults if there is none or it cannot be read (it is told of).
    pub fn load() -> Config {
        let text = match std::fs::read_to_string(path()) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::default(),
            Err(e) => {
                eprintln!("{}: {e}", path().display());
                return Config::default();
            }
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut config) => {
                config.nickname = net::clean_nick(&config.nickname);
                config
            }
            Err(e) => {
                eprintln!("{}: {e}", path().display());
                Config::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path();
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, serde_json::to_string_pretty(self)?)
        };
        write().map_err(|e| format!("Could not save {}: {e}", path.display()))
    }

    /// Remembers the character we drive, if it is another.
    pub fn set_character(&mut self, character: u32) {
        if self.character != Some(character) {
            self.character = Some(character);
            if let Err(why) = self.save() {
                eprintln!("{why}");
            }
        }
    }
}

/// A setting to change, as `/config set` names it.
#[derive(Clone, Debug)]
pub enum Setting {
    /// Already cleaned (see `net::clean_nick`).
    Nickname(String),
}

const USAGE: &str = "Usage: /config set nickname [name, up to 12 characters]";

impl Setting {
    /// The setting a line after `/config set` names, or what is wrong with it.
    pub fn parse(line: &str) -> Result<Setting, String> {
        let (key, value) = word(line);
        match key {
            "nickname" => {
                if value.chars().count() > net::NICK_MAX {
                    return Err(format!("A nickname is up to {} characters", net::NICK_MAX));
                }
                if value.chars().any(char::is_control) {
                    return Err("A nickname cannot hold control characters".into());
                }
                Ok(Setting::Nickname(value.to_string()))
            }
            "" => Err(USAGE.into()),
            _ => Err(format!("Unknown setting: {key}\n{USAGE}")),
        }
    }
}

pub struct ConfigPlugin;

impl Plugin for ConfigPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, apply);
    }
}

/// Does what `/config set` asks: changes the setting, keeps it, and tells whoever it concerns.
fn apply(
    mut actions: MessageReader<Action>,
    mut config: ResMut<Config>,
    lobby: Res<Lobby>,
    time: Res<Time<Real>>,
    mut nickname_changed: Local<Option<f32>>,
    mut toasts: MessageWriter<Toast>,
) {
    let now = time.elapsed_secs();
    for action in actions.read() {
        let Action::SetConfig(setting) = action else {
            continue;
        };
        let refuse = |why: String| Toast::new(why, 3000).color(Color::srgb(1.0, 0.4, 0.4));
        match setting {
            Setting::Nickname(nick) => {
                if *nick == config.nickname {
                    toasts.write(Toast::new("That is your nickname already", 2000));
                    continue;
                }
                let cooldown = net::NICK_COOLDOWN.as_secs_f32();
                if let Some(at) = *nickname_changed
                    && now - at < cooldown
                {
                    let wait = (cooldown - (now - at)).ceil();
                    toasts.write(refuse(format!("You can change your nickname in {wait} s")));
                    continue;
                }
                config.nickname = nick.clone();
                // (It holds for this session even if the file cannot be written.)
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                *nickname_changed = Some(now);
                // The others hear of it, in a race too.
                lobby.set_nickname(nick);
                let text = if nick.is_empty() {
                    "Nickname cleared".to_string()
                } else {
                    format!("Nickname: {nick}")
                };
                toasts.write(Toast::new(text, 2500));
            }
        }
    }
}
