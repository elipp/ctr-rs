//! Who is where in an online race, down the left of the screen, as retail draws it
//! (`UI_DrawRankedDrivers`): the places 1 to 4 in the small font, each over the portrait
//! (`render::portraits`) of who holds it, which is half-transparent. The numbers stay on their
//! lines and the portraits move: one slides to its new place when the order changes, and one
//! that gained a place sticks out for a moment (retail's own slide, `UI_Lerp2D_Angular`, is not
//! ported). As in retail, only the first four places are shown, and a place is red once that
//! many karts have finished.

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::lobby::Lobby;
use crate::portrait_row::{
    HIGHLIGHT, Portraits, portraits, spawn_host_badge, spawn_portrait, spawn_tags,
};

/// The column of rows, with the race it was built for.
#[derive(Component)]
struct Board {
    names: Vec<String>,
    characters: Vec<u8>,
    /// The slots in the order of the race.
    order: Vec<usize>,
}

/// A portrait, which slides to its place when the order changes.
#[derive(Component)]
struct Row {
    slot: usize,
    /// Where the portrait is, in lines down from the first place's.
    y: f32,
    /// 1 right after the row gained a place, down to 0: how far it sticks out of the column.
    pop: f32,
}

/// The name on a portrait, which follows the nicknames.
#[derive(Component)]
struct NameTag(usize);

/// The number of a place, from 0.
#[derive(Component)]
struct PlaceLabel(usize);

/// `UI_RANK_VISIBLE_ARCADE_COUNT`: the places shown.
const VISIBLE: usize = 4;

/// Where the first place's portrait and number are on retail's screen (`UI_RANK_ICON_X`,
/// `UI_RANK_ICON_BASE_Y`, `UI_RANK_TEXT_X`, `UI_RANK_TEXT_START_Y`), and the lines from one place
/// to the next (`UI_RANK_TEXT_SLOT_HEIGHT`).
pub(crate) const ICON_X: f32 = 20.0;
pub(crate) const ICON_Y: f32 = 57.0;
pub(crate) const TEXT_X: f32 = 52.0;
pub(crate) const TEXT_Y: f32 = 56.0;
pub(crate) const SLOT: f32 = 27.0;

/// How fast a row closes in on its place (per second), how far a row that gained a place sticks
/// out at first (in pixels of retail's screen), and how fast that goes (per second).
const SLIDE_RATE: f32 = 12.0;
const POP_DISTANCE: f32 = 9.0;
const POP_DECAY: f32 = 2.5;

pub struct LeaderboardPlugin;

impl Plugin for LeaderboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (build, order, tint, names, highlight)
                .chain()
                .run_if(crate::drive::driving),
        );
    }
}

