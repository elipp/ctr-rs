//! The other players' karts in an online race. Each client simulates all of them, from the pads
//! the others send, so every sim ends up seeing the same race. They are drawn like our own kart
//! (`kart.rs`), only in the world's layer: the walls hide them.

use std::collections::HashMap;

use bevy::prelude::*;
use loader::icons::Icon;
use loader::model::Model;
use race::KartSpec;

use crate::Track;
use crate::drive::{Drive, Kart};
use crate::kart::{KartBuilder, KartSlot};
use crate::lobby::Lobby;

/// Whether karts bump into each other: on unless `CTR_KART_COLLISIONS=0`. (Retail has them off
/// online. Every player of a race has to run it the same way.)
pub fn kart_collisions() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("CTR_KART_COLLISIONS").map_or(true, |v| v != "0"))
}

/// Puts the karts of the others on the grid when a race starts, and takes them away when it ends.
/// (Ours is the one with the [`Kart`] marker; the others are every kart that has none.)
pub fn build(
    mut lobby: ResMut<Lobby>,
    track: Res<Track>,
    drive: Res<Drive>,
    others: Query<Entity, (With<KartSlot>, Without<Kart>)>,
    mut builder: KartBuilder,
) {
    // Not before the level of the race is up.
    let Some(online) = lobby
        .online
        .as_mut()
        .filter(|online| !online.built && online.track == track.1.0)
    else {
        if lobby.online.is_none() {
            for entity in &others {
                builder.commands.entity(entity).try_despawn();
            }
        }
        return;
    };
    online.built = true;
    for entity in &others {
        builder.commands.entity(entity).try_despawn();
    }
    online.specs.clear();
    let mut models: HashMap<usize, (Model, Vec<Icon>)> = HashMap::new();
    for (slot, &character) in online.characters.iter().enumerate() {
        // Every kart, ours too, is what the host said it drives: a character of ours that
        // differs would be a sim of its own.
        let character = character as usize;
        let (model, wheel_icons) = models
            .entry(character)
            .or_insert_with(|| crate::load_racer(character));
        let mut anim_num_frames = [0; 8];
        for (n, anim) in anim_num_frames.iter_mut().zip(&model.headers[0].animations) {
            *n = anim.num_frames;
        }
        let spec = KartSpec {
            character,
            anim_num_frames,
        };
        online.specs.push(spec);
        // (Ours is on the grid already.)
        if slot == online.slot {
            continue;
        }
        let birth = race::Kart::birth(&track.0, slot, &spec);
        // The first `show` puts it where the sim has it.
        builder.spawn(
            model,
            wheel_icons,
            Transform::default(),
            false,
            Some((slot, &birth)),
        );
    }
    // The race is built: the others can start once they are as well.
    lobby.ready();
}
