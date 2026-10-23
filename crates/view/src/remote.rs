//! The other players' karts in an online race. Each client simulates all of them, from the pads
//! the lockstep hands out, so every sim sees the same race. They are drawn like our own kart
//! (`kart.rs`), only in the world's layer: the walls hide them.

use std::collections::HashMap;

use bevy::prelude::*;
use coll::Collision;
use loader::icons::Icon;
use loader::model::Model;
use vehicle::{Driver, Effect, Env};

use crate::Track;
use crate::drive::{Drive, Kart, input_of_pad};
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

/// Whether karts bump into each other: on unless `CTR_KART_COLLISIONS=0`. (Retail has them off
/// online. Every player of a race has to run it the same way.)
pub fn kart_collisions() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("CTR_KART_COLLISIONS").map_or(true, |v| v != "0"))
}

/// Our own kart in a tick of all of them.
pub struct Own<'a> {
    pub driver: &'a mut Driver,
    pub coll: &'a mut Collision,
    pub input: &'a vehicle::Input,
    pub slot: usize,
}

impl Remotes {
    /// Plays one tick of all the karts: ours and the others', which play from their pads of
    /// `frame`. The karts share the level's objects and bump into each other, so every client
    /// has to play them the same way: in the order of the slots, each stage over all of them.
    /// The result is ours, and what ours asks for (sounds, rumble).
    pub fn play(
        &mut self,
        own: Own,
        frame: Option<&[net::Pad]>,
        env: &Env,
        track: &Track,
        sim: &mut Sim,
        particles: &mut crate::exhaust::Particles,
        feedback: &mut vehicle::collide::Feedback,
        motions: &mut Query<(&mut KartMotion, Has<Kart>)>,
    ) -> (vehicle::sim::TickResult, Vec<Effect>) {
        struct Entry<'a> {
            slot: usize,
            driver: &'a mut Driver,
            coll: &'a mut Collision,
            input: vehicle::Input,
            effects: Vec<Effect>,
            /// The entity of another player's kart; ours is none.
            remote: Option<Entity>,
        }
        let mut entries = Vec::new();
        for (slot, kart) in self.karts.iter_mut().enumerate() {
            let (Some(kart), Some(frame)) = (kart, frame) else { continue };
            let input = input_of_pad(&frame[slot], kart.held);
            kart.held = input.buttons_held;
            entries.push(Entry {
                slot,
                driver: &mut kart.driver,
                coll: &mut kart.coll,
                input,
                effects: Vec::new(),
                remote: Some(kart.entity),
            });
        }
        let own_slot = own.slot;
        entries.push(Entry {
            slot: own_slot,
            driver: own.driver,
            coll: own.coll,
            input: *own.input,
            effects: Vec::new(),
            remote: None,
        });
        entries.sort_by_key(|e| e.slot);

        for e in &mut entries {
            crate::drive::before_tick(e.driver, &e.input, sim, &mut e.effects);
        }
        let results = {
            let mut racers: Vec<vehicle::sim::Racer> = entries
                .iter_mut()
                .map(|e| vehicle::sim::Racer { driver: &mut *e.driver, coll: &mut *e.coll, input: &e.input, effects: &mut e.effects })
                .collect();
            vehicle::sim::tick_all(&mut racers, &track.0, env, &mut sim.objects, feedback)
        };

        let mut ours = None;
        for (mut e, result) in entries.into_iter().zip(results) {
            crate::drive::after_tick(e.driver, env, e.remote.is_none().then_some(&mut *particles), &mut e.effects);
            match e.remote {
                Some(entity) => {
                    if let Ok((mut motion, _)) = motions.get_mut(entity) {
                        motion.push(e.driver);
                    }
                }
                // Their effects (sounds, rumble) are not ours to play.
                None => ours = Some((result, e.effects)),
            }
        }
        ours.expect("our own kart plays")
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
