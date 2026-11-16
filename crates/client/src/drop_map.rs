//! Drag and drop of custom maps: a `.xdelta` patch dropped on the window is imported as
//! `ctr-map import` does (a pack of several tracks makes several maps), a `.ctrmap` is taken as
//! it is. The map is saved (`maps::add`) and listed
//! at once, so a lobby we host sends it to the others when a race starts on it.
//!
//! The files come from the window (`FileDragAndDrop`), or in a browser from the page, which
//! holds what was dropped on it while the game runs (`web::take_dropped`, as for the disc image
//! before it). The import takes a few seconds, on a thread. A browser can only take a `.ctrmap`:
//! a patch is of the whole disc image, which the page lets go of once the game has started
//! (`web::release_disc`).

use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task, futures::check_ready};

use crate::toast::Toast;

pub struct DropMapPlugin;

impl Plugin for DropMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Importing>()
            .add_message::<Dropped>()
            .add_systems(Update, (collect, start, finish).chain());
    }
}

/// A file that was dropped: its name, and what is in it.
#[derive(Message)]
struct Dropped {
    name: String,
    bytes: Arc<[u8]>,
}

/// What an import made: the maps, and what else the patch changes (`Import::unfollowed`).
type Made = Result<(Vec<map::Map>, Vec<String>), String>;

/// The imports that are running.
#[derive(Resource, Default)]
struct Importing(Vec<(String, Task<Made>)>);

fn failure(text: String) -> Toast {
    Toast::new(text, 5000).color(Color::srgb(1.0, 0.4, 0.4))
}

/// Reads what was dropped, from the window or the page.
fn collect(mut drops: MessageReader<FileDragAndDrop>, mut dropped: MessageWriter<Dropped>) {
    for drop in drops.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = drop else {
            continue;
        };
        let name = path_buf
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match std::fs::read(path_buf) {
            Ok(bytes) => {
                dropped.write(Dropped {
                    name,
                    bytes: bytes.into(),
                });
            }
            Err(e) => {
                error!("{}: {e}", path_buf.display());
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    for (name, bytes) in crate::web::take_dropped() {
        dropped.write(Dropped {
            name,
            bytes: bytes.into(),
        });
    }
}

fn start(
    mut dropped: MessageReader<Dropped>,
    mut importing: ResMut<Importing>,
    mut toasts: MessageWriter<Toast>,
) {
    for Dropped { name, bytes } in dropped.read() {
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, ext)) => (stem.to_string(), ext.to_ascii_lowercase()),
            None => (name.clone(), String::new()),
        };
        let bytes = bytes.clone();
        let work: Box<dyn FnOnce() -> Made + Send> = match extension.as_str() {
            "xdelta" if cfg!(target_arch = "wasm32") => {
                toasts.write(failure(format!(
                    "{name}: a patch cannot be imported in a browser: drop its .ctrmap \
                     (`ctr-map import` makes it)"
                )));
                continue;
            }
            "xdelta" => {
                toasts.write(Toast::new(format!("Importing {name}..."), 3000));
                Box::new(move || import_patch(stem, bytes))
            }
            "ctrmap" => Box::new(move || {
                let map = map::Map::from_bytes(&bytes).map_err(|e| e.to_string())?;
                Ok((vec![map], Vec::new()))
            }),
            _ => {
                let what = if cfg!(target_arch = "wasm32") {
                    "a .ctrmap"
                } else {
                    "an .xdelta patch or a .ctrmap"
                };
                toasts.write(failure(format!("{name}: drop {what}")));
                continue;
            }
        };
        importing
            .0
            .push((name.clone(), IoTaskPool::get().spawn(async move { work() })));
    }
}

fn import_patch(name: String, patch: Arc<[u8]>) -> Made {
    let disc = crate::disc::image()
        .ok_or("importing a patch needs the disc image (CTR_BIN, or \"iso\" in the config)")?;
    let done = map::import::import_unattended(&disc, name, patch).map_err(|e| e.to_string())?;
    Ok((done.maps.into_iter().map(|m| m.map).collect(), done.unfollowed))
}

fn finish(mut importing: ResMut<Importing>, mut toasts: MessageWriter<Toast>) {
    importing.0.retain_mut(|(label, task)| {
        let Some(result) = check_ready(task) else {
            return true;
        };
        let (maps, unfollowed) = match result {
            Ok(made) => made,
            Err(why) => {
                toasts.write(failure(format!("{label}: {why}")));
                return false;
            }
        };
        let mut lines = Vec::new();
        let mut last = None;
        for map in maps {
            let name = map.name.clone();
            match crate::maps::add(map) {
                Ok((id, new)) => {
                    let slug = crate::maps::slug(id);
                    let what = if new { "added" } else { "already there" };
                    info!("custom track {name} ({id:#x}, {slug}): {what}");
                    lines.push(format!("{} track {name}", if new { "Added" } else { "Already have" }));
                    last = Some(slug);
                }
                Err(why) => {
                    toasts.write(failure(format!("{label}: {name}: {why}")));
                }
            }
        }
        if !unfollowed.is_empty() {
            warn!(
                "{label} also changes what no map carries (the port plays retail rules there): {}",
                unfollowed.join(", ")
            );
            lines.push(format!("It also changes {} other files or code (see the log)", unfollowed.len()));
        }
        if let Some(slug) = last {
            lines.push(format!("] steps to it; /cup create can use {slug}"));
            toasts.write(Toast::new(lines.join("\n"), 5000).color(Color::linear_rgb(0.4, 1.0, 0.4)));
        }
        false
    });
}
