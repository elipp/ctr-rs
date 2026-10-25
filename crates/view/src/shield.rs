//! The shield bubble of a kart that holds one, as Bevy entities. How big it is, how transparent
//! and where its highlight is, is `vehicle::shield` (`Driver::shield`); the models are
//! `render::shield`'s. The bubble is the dark layer and the coloured one (blue when juiced, green
//! if not), which face the camera wherever the kart goes (they are a half-sphere), and the highlight that
//! sweeps across them. A green bubble fades over its last two seconds, and pops in a squash and a shrink.
//!
//! Our own kart's bubble is in [`crate::SHIELD_LAYER`], which a camera draws over the kart's
//! (retail files it after the kart in the ordering table); the others' is in the world, where the
//! walls hide it.
//!
//! The three layers and both colours are made for each kart (hidden); the ones the shield is, show.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::model::Model;
use loader::vram::Vram;
use render::mesh;
use render::shield::Shields;
use vehicle::shield::Shield;

use crate::DEPTH_BIAS_PER_ORDER;
use crate::drive::{ChaseCamera, animation_pose, rotation_of};
use crate::kart::{Attached, KartMotion, KartSlot};
use crate::spawn_parts;
use crate::subtract::SubtractMaterial;

/// How deep the highlight's model is next to the bubble's (`ctr-query models`: the bubble is a
/// dome 117 deep, the highlight a strip of a sphere 160 deep), which makes it stand out of the
/// bubble towards the camera. It is flattened by this to lie on the dome.
const HIGHLIGHT_FLATTEN: f32 = 117.0 / 160.0;

/// `alphaScale` at which the bubble is gone.
const ALPHA_GONE: f32 = 0x1000 as f32;

/// The shield's models, from the racer pack.
#[derive(Resource)]
pub struct ShieldModels(pub Option<Shields>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    Dark,
    Blue,
    Green,
    Highlight,
}

/// One layer of a kart's shield.
#[derive(Component)]
pub struct ShieldPart {
    layer: Layer,
    /// The alpha the materials have.
    alpha: u16,
}

/// Makes the four layers of `kart`'s shields, hidden.
pub fn spawn(
    shields: &Shields,
    vram: &Vram,
    kart: Entity,
    layer: usize,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    for (which, model) in [
        (Layer::Dark, &shields.dark),
        (Layer::Blue, &shields.blue),
        (Layer::Green, &shields.green),
        (Layer::Highlight, &shields.highlight),
    ] {
        spawn_part(
            which,
            model,
            vram,
            kart,
            layer,
            commands,
            images,
            meshes,
            materials,
            subtract_materials,
        );
    }
}

