//! The other players' karts in an online race, and the bots'. Each client simulates all of them,
//! from the pads the others send, so every sim ends up seeing the same race. They are drawn like
//! our own kart (`kart.rs`), only in the world's layer: the walls hide them.

use std::collections::HashMap;

use bevy::prelude::*;
use loader::icons::Icon;
use loader::model::Model;
use race::KartSpec;

use crate::Track;
use crate::drive::Kart;
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
    others: Query<Entity, (With<KartSlot>, Without<Kart>)>,
    mut builder: KartBuilder,
    loading: Option<Res<crate::loading::Loading>>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    // Not before the level of the race is up, and settled (`loading.rs`): the others wait for our
    // ready.
    if loading.is_some() {
        return;
    }
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
    online.retail_specs.clear();
    // (Every client drops them alike: the level is the same for all.)
    if !online.retail.characters.is_empty()
        && !track.0.nav_paths.iter().any(|path| path.frames.len() > 1)
    {
        online.retail.characters.clear();
        toasts.write(crate::toast::Toast::new(
            "This track has no paths for bots: they sit this race out",
            4000,
        ));
    }
    let players = online.characters.len();
    let mut models: HashMap<usize, (Model, Vec<Icon>)> = HashMap::new();
    let bots = online.retail.characters.iter().enumerate();
    let karts = online
        .characters
        .iter()
        .enumerate()
        .chain(bots.map(|(i, character)| (players + i, character)));
    for (slot, &character) in karts.collect::<Vec<_>>() {
        // Every kart, ours too, is what the host said it drives: a character of ours that
        // differs would be a sim of its own.
        let character = character as usize;
        let (model, wheel_icons) = models
            .entry(character)
            .or_insert_with(|| crate::load_racer(character));
        let spec = KartSpec {
            character,
            anim_num_frames: crate::scene::anim_num_frames(model),
            bot: false,
            line: None,
        };
        if slot < players {
            online.specs.push(spec);
        } else {
            online.retail_specs.push(spec);
        }
        // (Ours is on the grid already. A spectator's slot is past the players', where the bots'
        // karts are.)
        if slot == online.slot && !online.spectating() {
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
