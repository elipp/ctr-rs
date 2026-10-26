//! Who is where in an online race, down the left of the screen under the times, as retail shows
//! it: each player's portrait (`render::portraits`) in a black frame, with their place at its top
//! right in the middle font, the leader on top. A row slides to its new place when the order
//! changes, and one that gained a place sticks out for a moment. Our own place is orange and the
//! others' white: retail's are all white, but online two players may drive the same character.
//! The places and sizes are measured from retail screenshots.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::bigindex::BigIndex;
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::icons::parse_icons;
use render::font::{Font, Gradient};
use render::portraits::{HEIGHT, ICON_IDS, WIDTH, portrait_atlas};

use crate::font::CtrText;
use crate::hud;
use crate::lobby::Lobby;

/// The portraits as a picture, loaded when the first race needs it.
#[derive(Resource)]
struct Portraits(Handle<Image>);

/// The column of rows, with the race it was built for.
#[derive(Component)]
struct Board {
    names: Vec<String>,
    characters: Vec<u8>,
    /// The slots in the order of the race.
    order: Vec<usize>,
    /// The distance from one row's top to the next, in logical pixels.
    pitch: f32,
}

/// A row, which slides to its place when the order changes.
#[derive(Component)]
struct Row {
    slot: usize,
    /// Where the row is, in pixels from the top of the column.
    y: f32,
    /// 1 right after the row gained a place, down to 0: how far it sticks out of the column.
    pop: f32,
}

/// The place number in a row.
#[derive(Component)]
struct PlaceLabel(usize);

/// Where the column starts, in pixels and lines of retail's screen: under the times.
const LEFT: f32 = 12.0;
const TOP: f32 = 66.0;

/// How far down the rows may go: above the big place at the bottom left. More players than fit
/// at full size make the rows smaller.
const BOTTOM: f32 = 176.0;

/// The frame round a portrait, and the space between two rows, in pixels of retail's screen.
const FRAME: f32 = 1.0;
const GAP: f32 = 2.0;

/// How far the place number is in from the portrait's right edge, and down from its top.
const NUMBER_IN: Vec2 = Vec2::new(2.0, 1.0);

/// How fast a row closes in on its place (per second), how far a row that gained a place sticks
/// out at first (in pixels of retail's screen), and how fast that goes (per second).
const SLIDE_RATE: f32 = 12.0;
const POP_DISTANCE: f32 = 9.0;
const POP_DECAY: f32 = 2.5;

pub struct LeaderboardPlugin;

impl Plugin for LeaderboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (build, order).chain());
    }
}

/// The icons of a racer's pack, which have the HUD's pictures. Every racer's pack has the same
/// ones; character 0's will do.
pub(crate) fn pack_icons() -> Vec<loader::icons::Icon> {
    let path = crate::bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let pack = big
        .read(&mut disc, BigIndex::ARCADE_1P_PACK.get())
        .expect("read racer pack");
    parse_icons(&DramFile::parse(&pack).expect("parse racer pack")).expect("parse racer pack icons")
}

