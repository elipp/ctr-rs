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
    dummies.1 = None;
    for (entity, _) in &others {
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

/// The karts of `/debug spawn-karts`, offline: on the grid with ours, in the slots after it (1 on),
/// either bots (`race::bot`), standing still, or driven by a learned policy (`learned.rs`). The
/// first two have left the race from tick 0 (`Rollback::leave`), so nothing waits for their pads
/// or predicts them: a bot's the race makes itself, and the others hold a neutral one. The
/// policy's are made each tick and confirmed like ours (`drive_tick`).
/// `drive_tick` builds the race with these; a new scene (another track, `/debug viewer`) takes
/// them away (`scene::restart_drive`), and so does an online race.
/// (Temporary) The second is the slot watched by `/debug spectate`: then ours is driven as the
/// dummies are (a bot, or by the policy), drawn by a dummy of slot 0, and the race has no kart of
/// ours.
/// The third is the bot path every bot drives, of `/debug bot-line` (`KartSpec::line`); it stays
/// from one scene to the next.
/// The fourth is whether the policy drives the dummies (`/debug spawn-karts <n> brain`): their
/// specs say `bot: false`, as the static ones' do.
#[derive(Resource, Default)]
pub struct DummyKarts(
    pub Vec<KartSpec>,
    pub Option<usize>,
    pub Option<u8>,
    pub bool,
);

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
    old: Query<(Entity, &KartSlot), With<DummyKart>>,
    drive: Option<Res<crate::drive::Drive>>,
    mut builder: KartBuilder,
    mut toasts: MessageWriter<crate::toast::Toast>,
    mut brains: ResMut<crate::learned::Brains>,
) {
    use crate::command::{Action, DummyKind};
    use std::hash::BuildHasher;
    for action in actions.read() {
        let (count, kind) = match action {
            Action::DebugSpawnKarts { count, kind } => (*count, kind.clone()),
            Action::DebugSpectate => (0, DummyKind::Static),
            _ => continue,
        };
        if lobby.online.is_some() || loading.is_some() || drive.is_none() {
            toasts.write(
                crate::toast::Toast::new("Not now: offline, once the level is up", 2500)
                    .color(Color::srgb(1.0, 0.4, 0.4)),
            );
            continue;
        }
        // (Temporary) `/debug spectate`: ours turns bot, and a dummy of slot 0 draws it (ours
        // is hidden, as a spectator's is). Off, that dummy goes and ours drives again.
        if matches!(action, Action::DebugSpectate) {
            if dummies.1.is_some() {
                for (entity, slot) in &old {
                    if slot.0 == 0 {
                        builder.commands.entity(entity).try_despawn();
                    }
                }
                dummies.1 = None;
                toasts.write(crate::toast::Toast::new("Spectating off", 2000));
            } else if dummies.0.is_empty() {
                toasts.write(
                    crate::toast::Toast::new("No dummy karts to watch (/debug spawn-karts)", 2500)
                        .color(Color::srgb(1.0, 0.4, 0.4)),
                );
            } else if let Some(drive) = &drive {
                let spec = KartSpec {
                    bot: !dummies.3,
                    line: dummies.2,
                    ..drive.spec()
                };
                let (model, wheel_icons) = crate::load_racer(spec.character);
                let birth = race::Kart::birth(&track.0, 0, &spec);
                let kart = builder.spawn(
                    &model,
                    &wheel_icons,
                    Transform::default(),
                    false,
                    Some((0, &birth)),
                );
                builder.commands.entity(kart).insert(DummyKart);
                dummies.1 = Some(1);
                toasts.write(crate::toast::Toast::new(
                    "Spectating: R1 and L1 pick the kart",
                    3000,
                ));
            }
            continue;
        }
        // The policy first: without it, the karts there are stay.
        if let DummyKind::Brain(path) = &kind
            && count > 0
        {
            match brains.load(path.as_deref()) {
                Ok(name) => info!("learned bots: the policy {name}"),
                Err(e) => {
                    toasts.write(
                        crate::toast::Toast::new(e, 4000).color(Color::srgb(1.0, 0.4, 0.4)),
                    );
                    continue;
                }
            }
        }
        for (entity, _) in &old {
            builder.commands.entity(entity).try_despawn();
        }
        dummies.0.clear();
        dummies.1 = None;
        dummies.3 = matches!(kind, DummyKind::Brain(_));
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
                bot: kind == DummyKind::Bot,
                line: dummies.2,
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
            let kind = match kind {
                DummyKind::Bot => "Bots",
                DummyKind::Static => "Static karts",
                DummyKind::Brain(_) => "Learned bots",
            };
            format!("{kind}: {}", names.join(", "))
        };
        toasts.write(crate::toast::Toast::new(text, 3000));
    }
}

