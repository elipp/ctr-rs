//! The other players' karts in an online race. Each client simulates all of them, from the pads
//! the lockstep hands out, so every sim sees the same race. They are drawn like our own kart
//! (`kart.rs`), only in the world's layer: the walls hide them.

use std::collections::HashMap;
use std::ops::Range;

use bevy::prelude::*;
use coll::Collision;
use loader::icons::Icon;
use loader::model::Model;
use vehicle::Driver;

use crate::Track;
use crate::drive::{Drive, Kart, input_of_pad, play_kart};
use crate::kart::{self, KartMotion};
use crate::lobby::Lobby;
use crate::props::Sim;
use crate::subtract::SubtractMaterial;

/// The sim of another player's kart.
struct RemoteKart {
    driver: Driver,
    coll: Collision,
    /// The buttons held in the last tick, for the taps.
    held: u32,
    /// The entity that draws it.
    entity: Entity,
}

/// The other players' karts, by slot. Our own slot has none.
#[derive(Resource, Default)]
pub struct Remotes {
    karts: Vec<Option<RemoteKart>>,
}

impl Remotes {
    /// Plays one tick of the karts in `slots`, with their pads of `frame`.
    ///
    /// The karts share the level's objects, so every client has to play them in the same order:
    /// by slot, our own kart in its place among them.
    pub fn play(
        &mut self,
        slots: Range<usize>,
        frame: &[net::Pad],
        tick: u32,
        track: &Track,
        sim: &mut Sim,
        motions: &mut Query<(&mut KartMotion, Has<Kart>)>,
    ) {
        for slot in slots {
            let Some(Some(kart)) = self.karts.get_mut(slot) else { continue };
            let input = input_of_pad(&frame[slot], kart.held);
            kart.held = input.buttons_held;
            // Their effects (sounds, rumble) are not ours to play.
            let mut effects = Vec::new();
            play_kart(&mut kart.driver, &mut kart.coll, track, tick, &input, sim, None, &mut effects);
            if let Ok((mut motion, _)) = motions.get_mut(kart.entity) {
                motion.push(&kart.driver);
            }
        }
    }
}

/// Puts the karts of the others on the grid when a race starts, and takes them away when it ends.
pub fn build(
    mut lobby: ResMut<Lobby>,
    mut remotes: ResMut<Remotes>,
    track: Res<Track>,
    textures: Res<crate::Textures>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    // Not before the level of the race is up.
    let Some(online) = lobby.online.as_mut().filter(|online| !online.built && online.track == track.1.0) else {
        if lobby.online.is_none() {
            for kart in remotes.karts.drain(..).flatten() {
                commands.entity(kart.entity).try_despawn();
            }
        }
        return;
    };
    online.built = true;
    for kart in remotes.karts.drain(..).flatten() {
        commands.entity(kart.entity).try_despawn();
    }
    let mut models: HashMap<usize, (Model, Vec<Icon>)> = HashMap::new();
    for (slot, &character) in online.characters.iter().enumerate() {
        if slot == online.slot {
            remotes.karts.push(None);
            continue;
        }
        let character = character as usize;
        let (model, wheel_icons) = models.entry(character).or_insert_with(|| crate::load_racer(character));
        let mut anim_num_frames = [0; 8];
        for (n, anim) in anim_num_frames.iter_mut().zip(&model.headers[0].animations) {
            *n = anim.num_frames;
        }
        let (driver, coll) = Drive::birth(&track, character, anim_num_frames, slot);
        // The first `show` puts it where the sim has it.
        let entity = kart::spawn(
            model,
            wheel_icons,
            &textures.vram,
            Transform::default(),
            0,
            Some(KartMotion::new(&driver)),
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
        );
        remotes.karts.push(Some(RemoteKart { driver, coll, held: 0, entity }));
    }
}
