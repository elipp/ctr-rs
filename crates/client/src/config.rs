//! The player's settings, kept in `config.json` (see `paths.rs`), or in a browser in the page's
//! storage (`web.rs`): read when the viewer starts, and written whenever one changes.
//!
//! * `/config set nickname [name]`: what the others call us instead of our key (up to
//!   [`net::NICK_MAX`] characters; none clears it). It can change at any time, in a race too, but
//!   only every [`net::NICK_COOLDOWN`]
//! * `/config set vsync on|off`: whether the window waits for the display (on at first)
//! * `/config set fps-max <number>`: the most frames a second to draw, 0 for no limit (the
//!   default); the sim has its own clock, see `frame.rs`
//! * `/config set msaa off|2|4`: how many samples of anti-aliasing the cameras draw with (off at
//!   first: the picture is pixel art, and each camera's pass pays for it). It holds at once
//! * `/config set shader-effects on|off`: whether to draw the passes over the whole picture, the
//!   heat shimmer's (`heat.rs`) and the clock's trail (`clock.rs`) (on at first)
//! * `/config set volume music|sfx|voice <0-100>`: how loud the music, the effects and the
//!   voices are, in percent of the game's own volumes (100 at first). The pause menu's Options
//!   set them too
//!
//! * `/config set render-scale <0.25-1>`: in a browser, the part of the screen's resolution to
//!   draw at, across and down (`web/play.html`)
//!
//! Two network settings are kept here too, and set with `/net`:
//!
//! * `/net relay only on|off`: whether to reach the other players through the relay only, never
//!   straight, so that they do not learn our address (off at first; it costs latency, and holds
//!   from the next lobby we create or join)
//! * `/net relay add <url>`: another relay to use besides the public ones (each once, at most
//!   [`MAX_RELAYS`]; it holds from the next lobby), and `/net relay clear` to drop them all
//!
//! A browser has no vsync or fps-max, and no relay-only: it paces the frames to the display
//! itself, and reaches the others through the relay only.
//!
//! The last character driven is kept too, and is the one the client starts with, and the
//! keyboard's driving keys, which the pause menu's Controls change (`controls.rs`), and whether
//! the race shows its development text (`/debug hud`, off at first).
//!
//! The cups made with `/cup create` are kept here too, with the tracks by their short names, and
//! can be written by hand (`lobby/cup.rs`).
//!
//! To add a setting: a field of [`Config`], a variant of [`Setting`] with its parse, its arm in
//! [`apply`], its line in [`USAGE`], and its entry in the command line's suggestions
//! (`suggest.rs`, which does not read the parser: nothing but the reader notices a miss).

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::{DebandDither, Tonemapping};
use serde::{Deserialize, Serialize};

use crate::command::{Action, word};
use crate::controls::Controls;
use crate::lobby::Lobby;
use crate::lobby::cup::Cups;
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
    /// The samples of multisampling per pixel, 1 (off), 2 or 4 (`/config set msaa`).
    pub msaa: u8,
    /// Whether the heat shimmer and the clock's trail are drawn (`/config set shader-effects`).
    pub shader_effects: bool,
    /// Whether to play through the relay only (`/net relay only`).
    pub relay_only: bool,
    /// The relays to use besides the public ones (`/net relay add`).
    pub relays: Vec<String>,
    /// The effects', the music's and the voices' volumes, 0 to 100 (`/config set volume`).
    pub volume_fx: u8,
    pub volume_music: u8,
    pub volume_voice: u8,
    /// The keyboard's driving keys.
    pub controls: Controls,
    /// Whether the race shows the frame rate, the kart's state, the build and the boost reserves
    /// (`/debug hud`).
    pub debug_hud: bool,
    /// The part of the screen's resolution a browser draws at (`/config set render-scale`), which
    /// the page reads by this name.
    pub render_scale: f32,
    /// The cups made (`/cup create`).
    pub cups: Cups,
}

/// The most relays that can be added; every one is probed when an endpoint starts.
const MAX_RELAYS: usize = 8;

impl Default for Config {
    fn default() -> Config {
        Config {
            nickname: String::new(),
            character: None,
            iso: None,
            vsync: true,
            fps_max: 0,
            msaa: 1,
            shader_effects: true,
            relay_only: false,
            relays: Vec::new(),
            volume_fx: 100,
            volume_music: 100,
            volume_voice: 100,
            controls: Controls::default(),
            debug_hud: false,
            render_scale: 1.0,
            cups: Cups::default(),
        }
    }
}

impl Config {
    /// The multisampling of the cameras.
    pub fn msaa(&self) -> Msaa {
        match self.msaa {
            2 => Msaa::Sample2,
            4 => Msaa::Sample4,
            _ => Msaa::Off,
        }
    }