/// `/debug bot-line`: every bot drives that bot path (`KartSpec::line`), or each its slot's.
/// The dummies change, so the next tick starts the race over from the grid (as `/debug
/// spawn-karts` does), ours too when it is a bot (`/debug spectate`, see `drive_tick`).
pub fn bot_line(
    mut actions: MessageReader<crate::command::Action>,
    lobby: Res<Lobby>,
    track: Res<Track>,
    mut dummies: ResMut<DummyKarts>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    for action in actions.read() {
        let crate::command::Action::DebugBotLine(line) = *action else {
            continue;
        };
        if lobby.online.is_some() {
            toasts.write(
                crate::toast::Toast::new("Not online", 2500).color(Color::srgb(1.0, 0.4, 0.4)),
            );
            continue;
        }
        dummies.2 = line;
        let learned = dummies.3;
        for spec in dummies.0.iter_mut().filter(|spec| spec.bot || learned) {
            spec.line = line;
        }
        let text = match line {
            None => "(debug) bots drive the path of their slot".to_string(),
            Some(n) if track.0.nav_paths[n as usize].frames.len() < 2 => {
                format!("(debug) bots drive path {n}, which this level lacks: the next one")
            }
            Some(n) => format!("(debug) bots drive path {n}"),
        };
        toasts.write(crate::toast::Toast::new(text, 2500));
    }
}

/// (Temporary) `/debug dump-bots`: prints to stdout what replays the offline race of bots up to
/// now, for `race`'s test `needs_disc_replay_dump`. Its `BOT_DUMP` line rebuilds the race: the
/// race is a pure function of the level, the karts and their pads, and with `/debug spectate`
/// every kart is a bot, so no pads need recording. The checksum lets the test check that it
/// got the very same race. (Without spectating, our own pads would be needed, which nothing
/// keeps; `/debug set wumpa` and the like change the race outside of its pads, so they
/// spoil a dump as well.)
pub fn dump_bots(
    mut actions: MessageReader<crate::command::Action>,
    lobby: Res<Lobby>,
    track: Res<Track>,
    dummies: Res<DummyKarts>,
    sim: Res<crate::props::Sim>,
    drive: Option<Res<crate::drive::Drive>>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    for action in actions.read() {
        if !matches!(action, crate::command::Action::DebugDumpBots) {
            continue;
        }
        if dummies.3 {
            toasts.write(
                crate::toast::Toast::new("Not with learned bots: their pads are not kept", 2500)
                    .color(Color::srgb(1.0, 0.4, 0.4)),
            );
            continue;
        }
        let (Some(watched), Some(drive)) = (dummies.1.filter(|_| lobby.online.is_none()), &drive)
        else {
            toasts.write(
                crate::toast::Toast::new("Only offline, spectating bots (/debug spectate)", 2500)
                    .color(Color::srgb(1.0, 0.4, 0.4)),
            );
            continue;
        };
        let race = sim.race.state();
        let specs: Vec<String> = race
            .karts
            .iter()
            .enumerate()
            .map(|(slot, kart)| {
                // Slot 0 is ours, made a bot (as `drive_tick` builds the race); the rest are the
                // dummies.
                let spec = match slot {
                    0 => race::KartSpec {
                        bot: kart.bot.is_some(),
                        line: dummies.2.filter(|_| kart.bot.is_some()),
                        ..drive.spec()
                    },
                    _ => dummies.0[slot - 1],
                };
                let anim: Vec<String> = spec.anim_num_frames.iter().map(u16::to_string).collect();
                let mut text = format!("{}/{}/{}", spec.character, anim.join(","), spec.bot as u8);
                if let Some(line) = spec.line {
                    text += &format!("/{line}");
                }
                text
            })
            .collect();
        println!(
            "BOT_DUMP level={} collisions={} laps={} tick={} watched={} checksum={:#x} specs={}",
            track.1.0,
            kart_collisions() as u8,
            race.laps,
            race.tick,
            watched,
            race.checksum(),
            specs.join(";"),
        );
        for (slot, kart) in race.karts.iter().enumerate() {
            let d = &kart.driver;
            println!(
                "  kart {slot}{}: pos {:?} vel {:?} speed {} angle {} kart_state {} phys {:?} bot {:?}",
                if slot == watched { " (watched)" } else { "" },
                [d.pos_curr.x >> 8, d.pos_curr.y >> 8, d.pos_curr.z >> 8],
                [d.velocity.x, d.velocity.y, d.velocity.z],
                d.speed,
                d.angle,
                d.kart_state,
                d.phys_state,
                kart.bot,
            );
        }
        toasts.write(crate::toast::Toast::new(
            format!("(debug) race of bots at tick {} dumped to stdout", race.tick),
            2500,
        ));
    }
}
