//! The minimap at the bottom right, as retail draws it with one player (`UI_Map_DrawMap`,
//! `UI_Map_DrawDrivers`): the level's picture of the track (`render::minimap`), and a dot on it
//! for each kart. A bot's dot is its character's colour; a player's flashes between that and
//! white. Ours is on top of the others.
//!
//! The picture blends as the PS1 draws it: its texels with the STP bit (the grey edge of the
//! track) add to the scene behind, the others cover it. The UI's images are alpha blended, so the
//! picture is a node of its own material, premultiplied.

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, BlendState, RenderPipelineDescriptor};
use bevy::shader::ShaderRef;
use loader::icons::parse_level_icons;
use loader::lev::Minimap;
use render::minimap::{
    BOT_DOT, BOTTOM, CHARACTER_COLORS, PLAYER_DOT, RIGHT, WHITE, dot_image, icon_pos, map_image,
};

use crate::hud;
use crate::leaderboard::{atlas_image, pack_groups};
use crate::{Textures, Track};

/// The picture's material: its texels as they are, blended premultiplied.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct MapMaterial {
    #[texture(0)]
    #[sampler(1)]
    picture: Handle<Image>,
}

impl UiMaterial for MapMaterial {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_PKG_NAME"), "/minimap.wgsl").into()
    }

    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        for target in descriptor
            .fragment
            .iter_mut()
            .flat_map(|f| f.targets.iter_mut())
            .flatten()
        {
            target.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        }
    }
}

/// The picture, with the level's layout and the picture's size in texels.
#[derive(Component)]
struct Map {
    layout: Minimap,
    size: (usize, usize),
}

/// The dot of the kart in slot `.0`.
#[derive(Component)]
struct Dot(usize);

/// The dots tinted each colour (by character, then white), and their size in texels.
#[derive(Resource)]
struct DotImages {
    bot: Vec<Handle<Image>>,
    player: Vec<Handle<Image>>,
    size: (usize, usize),
}

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "minimap.wgsl");
        app.add_plugins(UiMaterialPlugin::<MapMaterial>::default())
            .add_systems(Update, (build, show).chain().run_if(crate::drive::driving));
    }
}

/// Puts the picture up for the level, when there is none or the level changed; the dots' images
/// are made once.
fn build(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    dots: Option<Res<DotImages>>,
    maps: Query<Entity, With<Map>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<MapMaterial>>,
    mut none: Local<bool>,
) {
    if dots.is_none() {
        let groups = pack_groups();
        let colors: Vec<u32> = CHARACTER_COLORS.iter().copied().chain([WHITE]).collect();
        let tinted = |id| -> Option<Vec<_>> {
            colors
                .iter()
                .map(|&c| dot_image(&groups, &textures.vram, id, c))
                .collect()
        };
        let (Some(bot), Some(player)) = (tinted(BOT_DOT), tinted(PLAYER_DOT)) else {
            panic!("the racer pack has the minimap's dots");
        };
        let size = (player[0].width, player[0].height);
        let mut handles = |atlases: Vec<_>| {
            atlases
                .into_iter()
                .map(|a| atlas_image(a, &mut images))
                .collect()
        };
        commands.insert_resource(DotImages {
            bot: handles(bot),
            player: handles(player),
            size,
        });
    }
    if track.is_changed() {
        *none = false;
        for map in &maps {
            commands.entity(map).despawn();
        }
    } else if !maps.is_empty() || *none {
        return;
    }
    let layout = Minimap::parse(&textures.dram).ok().flatten();
    let icons = parse_level_icons(&textures.dram).unwrap_or_default();
    let Some((layout, picture)) = layout.and_then(|layout| {
        Some((
            layout,
            map_image(&icons, &textures.vram, layout.top_half_mode == 0)?,
        ))
    }) else {
        *none = true;
        return;
    };
    let size = (picture.width, picture.height);
    commands.spawn((
        Map { layout, size },
        MaterialNode(materials.add(MapMaterial {
            picture: atlas_image(picture, &mut images),
        })),
        Node {
            position_type: PositionType::Absolute,
            right: hud::x((hud::WIDTH - RIGHT) as f32),
            bottom: hud::y((hud::HEIGHT - BOTTOM) as f32),
            width: hud::x(size.0 as f32),
            height: hud::y(size.1 as f32),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

/// Moves each kart's dot to where it is, and tints it.
fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    images: Option<Res<DotImages>>,
    maps: Query<(Entity, &Map)>,
    mut dots: Query<(Entity, &Dot, &mut ImageNode, &mut Node, &mut ZIndex)>,
) {
    let (Ok((map_entity, map)), Some(images)) = (maps.single(), images) else {
        return;
    };
    let karts = &sim.race.karts;
    for (entity, dot, ..) in &dots {
        if dot.0 >= karts.len() {
            commands.entity(entity).despawn();
        }
    }
    // The picture's top left on retail's screen, which the dots are placed from.
    let (left, top) = (RIGHT - map.size.0 as i32, BOTTOM - map.size.1 as i32);
    let flash = sim.race.objects.ticks() & 2 != 0;
    for (slot, kart) in karts.iter().enumerate() {
        let d = &kart.driver;
        let character = (d.character as usize).min(CHARACTER_COLORS.len() - 1);
        let image = if d.actions_flag_set & vehicle::actions::BOT != 0 {
            &images.bot[character]
        } else if flash {
            &images.player[character]
        } else {
            images.player.last().unwrap()
        };
        let (x, y) = icon_pos(&map.layout, d.pos_curr.x >> 8, d.pos_curr.z >> 8);
        let node = Node {
            position_type: PositionType::Absolute,
            left: hud::x((x - left) as f32),
            top: hud::y((y - top) as f32),
            width: hud::x(images.size.0 as f32),
            height: hud::y(images.size.1 as f32),
            ..default()
        };
        let z = ZIndex(if slot == sim.own { 1 } else { 0 });
        match dots.iter_mut().find(|(_, dot, ..)| dot.0 == slot) {
            Some((_, _, mut current, mut current_node, mut current_z)) => {
                if current.image != *image {
                    current.image = image.clone();
                }
                current_node.set_if_neq(node);
                current_z.set_if_neq(z);
            }
            None => {
                commands.spawn((
                    Dot(slot),
                    ImageNode::new(image.clone()).with_mode(NodeImageMode::Stretch),
                    node,
                    z,
                    Pickable::IGNORE,
                    ChildOf(map_entity),
                ));
            }
        }
    }
}