    /// How the window presents what it draws.
    pub fn present_mode(&self) -> bevy::window::PresentMode {
        if self.vsync {
            bevy::window::PresentMode::AutoVsync
        } else {
            bevy::window::PresentMode::AutoNoVsync
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn path() -> PathBuf {
    crate::paths::config_file()
}

/// Where the settings are kept, to tell of a failure.
fn place() -> String {
    if cfg!(target_arch = "wasm32") {
        "the browser's storage".to_string()
    } else {
        crate::paths::config_file().display().to_string()
    }
}

/// The text of the settings, none if there are none yet.
#[cfg(not(target_arch = "wasm32"))]
fn read() -> Result<Option<String>, String> {
    match std::fs::read_to_string(path()) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", place())),
    }
}

/// The text of the settings, none if there are none yet (or the page cannot read them; it tells
/// the console why).
#[cfg(target_arch = "wasm32")]
fn read() -> Result<Option<String>, String> {
    Ok(crate::web::load_config())
}

#[cfg(not(target_arch = "wasm32"))]
fn write(text: &str) -> Result<(), String> {
    let path = path();
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, text)
    };
    write().map_err(|e| format!("Could not save {}: {e}", place()))
}

#[cfg(target_arch = "wasm32")]
fn write(text: &str) -> Result<(), String> {
    match crate::web::save_config(text) {
        None => Ok(()),
        Some(e) => Err(format!("Could not save the settings: {e}")),
    }
}