/// Puts the column up when a race has been built, and takes it down when there is none. (A level
/// switch takes every node away, and a race on the new level builds it again.)
fn build(
    mut commands: Commands,
    lobby: Res<Lobby>,
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    loaded: Option<Res<Portraits>>,
    boards: Query<(Entity, &Board)>,
) {
    let online = lobby.online.as_ref().filter(|online| online.built);
    for (entity, board) in &boards {
        // The same race goes on; a restart keeps the players, so it keeps the column too.
        if online.is_some_and(|o| o.names == board.names && o.characters == board.characters) {
            return;
        }
        commands.entity(entity).despawn();
    }
    let Some(online) = online else {
        return;
    };
    let image = portraits(
        &mut commands,
        loaded.as_deref(),
        &textures.vram,
        &mut images,
    );
    let players = online.characters.len();
    let column = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::ZERO,
                top: Val::ZERO,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    for slot in 0..players {
        let y = slot as f32 * SLOT;
        let portrait =
            spawn_portrait(&mut commands, column, &image, online.characters[slot], ICON_Y + y);
        commands.entity(portrait).insert(Row { slot, y, pop: 0.0 });
        let label = online.labels.get(slot).map_or("", String::as_str);
        let key = online.names.get(slot).map_or("", String::as_str);
        // The host is slot 0 (`net::Event::Start`).
        if slot == 0 {
            spawn_host_badge(&mut commands, portrait, false);
        }
        let tag = spawn_tags(&mut commands, portrait, label, key);
        commands.entity(tag).insert(NameTag(slot));
    }
    // The numbers, over the portraits.
    for place in 0..players.min(VISIBLE) {
        commands.spawn((
            PlaceLabel(place),
            CtrText::new((place + 1).to_string(), Font::Middle, Gradient::WHITE).fixed(hud::PIXEL),
            Node {
                position_type: PositionType::Absolute,
                left: hud::x(TEXT_X),
                top: hud::y(TEXT_Y + place as f32 * SLOT),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(column),
        ));
    }
    commands.entity(column).insert(Board {
        names: online.names.clone(),
        characters: online.characters.clone(),
        order: (0..players).collect(),
    });
}

/// Works out the order of the race, numbers the rows and slides them to their places.
fn order(
    sim: Res<crate::props::Sim>,
    time: Res<Time<Real>>,
    mut boards: Query<&mut Board>,
    mut labels: Query<(&PlaceLabel, &mut CtrText)>,
    mut rows: Query<(&mut Row, &mut Node, &mut Visibility)>,
) {
    let Ok(mut board) = boards.single_mut() else {
        return;
    };
    let karts = &sim.race.karts;
    let count = board.order.len();
    if karts.len() == count {
        let mut slots: Vec<usize> = (0..count).collect();
        // A kart with no place yet (-1) goes after the ones that have one.
        slots.sort_by_key(|&slot| {
            let rank = karts[slot].driver.driver_rank;
            (rank < 0, rank, slot)
        });
        if slots != board.order {
            for (mut row, ..) in &mut rows {
                let place = |order: &[usize]| order.iter().position(|&s| s == row.slot);
                if place(&slots) < place(&board.order) {
                    row.pop = 1.0;
                }
            }
            board.order = slots;
        }
    }
    let finished = karts
        .iter()
        .filter(|k| k.driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0)
        .count();
    for (PlaceLabel(place), mut text) in &mut labels {
        let gradient = if *place < finished {
            Gradient::RED
        } else {
            Gradient::WHITE
        };
        if text.gradient != gradient {
            text.gradient = gradient;
        }
    }
    let dt = time.delta_secs();
    let follow = 1.0 - (-SLIDE_RATE * dt).exp();
    for (mut row, mut node, mut visibility) in &mut rows {
        let Some(place) = board.order.iter().position(|&s| s == row.slot) else {
            continue;
        };
        row.y += (place as f32 * SLOT - row.y) * follow;
        row.pop = (row.pop - POP_DECAY * dt).max(0.0);
        node.top = hud::y(ICON_Y + row.y);
        node.left = hud::x(ICON_X + row.pop * POP_DISTANCE);
        visibility.set_if_neq(if place < VISIBLE {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// `UI_Rank_GetDamageColor`: the multiplier of a portrait's colours `[r, g, b]` for a driver whose
/// `damageColorTimer` is `timer`, in 0x80ths (0x80 is the picture as it is, 0xff nearly twice as
/// bright): red the moment a weapon hurt the driver (positive), green for a green beaker
/// (negative), and back to neutral over 30 ticks.
fn damage_color(timer: i16) -> [i32; 3] {
    const FRAMES: i32 = 30;
    const STEP: i32 = 4;
    const MAX: i32 = 0xff;
    let timer = timer as i32;
    if timer < 0 {
        let strength = (timer + FRAMES) * STEP;
        [strength, MAX - strength, strength]
    } else if timer > 0 {
        let strength = (FRAMES - timer) * STEP;
        [MAX - strength, strength, strength]
    } else {
        [0x80; 3]
    }
}

/// Tints the portraits by how recently a weapon hurt their karts.
fn tint(sim: Res<crate::props::Sim>, mut rows: Query<(&Row, &mut ImageNode)>) {
    for (row, mut image) in &mut rows {
        let Some(kart) = sim.race.karts.get(row.slot) else {
            continue;
        };
        let [r, g, b] = damage_color(kart.driver.damage_color_timer).map(|c| c as f32 / 128.0);
        let color = Color::LinearRgba(LinearRgba::new(r, g, b, 1.0));
        if image.color != color {
            image.color = color;
        }
    }
}

/// Keeps the names on the portraits those of the nicknames now.
fn names(lobby: Res<Lobby>, mut tags: Query<(&NameTag, &mut Text)>) {
    let Some(online) = &lobby.online else {
        return;
    };
    for (NameTag(slot), mut text) in &mut tags {
        if let Some(label) = online.labels.get(*slot)
            && text.0 != *label
        {
            text.0.clone_from(label);
        }
    }
}

/// Colours the name of the kart the view follows (ours, or the one a spectator watches) in
/// [`HIGHLIGHT`].
fn highlight(lobby: Res<Lobby>, mut tags: Query<(&NameTag, &mut TextColor)>) {
    let Some(online) = &lobby.online else {
        return;
    };
    let viewed = online.viewed();
    for (NameTag(slot), mut text) in &mut tags {
        let color = if *slot == viewed {
            HIGHLIGHT
        } else {
            Color::WHITE
        };
        if text.0 != color {
            text.0 = color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hurt_icon_goes_red_or_green_and_back_to_neutral() {
        assert_eq!(damage_color(0), [0x80; 3]);
        assert_eq!(damage_color(30), [0xff, 0, 0]);
        assert_eq!(damage_color(-30), [0, 0xff, 0]);
        assert_eq!(damage_color(1), [0xff - 116, 116, 116]);
        assert_eq!(damage_color(-1), [116, 0xff - 116, 116]);
    }
}
