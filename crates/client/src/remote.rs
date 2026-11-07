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
    others: Query<(Entity, Has<DummyKart>), (With<KartSlot>, Without<Kart>)>,
    mut builder: KartBuilder,
    loading: Option<Res<crate::loading::Loading>>,
    mut dummies: ResMut<DummyKarts>,
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
        // (Offline, the dummy karts of `/debug spawn-karts` stay.)
        if lobby.online.is_none() {
            for (entity, dummy) in &others {
                if !dummy {
                    builder.commands.entity(entity).try_despawn();
                }
            }
        }
        return;
    };
    online.built = true;
    // The dummies too: the race is the lobby's.
    dummies.0.clear();
    for (entity, _) in &others {
        builder.commands.entity(entity).try_despawn();
    }
    online.specs.clear();
    online.bot_specs.clear();
    // (Every client drops them alike: the level is the same for all.)
    if !online.bots.characters.is_empty()
        && !track.0.nav_paths.iter().any(|path| path.frames.len() > 1)
    {
        online.bots.characters.clear();
        toasts.write(crate::toast::Toast::new(
            "This track has no paths for bots: they sit this race out",
            4000,
        ));
    }
    let players = online.characters.len();
    let mut models: HashMap<usize, (Model, Vec<Icon>)> = HashMap::new();
    let bots = online.bots.characters.iter().enumerate();
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
        };
        if slot < players {
            online.specs.push(spec);
        } else {
            online.bot_specs.push(spec);
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

/// The karts of `/debug spawn-karts`, offline: they stand on the grid with ours, in the slots
/// after it (1 on), to see what many karts cost. Each of them has left the race from tick 0
/// (`Rollback::leave`), so it holds a neutral pad, and nothing waits for it or predicts it.
/// `drive_tick` builds the race with these; a new scene (another track, `/debug viewer`) takes
/// them away (`scene::restart_drive`), and so does an online race.
#[derive(Resource, Default)]
pub struct DummyKarts(pub Vec<KartSpec>);

/// A kart of [`DummyKarts`].
#[derive(Component)]
pub struct DummyKart;

/// `/debug spawn-karts`: takes away the dummy karts there are, and puts that many on the grid,
/// of random characters. The next tick sees the race has other karts than these and starts it
/// over from the grid, ours too.
pub fn spawn_dummies(
    mut actions: MessageReader<crate::command::Action>,
    lobby: Res<Lobby>,
    track: Res<Track>,
    loading: Option<Res<crate::loading::Loading>>,
    mut dummies: ResMut<DummyKarts>,
    old: Query<Entity, With<DummyKart>>,
    mut builder: KartBuilder,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    use std::hash::BuildHasher;
    for action in actions.read() {
        let crate::command::Action::DebugSpawnKarts(count) = *action else {
            continue;
        };
        if lobby.online.is_some() || loading.is_some() {
            toasts.write(
                crate::toast::Toast::new("Not now: offline, once the level is up", 2500)
                    .color(Color::srgb(1.0, 0.4, 0.4)),
            );
            continue;
        }
        for entity in &old {
            builder.commands.entity(entity).try_despawn();
        }
        dummies.0.clear();
        // (The std hasher is random enough.)
        let random = std::collections::hash_map::RandomState::new();
        let mut models: HashMap<usize, (Model, Vec<Icon>)> = HashMap::new();
        let mut names = Vec::new();
        for slot in 1..=count as usize {
            let character = random.hash_one(slot) as usize % crate::CHARACTERS.len();
            let (model, wheel_icons) = models
                .entry(character)
                .or_insert_with(|| crate::load_racer(character));
            let spec = KartSpec {
                character,
                anim_num_frames: crate::scene::anim_num_frames(model),
            };
            let birth = race::Kart::birth(&track.0, slot, &spec);
            // The first `show` puts it where the sim has it.
            let kart = builder.spawn(
                model,
                wheel_icons,
                Transform::default(),
                false,
                Some((slot, &birth)),
            );
            builder.commands.entity(kart).insert(DummyKart);
            dummies.0.push(spec);
            names.push(crate::CHARACTERS[character]);
        }
        let text = if names.is_empty() {
            "No dummy karts".to_string()
        } else {
            format!("Dummy karts: {}", names.join(", "))
        };
        toasts.write(crate::toast::Toast::new(text, 3000));
    }
}
