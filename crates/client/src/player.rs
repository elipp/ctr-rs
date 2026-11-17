//! The player key: who we are on the leaderboards (TIME-TRIAL-RELIC-LEADERBOARDS.md, "Identity").
//! It only ever signs score requests (`score::Auth`); lobbies and the mesh keep their random
//! per-session endpoint keys, so it never shows up in pkarr/DNS or to lobby peers.
//!
//! Made on first use, not at start: a player who never asks for a board never has one. Losing it
//! loses the player's runs on the boards; `/score key export` is the only backup.

use std::sync::Mutex;

use bevy::prelude::*;
use score::SecretKey;

use crate::command::Action;
use crate::console::console;
use crate::toast::Toast;

static KEY: Mutex<Option<SecretKey>> = Mutex::new(None);

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, key_commands);
    }
}

/// The player key, made and kept if there is none. An error if the stored one can't be read, or
/// a new one can't be kept: a key that is gone at the next start would orphan whatever it signs.
pub fn key() -> Result<SecretKey, String> {
    let mut held = KEY.lock().map_err(|e| e.to_string())?;
    if let Some(key) = held.as_ref() {
        return Ok(key.clone());
    }
    let key = match read()? {
        Some(text) => score::secret_from_text(&text)
            .ok_or_else(|| format!("The player key in {} is not a key", place()))?,
        None => {
            let key = SecretKey::generate();
            write(&score::secret_to_text(&key))?;
            info!("made the player key {}", score::fingerprint(&key.public()));
            key
        }
    };
    *held = Some(key.clone());
    Ok(key)
}

/// Makes `key` ours, from now on and at every start.
fn replace(key: SecretKey) -> Result<(), String> {
    let mut held = KEY.lock().map_err(|e| e.to_string())?;
    write(&score::secret_to_text(&key))?;
    *held = Some(key);
    Ok(())
}

fn key_commands(mut actions: MessageReader<Action>, mut toasts: MessageWriter<Toast>) {
    let red = Color::srgb(1.0, 0.4, 0.4);
    for action in actions.read() {
        match action {
            Action::ScoreKeyExport => match key() {
                Ok(key) => {
                    console!(
                        "Your player key ({}). Anyone who has it can post and delete runs as \
                         you: keep it secret.",
                        score::fingerprint(&key.public())
                    );
                    console!("{}", score::secret_to_text(&key));
                }
                Err(why) => {
                    toasts.write(Toast::new(why, 4000).color(red));
                }
            },
            Action::ScoreKeyImport(text) => {
                let Some(new) = score::secret_from_text(text) else {
                    toasts.write(Toast::new("That is not a player key", 2500).color(red));
                    continue;
                };
                let fingerprint = score::fingerprint(&new.public());
                match replace(new) {
                    Ok(()) => {
                        toasts.write(Toast::new(format!("You are {fingerprint} now"), 2500));
                    }
                    Err(why) => {
                        toasts.write(Toast::new(why, 4000).color(red));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn place() -> String {
    crate::paths::player_key().display().to_string()
}

#[cfg(target_arch = "wasm32")]
fn place() -> String {
    "the browser's storage".to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn read() -> Result<Option<String>, String> {
    match std::fs::read_to_string(crate::paths::player_key()) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read {}: {e}", place())),
    }
}

#[cfg(target_arch = "wasm32")]
fn read() -> Result<Option<String>, String> {
    Ok(crate::web::load_player_key())
}

/// Through a temporary file and a rename, so that a failed write never leaves half a key.
#[cfg(not(target_arch = "wasm32"))]
fn write(text: &str) -> Result<(), String> {
    use std::io::Write;
    let path = crate::paths::player_key();
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("key.new");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temp, &path)
    };
    write().map_err(|e| format!("Could not save {}: {e}", place()))
}

#[cfg(target_arch = "wasm32")]
fn write(text: &str) -> Result<(), String> {
    match crate::web::save_player_key(text) {
        None => Ok(()),
        Some(e) => Err(format!("Could not save the player key: {e}")),
    }
}
