//! Who is where in an online race, down the left of the screen, as retail draws it
//! (`UI_DrawRankedDrivers`): the places 1 to 4 in the small font, each over the portrait
//! (`render::portraits`) of who holds it, which is half-transparent. The numbers stay on their
//! lines and the portraits move: one slides to its new place when the order changes, and one
//! that gained a place sticks out for a moment (retail's own slide, `UI_Lerp2D_Angular`, is not
//! ported). As in retail, only the first four places are shown, and a place is red once that
//! many karts have finished.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::bigindex::BigIndex;
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::icons::{parse_groups, parse_icons};
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

/// The number of a place, from 0.
#[derive(Component)]
struct PlaceLabel(usize);

/// `UI_RANK_VISIBLE_ARCADE_COUNT`: the places shown.
const VISIBLE: usize = 4;

/// Where the first place's portrait and number are on retail's screen (`UI_RANK_ICON_X`,
/// `UI_RANK_ICON_BASE_Y`, `UI_RANK_TEXT_X`, `UI_RANK_TEXT_START_Y`), and the lines from one place
/// to the next (`UI_RANK_TEXT_SLOT_HEIGHT`).
const ICON_X: f32 = 20.0;
const ICON_Y: f32 = 57.0;
const TEXT_X: f32 = 52.0;
const TEXT_Y: f32 = 56.0;
const SLOT: f32 = 27.0;

/// How fast a row closes in on its place (per second), how far a row that gained a place sticks
/// out at first (in pixels of retail's screen), and how fast that goes (per second).
const SLIDE_RATE: f32 = 12.0;
const POP_DISTANCE: f32 = 9.0;
const POP_DECAY: f32 = 2.5;

pub struct LeaderboardPlugin;

impl Plugin for LeaderboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (build, order).chain().run_if(crate::drive::driving));
    }
}

/// The icons of a racer's pack, which have the HUD's pictures. Every racer's pack has the same
/// ones; character 0's will do.
pub(crate) fn pack_icons() -> Vec<loader::icons::Icon> {
    parse_icons(&pack()).expect("parse racer pack icons")
}

/// The same pack's icon groups.
pub(crate) fn pack_groups() -> Vec<loader::icons::IconGroup> {
    parse_groups(&pack()).expect("parse racer pack icons")
}

fn pack() -> DramFile {
    let path = crate::bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let pack = big
        .read(&mut disc, BigIndex::ARCADE_1P_PACK.get())
        .expect("read racer pack");
    DramFile::parse(&pack).expect("parse racer pack")
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
        let character = (online.characters[slot] as usize).min(ICON_IDS.len() - 1);
        let y = slot as f32 * SLOT;
        commands.spawn((
            Row { slot, y, pop: 0.0 },
            // Stretch: `Auto` would keep the picture's square texels, not the HUD's tall ones.
            ImageNode::new(portraits.clone())
                .with_rect(Rect::new(
                    0.0,
                    (character * HEIGHT) as f32,
                    WIDTH as f32,
                    ((character + 1) * HEIGHT) as f32,
                ))
                .with_mode(NodeImageMode::Stretch),
            Node {
                position_type: PositionType::Absolute,
                left: hud::x(ICON_X),
                top: hud::y(ICON_Y + y),
                width: hud::x(WIDTH as f32),
                height: hud::y(HEIGHT as f32),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(column),
        ));
    }
    // The numbers, over the portraits.
    for place in 0..players.min(VISIBLE) {
        commands.spawn((
            PlaceLabel(place),
            CtrText::new((place + 1).to_string(), Font::Middle, Gradient::WHITE)
                .fixed(hud::PIXEL),
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
