//! Drag and drop of custom maps: a `.xdelta` patch dropped on the window is imported as
//! `ctr-map import` does, a `.ctrmap` is taken as it is. The map is saved (`maps::add`) and listed
//! at once, so a lobby we host sends it to the others when a race starts on it.
//!
//! The files come from the window (`FileDragAndDrop`), or in a browser from the page, which
//! holds what was dropped on it while the game runs (`web::take_dropped`, as for the disc image
//! before it). The import takes a few seconds: on a thread, but in a browser, which has none
//! here, it stops the frames meanwhile.

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
    bytes: Vec<u8>,
}

/// The imports that are running.
#[derive(Resource, Default)]
struct Importing(Vec<(String, Task<Result<map::Map, String>>)>);

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
                dropped.write(Dropped { name, bytes });
            }
            Err(e) => {
                error!("{}: {e}", path_buf.display());
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    for (name, bytes) in crate::web::take_dropped() {
        dropped.write(Dropped { name, bytes });
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
        let work: Box<dyn FnOnce() -> Result<map::Map, String> + Send> = match extension.as_str() {
            "xdelta" => {
                toasts.write(Toast::new(format!("Importing {name}..."), 3000));
                Box::new(move || import_patch(stem, bytes))
            }
            "ctrmap" => Box::new(move || map::Map::from_bytes(&bytes).map_err(|e| e.to_string())),
            _ => {
                toasts.write(failure(format!("{name}: drop an .xdelta patch or a .ctrmap")));
                continue;
            }
        };
        importing
            .0
            .push((name.clone(), IoTaskPool::get().spawn(async move { work() })));
    }
}

fn import_patch(name: String, patch: Vec<u8>) -> Result<map::Map, String> {
    let disc = crate::disc::image()
        .ok_or("importing a patch needs the disc image (CTR_BIN, or \"iso\" in the config)")?;
    map::import::import_unattended(&disc, name, patch)
        .map(|done| done.map)
        .map_err(|e| e.to_string())
}

fn finish(mut importing: ResMut<Importing>, mut toasts: MessageWriter<Toast>) {
    importing.0.retain_mut(|(label, task)| {
        let Some(result) = check_ready(task) else {
            return true;
        };
        match result.and_then(|map| {
            let name = map.name.clone();
            crate::maps::add(map).map(|(id, new)| (name, id, new))
        }) {
            Ok((name, id, new)) => {
                let slug = crate::maps::slug(id);
                info!("custom track {name} ({id:#x}, {slug}): {}", if new { "added" } else { "already there" });
                toasts.write(
                    Toast::new(
                        format!(
                            "{} track {name}\n] steps to it; /cup create can use {slug}",
                            if new { "Added" } else { "Already have" }
                        ),
                        5000,
                    )
                    .color(Color::linear_rgb(0.4, 1.0, 0.4)),
                );
            }
            Err(why) => {
                toasts.write(failure(format!("{label}: {why}")));
            }
        }
        false
    });
}
