//! The other players' karts in an online race. Each client simulates all of them, from the pads
//! the others send, so every sim ends up seeing the same race. They are drawn like our own kart
//! (`kart.rs`), only in the world's layer: the walls hide them.

use std::collections::HashMap;

use bevy::prelude::*;
use loader::icons::Icon;
use loader::model::Model;
use race::KartSpec;

use crate::Track;
use crate::drive::Drive;
use crate::kart::{self, KartMotion};
use crate::lobby::Lobby;
use crate::subtract::SubtractMaterial;

/// The entities that draw the other players' karts, by slot. Our own slot has none (the kart we
/// drive is `drive::Kart`), nor has a slot before the race starts. What they do is the race's,
/// in `Sim`: these only show it.
#[derive(Resource, Default)]
pub struct Remotes {
    pub entities: Vec<Option<Entity>>,
}

/// Whether karts bump into each other: on unless `CTR_KART_COLLISIONS=0`. (Retail has them off
/// online. Every player of a race has to run it the same way.)
pub fn kart_collisions() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("CTR_KART_COLLISIONS").map_or(true, |v| v != "0"))
}

/// Puts the karts of the others on the grid when a race starts, and takes them away when it ends.
pub fn build(
    mut lobby: ResMut<Lobby>,
    mut remotes: ResMut<Remotes>,
    track: Res<Track>,
    textures: Res<crate::Textures>,
    drive: Res<Drive>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    // Not before the level of the race is up.
    let Some(online) = lobby
        .online
        .as_mut()
        .filter(|online| !online.built && online.track == track.1.0)
    else {
        if lobby.online.is_none() {
            for entity in remotes.entities.drain(..).flatten() {
                commands.entity(entity).try_despawn();
            }
        }
        return;
    };
    online.built = true;
    for entity in remotes.entities.drain(..).flatten() {
        commands.entity(entity).try_despawn();
    }
    online.specs.clear();
    let mut models: HashMap<usize, (Model, Vec<Icon>)> = HashMap::new();
    for (slot, &character) in online.characters.iter().enumerate() {
        if slot == online.slot {
            online.specs.push(drive.spec());
            remotes.entities.push(None);
            continue;
        }
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
        let birth = race::Kart::birth(&track.0, slot, &spec);
        // The first `show` puts it where the sim has it.
        let entity = kart::spawn(
            model,
            wheel_icons,
            &textures.vram,
            Transform::default(),
            0,
            Some(KartMotion::new(&birth.driver)),
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
        );
        remotes.entities.push(Some(entity));
    }
    // The race is built: the others can start once they are as well.
    lobby.ready();
}
