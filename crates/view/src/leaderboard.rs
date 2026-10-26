//! The leaderboard of an online race, down the right side of the screen: a row for each player
//! with their place, their character's portrait (`render::portraits`) and the short form of
//! their iroh key and the round trip to them (ping), the leader on top. Our own row is yellow.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::bigindex::BigIndex;
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::icons::parse_icons;
use render::portraits::{HEIGHT, ICON_IDS, WIDTH, portrait_atlas};

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

/// The round trip to a player in a row, in milliseconds (nothing for our own row).
#[derive(Component)]
struct PingLabel(usize);

const OWN_COLOR: Color = Color::srgb(1.0, 0.9, 0.2);

/// How many times larger than on the PlayStation a portrait is drawn.
const PORTRAIT_SCALE: f32 = 1.0;

/// Where the column starts: under the lap times.
const TOP: f32 = 230.0;

/// The height of a row, and the distance from one row's top to the next.
const ROW_HEIGHT: f32 = 30.0;
const PITCH: f32 = ROW_HEIGHT + 4.0;
const COLUMN_WIDTH: f32 = 210.0;

/// How fast a row closes in on its place (per second), how far a row that gained a place sticks
/// out at first, and how fast that goes (per second).
const SLIDE_RATE: f32 = 12.0;
const POP_DISTANCE: f32 = 18.0;
const POP_DECAY: f32 = 2.5;

pub struct LeaderboardPlugin;

impl Plugin for LeaderboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (build, order, pings).chain());
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
    let column = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(TOP),
                right: Val::Px(20.0),
                width: Val::Px(COLUMN_WIDTH),
                height: Val::Px(online.characters.len() as f32 * PITCH),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    (0..online.characters.len()).for_each(|slot| {
        let own = slot == online.slot;
        let color = if own { OWN_COLOR } else { Color::WHITE };
        let character = (online.characters[slot] as usize).min(ICON_IDS.len() - 1);
        let row = commands
            .spawn((
                Row {
                    slot,
                    y: slot as f32 * PITCH,
                    pop: 0.0,
                },
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(slot as f32 * PITCH),
                    right: Val::Px(0.0),
                    height: Val::Px(ROW_HEIGHT),
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                ChildOf(column),
            ))
            .id();
        commands.spawn((
            PlaceLabel(slot),
            Text::new(""),
            TextFont {
                font_size: 20.0.into(),
                ..default()
            },
            TextColor(color),
            ChildOf(row),
        ));
        commands.spawn((
            ImageNode::new(portraits.clone()).with_rect(Rect::new(
                0.0,
                (character * HEIGHT) as f32,
                WIDTH as f32,
                ((character + 1) * HEIGHT) as f32,
            )),
            Node {
                width: Val::Px(WIDTH as f32 * PORTRAIT_SCALE),
                height: Val::Px(HEIGHT as f32 * PORTRAIT_SCALE),
                ..default()
            },
            ChildOf(row),
        ));
        commands.spawn((
            Text::new(online.names[slot].clone()),
            TextFont {
                font_size: 18.0.into(),
                ..default()
            },
            TextColor(color),
            ChildOf(row),
        ));
        commands.spawn((
            PingLabel(slot),
            Text::new(""),
            TextFont {
                font_size: 14.0.into(),
                ..default()
            },
            TextColor(Color::srgb(0.7, 0.7, 0.7)),
            ChildOf(row),
        ));
        let _ = row;
    });
    commands.entity(column).insert(Board {
        names: online.names.clone(),
        characters: online.characters.clone(),
        order: (0..online.characters.len()).collect(),
    });
}

/// Works out the order of the race, numbers the rows and slides them to their places.
fn order(
    sim: Res<crate::props::Sim>,
    time: Res<Time<Real>>,
    mut boards: Query<&mut Board>,
    mut labels: Query<(&PlaceLabel, &mut Text)>,
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
            if text.0 != label {
                text.0 = label;
            }
        }
    }
    let dt = time.delta_secs();
    let follow = 1.0 - (-SLIDE_RATE * dt).exp();
    for (mut row, mut node) in &mut rows {
        let Some(place) = board.order.iter().position(|&s| s == row.slot) else {
            continue;
        };
        row.y += (place as f32 * PITCH - row.y) * follow;
        row.pop = (row.pop - POP_DECAY * dt).max(0.0);
        node.top = Val::Px(row.y);
        node.right = Val::Px(-row.pop * POP_DISTANCE);
    }
}

/// Writes the round trip to each player (the last ping) in their row.
fn pings(lobby: Res<Lobby>, mut labels: Query<(&PingLabel, &mut Text)>) {
    let Some(online) = &lobby.online else {
        return;
    };
    for (PingLabel(slot), mut text) in &mut labels {
        let ping = online.names.get(*slot).and_then(|name| lobby.pings.get(name));
        let label = match ping {
            Some(rtt) => format!("{:.0} ms", rtt.as_secs_f32() * 1000.0),
            None => String::new(),
        };
        if text.0 != label {
            text.0 = label;
        }
    }
}
