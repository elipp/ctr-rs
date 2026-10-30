//! The player's settings, kept in `config.json` (see `paths.rs`): read when the viewer starts, and
//! written whenever one changes.
//!
//! * `/config set nickname [name]`: what the others call us instead of our key (up to
//!   [`net::NICK_MAX`] characters; none clears it). It can change at any time, in a race too, but
//!   only every [`net::NICK_COOLDOWN`]
//! * `/config set vsync on|off`: whether the window waits for the display (on at first)
//! * `/config set fps-max <number>`: the most frames a second to draw, 0 for no limit (the
//!   default); the sim has its own clock, see `frame.rs`
//!
//! * `/config set volume music|fx <0-100>`: how loud the music and the effects (the voices too)
//!   are, in percent of the game's own volumes (100 at first)
//!
//! The last character driven is kept too, and is the one the client starts with.
//!
//! To add a setting: a field of [`Config`], a variant of [`Setting`] with its parse, its arm in
//! [`apply`], its line in [`USAGE`], and its entry in the command line's suggestions
//! (`suggest.rs`, which does not read the parser: nothing but the reader notices a miss).

use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::command::{Action, word};
use crate::lobby::Lobby;
use crate::toast::Toast;

#[derive(Resource, Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// What we are called in a lobby, empty to be known by our key (see `net::clean_nick`).
    pub nickname: String,
    /// The last character driven (`CHARACTERS` order).
    pub character: Option<u32>,
    /// The disc image (see `disc.rs`), once a picked or written one has proved to be a disc.
    pub iso: Option<PathBuf>,
    /// Whether the window waits for the display to draw (`/config set vsync`).
    pub vsync: bool,
    /// The most frames a second to draw, 0 for no limit (`/config set fps-max`).
    pub fps_max: u32,
    /// The effects' and the music's volumes, 0 to 100 (`/config set volume`).
    pub volume_fx: u8,
    pub volume_music: u8,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            nickname: String::new(),
            character: None,
            iso: None,
            vsync: true,
            fps_max: 0,
            volume_fx: 100,
            volume_music: 100,
        }
    }
}

impl Config {
    /// How the window presents what it draws.
    pub fn present_mode(&self) -> bevy::window::PresentMode {
        if self.vsync {
            bevy::window::PresentMode::AutoVsync
        } else {
            bevy::window::PresentMode::AutoNoVsync
        }
    }
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
    Vsync(bool),
    /// 0 for no limit.
    FpsMax(u32),
    /// 0 to 100.
    VolumeFx(u8),
    VolumeMusic(u8),
}

const USAGE: &str = "Usage: /config set nickname [name, up to 12 characters]\n\
    /config set vsync on|off\n\
    /config set fps-max <frames a second, 0 for no limit>\n\
    /config set volume music|fx <0-100>";

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
            "vsync" => match value {
                "on" => Ok(Setting::Vsync(true)),
                "off" => Ok(Setting::Vsync(false)),
                _ => Err("Usage: /config set vsync on|off".into()),
            },
            "fps-max" => value.parse().map(Setting::FpsMax).map_err(|_| {
                "Usage: /config set fps-max <frames a second, 0 for no limit>".to_string()
            }),
            "volume" => {
                const USAGE: &str = "Usage: /config set volume music|fx <0-100>";
                let (kind, percent) = word(value);
                let percent: u8 = percent
                    .parse()
                    .ok()
                    .filter(|p| *p <= 100)
                    .ok_or_else(|| USAGE.to_string())?;
                match kind {
                    "music" => Ok(Setting::VolumeMusic(percent)),
                    "fx" => Ok(Setting::VolumeFx(percent)),
                    _ => Err(USAGE.into()),
                }
            }
            "" => Err(USAGE.into()),
            _ => Err(format!("Unknown setting: {key}\n{USAGE}")),
        }
    }
}

pub struct ConfigPlugin;

impl Plugin for ConfigPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (apply, present_mode));
    }
}

/// The windows present as the config says, when it changes.
fn present_mode(config: Res<Config>, mut windows: Query<&mut Window>) {
    if !config.is_changed() {
        return;
    }
    for mut window in &mut windows {
        if window.present_mode != config.present_mode() {
            window.present_mode = config.present_mode();
        }
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
            Setting::Vsync(on) => {
                config.vsync = *on;
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                toasts.write(Toast::new(
                    format!("Vsync {}", if *on { "on" } else { "off" }),
                    2000,
                ));
            }
            Setting::FpsMax(max) => {
                config.fps_max = *max;
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                let text = match max {
                    0 => "Frame rate: no limit".to_string(),
                    max => format!("Frame rate: at most {max} a second"),
                };
                toasts.write(Toast::new(text, 2000));
            }
            Setting::VolumeFx(percent) | Setting::VolumeMusic(percent) => {
                let name = if matches!(setting, Setting::VolumeFx(_)) {
                    config.volume_fx = *percent;
                    "Effects"
                } else {
                    config.volume_music = *percent;
                    "Music"
                };
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                toasts.write(Toast::new(format!("{name} volume: {percent}"), 2000));
            }
        }
    }
}