impl Config {
    /// What is kept; the defaults if there is nothing or it cannot be read (it is told of).
    pub fn load() -> Config {
        let text = match read() {
            Ok(Some(text)) => text,
            Ok(None) => return Config::default(),
            Err(why) => {
                eprintln!("{why}");
                return Config::default();
            }
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut config) => {
                config.nickname = net::clean_nick(&config.nickname);
                // Each relay once, as written the same way (the file may be edited by hand).
                let mut relays: Vec<String> = Vec::new();
                for url in config.relays.iter().filter_map(|u| net::parse_relay(u).ok()) {
                    if !relays.contains(&url) {
                        relays.push(url);
                    }
                }
                config.relays = relays;
                for cup in config.cups.take_invalid() {
                    eprintln!("{}: dropped the cup {cup:?}", place());
                }
                config
            }
            Err(e) => {
                eprintln!("{}: {e}", place());
                Config::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Could not save {}: {e}", place()))?;
        write(&text)
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
    /// 1 (off), 2 or 4 samples.
    Msaa(u8),
    ShaderEffects(bool),
    RelayOnly(bool),
    /// A relay's URL, as `net::parse_relay` keeps it.
    AddRelay(String),
    /// Back to the public relays only.
    ClearRelays,
    /// 0 to 100.
    VolumeSfx(u8),
    VolumeMusic(u8),
    VolumeVoice(u8),
    /// 0.25 to 1.
    RenderScale(f32),
}

const RENDER_SCALE_USAGE: &str = "Usage: /config set render-scale <0.25 to 1>";

/// The render scale that `value` says, if it is one.
fn render_scale(value: &str) -> Option<f32> {
    value
        .parse()
        .ok()
        .filter(|scale| (0.25..=1.0).contains(scale))
}

#[cfg(not(target_arch = "wasm32"))]
const USAGE: &str = "Usage: /config set nickname [name, up to 12 characters]\n\
    /config set vsync on|off\n\
    /config set fps-max <frames a second, 0 for no limit>\n\
    /config set msaa off|2|4\n\
    /config set shader-effects on|off\n\
    /config set volume music|sfx|voice <0-100>";
#[cfg(target_arch = "wasm32")]
const USAGE: &str = "Usage: /config set nickname [name, up to 12 characters]\n\
    /config set msaa off|2|4\n\
    /config set shader-effects on|off\n\
    /config set render-scale <0.25 to 1>\n\
    /config set volume music|sfx|voice <0-100>";

impl Setting {
    /// The setting a line after `/config set` names, or what is wrong with it.
    pub fn parse(line: &str) -> Result<Setting, String> {
        let (key, value) = word(line);
        if cfg!(target_arch = "wasm32") {
            match key {
                "vsync" | "fps-max" => {
                    return Err("In a browser, the browser sets the frame rate".into());
                }
                _ => {}
            }
        }
        if cfg!(not(target_arch = "wasm32")) && key == "render-scale" {
            return Err("Render scale is only for the browser build".into());
        }
        match key {
            "render-scale" => render_scale(value)
                .map(Setting::RenderScale)
                .ok_or_else(|| RENDER_SCALE_USAGE.to_string()),
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
            "msaa" => match value {
                "off" => Ok(Setting::Msaa(1)),
                "2" => Ok(Setting::Msaa(2)),
                "4" => Ok(Setting::Msaa(4)),
                _ => Err("Usage: /config set msaa off|2|4".into()),
            },
            "shader-effects" => match value {
                "on" => Ok(Setting::ShaderEffects(true)),
                "off" => Ok(Setting::ShaderEffects(false)),
                _ => Err("Usage: /config set shader-effects on|off".into()),
            },
            "volume" => {
                const USAGE: &str = "Usage: /config set volume music|sfx|voice <0-100>";
                let (kind, percent) = word(value);
                let percent: u8 = percent
                    .parse()
                    .ok()
                    .filter(|p| *p <= 100)
                    .ok_or_else(|| USAGE.to_string())?;
                match kind {
                    "music" => Ok(Setting::VolumeMusic(percent)),
                    "sfx" => Ok(Setting::VolumeSfx(percent)),
                    "voice" => Ok(Setting::VolumeVoice(percent)),
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
        app.add_systems(Update, (apply, present_mode, net_transport, msaa))
            .add_observer(new_camera_defaults);
    }
}

/// A camera starts with the multisampling the config says, not Bevy's 4x, and without the
/// tonemapping and the dither that Bevy gives a 3D camera: the game's colors are the disc's, and
/// each of those is a pass over the whole picture.
fn new_camera_defaults(add: On<Add<Camera3d>>, config: Res<Config>, mut commands: Commands) {
    commands.entity(add.entity).insert((
        config.msaa(),
        Tonemapping::None,
        DebandDither::Disabled,
    ));
}

/// The cameras multisample as the config says, when it changes.
fn msaa(config: Res<Config>, mut cameras: Query<&mut Msaa>) {
    if !config.is_changed() {
        return;
    }
    for mut msaa in &mut cameras {
        if *msaa != config.msaa() {
            *msaa = config.msaa();
        }
    }
}

/// The network uses the relay only, or not, as the config says, from the next session on.
fn net_transport(config: Res<Config>) {
    if config.is_changed() {
        net::set_relay_only(config.relay_only);
        net::set_relays(&config.relays);
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
            Setting::Msaa(samples) => {
                config.msaa = *samples;
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                let text = match samples {
                    1 => "Multisampling off".to_string(),
                    samples => format!("Multisampling: {samples}x"),
                };
                toasts.write(Toast::new(text, 2000));
            }
            Setting::ShaderEffects(on) => {
                config.shader_effects = *on;
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                toasts.write(Toast::new(
                    format!("Shader effects {}", if *on { "on" } else { "off" }),
                    2000,
                ));
            }
            Setting::RenderScale(scale) => {
                config.render_scale = *scale;
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                #[cfg(target_arch = "wasm32")]
                crate::web::set_render_scale(*scale);
                toasts.write(Toast::new(format!("Render scale {scale}"), 2000));
            }
            Setting::AddRelay(url) => {
                let text = if config.relays.contains(url) {
                    format!("Relay already added: {url}")
                } else if config.relays.len() >= MAX_RELAYS {
                    format!("Too many relays (at most {MAX_RELAYS})")
                } else {
                    config.relays.push(url.clone());
                    if let Err(why) = config.save() {
                        toasts.write(refuse(why));
                    }
                    format!("Relay added: {url}\nIt takes hold in the next lobby")
                };
                toasts.write(Toast::new(text, 3000));
            }
            Setting::ClearRelays => {
                let text = if config.relays.is_empty() {
                    "No relays added".to_string()
                } else {
                    config.relays.clear();
                    if let Err(why) = config.save() {
                        toasts.write(refuse(why));
                    }
                    "Relays cleared\nIt takes hold in the next lobby".to_string()
                };
                toasts.write(Toast::new(text, 3000));
            }
            Setting::RelayOnly(on) => {
                config.relay_only = *on;
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                let text = match (on, lobby.is_open()) {
                    (true, false) => "Relay only: the others will not see your address",
                    (false, false) => "Relay only off: connections go straight when they can",
                    (_, true) => "Relay only changes\nIt takes hold in the next lobby",
                };
                toasts.write(Toast::new(text, 3500));
            }
            Setting::VolumeSfx(percent)
            | Setting::VolumeMusic(percent)
            | Setting::VolumeVoice(percent) => {
                let (volume, name) = match setting {
                    Setting::VolumeSfx(_) => (&mut config.volume_fx, "Effects"),
                    Setting::VolumeMusic(_) => (&mut config.volume_music, "Music"),
                    _ => (&mut config.volume_voice, "Voice"),
                };
                *volume = *percent;
                if let Err(why) = config.save() {
                    toasts.write(refuse(why));
                }
                toasts.write(Toast::new(format!("{name} volume: {percent}"), 2000));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_scale_is_a_part_of_the_resolution() {
        assert_eq!(render_scale("0.5"), Some(0.5));
        assert_eq!(render_scale("1"), Some(1.0));
        assert_eq!(render_scale("0.25"), Some(0.25));
        for wrong in ["0.2", "1.5", "0", "NaN", "half", ""] {
            assert_eq!(render_scale(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn the_cups_are_kept() {
        use crate::lobby::cup::CupDef;
        let mut config = Config::default();
        config.cups.set(CupDef {
            name: "fast".into(),
            tracks: vec!["dingo_canyon".into(), "crash_cove".into()],
            laps: 3,
        });
        let text = serde_json::to_string(&config).unwrap();
        let read: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(read.cups.all(), config.cups.all());
        // A config from before there were any is still read.
        let old: Config = serde_json::from_str(r#"{"nickname": "Bob"}"#).unwrap();
        assert_eq!(old.nickname, "Bob");
        assert!(old.cups.all().is_empty());
    }
}