/// A picture of the HUD as a texture, for the UI to cut from.
pub(crate) fn atlas_image(atlas: render::mesh::Atlas, images: &mut Assets<Image>) -> Handle<Image> {
    let mut image = Image::new(
        Extent3d {
            width: atlas.width as u32,
            height: atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    images.add(image)
}

fn load_portraits(vram: &loader::vram::Vram, images: &mut Assets<Image>) -> Handle<Image> {
    atlas_image(portrait_atlas(&pack_icons(), vram), images)
}

/// How much smaller than retail's the rows of `players` are, to fit above the big place: 1 when
/// they fit as they are.
fn row_scale(players: usize) -> f32 {
    let row = HEIGHT as f32 + 2.0 * FRAME + GAP;
    ((BOTTOM - TOP) / (players as f32 * row)).min(1.0)
}

/// Puts the column up when a race has been built, and takes it down when there is none. (A level
/// switch takes every node away, and a race on the new level builds it again.)
fn build(
    mut commands: Commands,
    lobby: Res<Lobby>,
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    portraits: Option<Res<Portraits>>,
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
    let portraits = match portraits {
        Some(portraits) => portraits.0.clone(),
        None => {
            let handle = load_portraits(&textures.vram, &mut images);
            commands.insert_resource(Portraits(handle.clone()));
            handle
        }
    };
    let players = online.characters.len();
    let scale = row_scale(players);
    // A pixel of retail's screen, at the rows' size.
    let pixel = hud::PIXEL * scale;
    let (across, down) = (|n: f32| Val::Px(n * pixel.x), |n: f32| Val::Px(n * pixel.y));
    let pitch = (HEIGHT as f32 + 2.0 * FRAME + GAP) * pixel.y;
    let column = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: hud::x(LEFT),
                top: hud::y(TOP),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    for slot in 0..players {
        let character = (online.characters[slot] as usize).min(ICON_IDS.len() - 1);
        let row = commands
            .spawn((
                Row {
                    slot,
                    y: slot as f32 * pitch,
                    pop: 0.0,
                },
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(slot as f32 * pitch),
                    left: Val::ZERO,
                    border: UiRect::axes(across(FRAME), down(FRAME)),
                    ..default()
                },
                BorderColor::all(Color::BLACK),
                Pickable::IGNORE,
                ChildOf(column),
            ))
            .id();
        commands.spawn((
            ImageNode::new(portraits.clone()).with_rect(Rect::new(
                0.0,
                (character * HEIGHT) as f32,
                WIDTH as f32,
                ((character + 1) * HEIGHT) as f32,
            )),
            Node {
                width: across(WIDTH as f32),
                height: down(HEIGHT as f32),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(row),
        ));
        let gradient = if slot == online.slot {
            Gradient::ORANGE
        } else {
            Gradient::WHITE
        };
        commands.spawn((
            PlaceLabel(slot),
            CtrText::new("", Font::Middle, gradient).fixed(pixel),
            Node {
                position_type: PositionType::Absolute,
                right: across(NUMBER_IN.x),
                top: down(NUMBER_IN.y),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(row),
        ));
    }
    commands.entity(column).insert(Board {
        names: online.names.clone(),
        characters: online.characters.clone(),
        order: (0..players).collect(),
        pitch,
    });
}

/// Works out the order of the race, numbers the rows and slides them to their places.
fn order(
    sim: Res<crate::props::Sim>,
    time: Res<Time<Real>>,
    mut boards: Query<&mut Board>,
    mut labels: Query<(&PlaceLabel, &mut CtrText)>,
    mut rows: Query<(&mut Row, &mut Node)>,
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
            for (mut row, _) in &mut rows {
                let place = |order: &[usize]| order.iter().position(|&s| s == row.slot);
                if place(&slots) < place(&board.order) {
                    row.pop = 1.0;
                }
            }
            board.order = slots;
        }
    }
    for (PlaceLabel(slot), mut text) in &mut labels {
        if let Some(place) = board.order.iter().position(|s| s == slot) {
            let label = (place + 1).to_string();
            if text.text != label {
                text.text = label;
            }
        }
    }
    let dt = time.delta_secs();
    let follow = 1.0 - (-SLIDE_RATE * dt).exp();
    for (mut row, mut node) in &mut rows {
        let Some(place) = board.order.iter().position(|&s| s == row.slot) else {
            continue;
        };
        row.y += (place as f32 * board.pitch - row.y) * follow;
        row.pop = (row.pop - POP_DECAY * dt).max(0.0);
        node.top = Val::Px(row.y);
        node.left = hud::x(row.pop * POP_DISTANCE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_shrink_to_fit_above_the_place() {
        assert_eq!(row_scale(1), 1.0);
        assert_eq!(row_scale(3), 1.0);
        let row = HEIGHT as f32 + 2.0 * FRAME + GAP;
        for players in 4..=8 {
            let scale = row_scale(players);
            assert!(scale < 1.0);
            assert!(players as f32 * row * scale <= BOTTOM - TOP + 0.001);
        }
    }
}
