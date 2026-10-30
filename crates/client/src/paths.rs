//! Where the client keeps its files. A debug build (`cargo run`) uses the checkout's `assets/`;
//! a release build keeps them per user, in `%APPDATA%\ctr-rs` on Windows,
//! `~/Library/Application Support/ctr-rs` on macOS and `~/.config/ctr-rs` elsewhere.

use std::path::PathBuf;

#[cfg(debug_assertions)]
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

#[cfg(not(debug_assertions))]
fn root() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("ctr-rs"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The settings, see `config.rs`.
pub fn config_file() -> PathBuf {
    cache().join("config.json")
}

/// Where the panic that ended the last run is written, see `crash.rs`.
pub fn crash_log() -> PathBuf {
    root().join("crash.log")
}

/// What is made from the disc image (`prepare`'s output), which can all go to make it again.
pub fn disc() -> PathBuf {
    root().join("disc")
}

/// The maps of the game's own tracks, made from the disc.
pub fn builtin_maps() -> PathBuf {
    disc().join("maps")
}

/// The maps that are ours.
pub fn maps() -> PathBuf {
    root().join("maps")
}

/// The maps that other players sent: everything in it may go. (Not the same folder as `maps`
/// in any build: `/cache purge` deletes this one whole.)
pub fn cached_maps() -> PathBuf {
    root().join(".cache").join("maps")
}

fn cache() -> PathBuf {
    if cfg!(debug_assertions) {
        root().join(".cache")
    } else {
        root()
    }
}