fn spawn_part(
    which: Layer,
    model: &Model,
    vram: &Vram,
    kart: Entity,
    layer: usize,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    let Some(header) = model.headers.first() else {
        return;
    };
    let Some(pose) = animation_pose(header, 0, 0).or_else(|| header.static_frame.clone()) else {
        return;
    };
    // Unscaled: the bubble's size is set on the entity.
    let data = mesh::model_mesh(header, &pose, vram, [0x1000; 3]);
    let mut atlas = Image::new(
        Extent3d {
            width: data.atlas.width as u32,
            height: data.atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data.atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = ImageSampler::nearest();
    // The layers are the same sphere, so they fight over the depth buffer (the coloured one
    // flickers away): the dark one is farthest back. The coloured one and the highlight stand
    // together over it, as they did before, so that the highlight is not pulled in front of the
    // bubble (the bias also sorts the blended parts).
    let order = match which {
        Layer::Dark => 0,
        Layer::Blue | Layer::Green | Layer::Highlight => 3,
    };
    let base = StandardMaterial {
        depth_bias: order as f32 * DEPTH_BIAS_PER_ORDER,
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(atlas)),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    let group = commands
        .spawn((Transform::default(), Visibility::Hidden))
        .id();
    spawn_parts(
        data.parts,
        &base,
        Transform::default(),
        Some(group),
        commands,
        meshes,
        materials,
        subtract_materials,
        layer,
    );
    commands.entity(group).insert((
        Attached(kart),
        ShieldPart {
            layer: which,
            alpha: 0,
        },
    ));
}

/// Shows each driver's shield round its kart, in the size and transparency the sim has.
pub fn update(
    sim: Res<crate::props::Sim>,
    fixed: Res<Time<Fixed>>,
    karts: Query<(&KartSlot, &KartMotion)>,
    cameras: Query<&Transform, (With<ChaseCamera>, Without<ShieldPart>)>,
    mut instances: Query<(
        &mut ShieldPart,
        &Attached,
        &mut Transform,
        &mut Visibility,
        &Children,
    )>,
    mut parts: Query<
        (
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&MeshMaterial3d<SubtractMaterial>>,
            &mut Transform,
        ),
        (Without<ShieldPart>, Without<ChaseCamera>),
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let t = fixed.overstep_fraction();
    // The bubble is a half-sphere that is drawn facing the camera (`DRAW_BILLBOARD`).
    let Ok(camera) = cameras.single().map(|c| c.rotation) else {
        return;
    };

    for (mut part, attached, mut transform, mut visibility, children) in &mut instances {
        let Ok((slot, motion)) = karts.get(attached.0) else {
            continue;
        };
        let shield: Option<Shield> = sim.race.karts.get(slot.0).and_then(|k| k.driver.shield);
        let shown = shield.filter(|s| match part.layer {
            Layer::Dark => true,
            Layer::Blue => s.blue,
            Layer::Green => !s.blue,
            Layer::Highlight => !s.highlight_hidden,
        });
        let Some(shield) = shown else {
            *visibility = Visibility::Hidden;
            continue;
        };

        // (The coloured layer and the dark one scale together; the highlight keeps its own.)
        let [round, along] = if part.layer == Layer::Highlight {
            shield.highlight_scale
        } else {
            shield.scale
        };
        let (round, along) = (round as f32 / 4096.0, along as f32 / 4096.0);
        transform.translation = motion.pose(t).translation;
        transform.rotation = camera;
        // The highlight turns about the bubble in its parts, under the flattening of its group
        // (which is in the camera's axes, as the dome is).
        let (depth, turn) = if part.layer == Layer::Highlight {
            (
                HIGHLIGHT_FLATTEN,
                rotation_of(&vehicle::stuck::MaskHead::rotation(shield.highlight_rot_y)),
            )
        } else {
            (1.0, Quat::IDENTITY)
        };
        transform.scale = Vec3::new(round, along, round * depth);
        for child in children {
            if let Ok((_, _, mut t)) = parts.get_mut(*child) {
                t.rotation = turn;
            }
        }
        *visibility = Visibility::Inherited;

        // It fades with the bubble: in the colour, and in the alpha of the blended parts.
        if part.alpha != shield.alpha_scale {
            part.alpha = shield.alpha_scale;
            // (The root of it keeps the ends, solid and gone, and makes the rest more opaque.)
            let fade = (1.0 - part.alpha as f32 / ALPHA_GONE)
                .clamp(0.0, 1.0)
                .sqrt();
            let dim = |m: &mut StandardMaterial| {
                let masked = matches!(m.alpha_mode, AlphaMode::Mask(_));
                m.base_color =
                    Color::linear_rgba(fade, fade, fade, if masked { 1.0 } else { fade });
            };
            for child in children {
                let Ok((standard, subtract, _)) = parts.get(*child) else {
                    continue;
                };
                if let Some(mut m) = standard.and_then(|h| materials.get_mut(&h.0)) {
                    dim(&mut m);
                }
                if let Some(mut m) = subtract.and_then(|h| subtract_materials.get_mut(&h.0)) {
                    dim(&mut m.base);
                }
            }
        }
    }
}
