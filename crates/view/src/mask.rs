//! The Aku Aku or Uka Uka mask of a mask grab as Bevy entities. How it moves, when it shows and
//! how big it is, is `vehicle::stuck` (`Driver::mask`): it floats round the kart turning, with a
//! beam of light in it, comes down from above the kart while the grab holds it, and shrinks away
//! after. The models are `render::mask`'s.
//!
//! Both masks and both beams are made once (hidden); the one that the driver's mask is, shows.
//! The head stays in the first pose of its animation, as in the game: only the beam animates.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::model::{Model, ModelHeader};
use render::mask::Masks;
use render::mesh::{self, PARTS, SCALE};
use vehicle::stuck::{MaskHead, MaskShown, Placed};

use crate::drive::{Drive, animation_pose, interpolated, rotation_of};
use crate::subtract::SubtractMaterial;
use crate::{KART_LAYER, Textures, spawn_parts};

/// `alphaScale` at which the beam is gone.
const ALPHA_GONE: f32 = 0x1000 as f32;

/// The masks, from the racer pack.
#[derive(Resource)]
pub struct MaskModels(pub Option<Masks>);

/// Where one of the mask's instances is, at a tick.
#[derive(Clone, Copy)]
struct Placement {
    /// From the kart, or from `base` in the world.
    offset: Vec3,
    /// On top of the kart's rotation, or in the world.
    rotation: Quat,
    scale: f32,
    base: Vec3,
    world: bool,
}

impl Placement {
    fn new(mask: &MaskHead, shown: &MaskShown, placed: &Placed) -> Placement {
        let at = |v: [i16; 3]| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32) * SCALE;
        Placement {
            offset: at(placed.offset),
            rotation: placed.rot_y.map_or(Quat::IDENTITY, |y| rotation_of(&MaskHead::rotation(y))),
            scale: placed.scale.max(0) as f32 / 4096.0,
            base: at(mask.pos),
            world: shown.world_space,
        }
    }

    /// The transform in the world, given the kart's.
    fn transform(&self, kart: &Transform) -> Transform {
        let (translation, rotation) = if self.world {
            (self.base + self.offset, self.rotation)
        } else {
            (kart.translation + kart.rotation * self.offset, kart.rotation * self.rotation)
        };
        Transform { translation, rotation, scale: Vec3::splat(self.scale) }
    }
}

/// One instance of a mask: its head or its beam.
#[derive(Component)]
pub struct MaskPart {
    good_guy: bool,
    beam: bool,
    header: ModelHeader,
    parts: [Option<Handle<Mesh>>; PARTS],
    /// The frame of the animation the mesh has, and the alpha of its materials.
    frame: i16,
    alpha: u16,
    /// The tick the placements are of, and the placement at it and the one before.
    tick: u32,
    previous: Option<Placement>,
    current: Option<Placement>,
}

/// Makes the four instances, hidden.
pub fn spawn(
    models: Res<MaskModels>,
    textures: Res<Textures>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let Some(masks) = &models.0 else { return };
    for good_guy in [true, false] {
        let mask = masks.of(good_guy);
        for (beam, model) in [(false, &mask.head), (true, &mask.beam)] {
            spawn_part(
                good_guy,
                beam,
                model,
                &textures,
                &mut commands,
                &mut images,
                &mut meshes,
                &mut materials,
                &mut subtract_materials,
            );
        }
    }
}

fn spawn_part(
    good_guy: bool,
    beam: bool,
    model: &Model,
    textures: &Textures,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) {
    let header = &model.headers[0];
    let Some(pose) = animation_pose(header, 0, 0) else { return };
    // Unscaled: the mask's own size is set on the entity.
    let data = mesh::model_mesh(header, &pose, &textures.vram, [0x1000; 3]);
    let mut atlas = Image::new(
        Extent3d { width: data.atlas.width as u32, height: data.atlas.height as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data.atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = ImageSampler::nearest();
    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(atlas)),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    let group = commands.spawn((Transform::default(), Visibility::Hidden)).id();
    let parts = spawn_parts(data.parts, &base, Transform::default(), Some(group), commands, meshes, materials, subtract_materials, KART_LAYER);
    commands.entity(group).insert(MaskPart {
        good_guy,
        beam,
        header: header.clone(),
        parts,
        frame: 0,
        alpha: 0,
        tick: 0,
        previous: None,
        current: None,
    });
}

/// Shows the driver's mask where the sim has it, between the last two ticks, and the beam in the
/// frame of its animation.
pub fn update(
    drive: Res<Drive>,
    fixed: Res<Time<Fixed>>,
    mut instances: Query<(&mut MaskPart, &mut Transform, &mut Visibility, &Children)>,
    parts: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<SubtractMaterial>>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let mask = drive.driver.mask;
    let kart = {
        let pose = interpolated(&drive, &fixed);
        Transform { translation: pose.translation, rotation: pose.rotation, scale: Vec3::ONE }
    };
    let t = fixed.overstep_fraction();

    for (mut part, mut transform, mut visibility, children) in &mut instances {
        let shown = mask.and_then(|m| Some((m, m.shown?))).filter(|(m, _)| m.good_guy == part.good_guy);
        let Some((mask, shown)) = shown else {
            *visibility = Visibility::Hidden;
            part.previous = None;
            part.current = None;
            continue;
        };

        // A new tick: what was current is previous.
        if part.tick != drive.tick || part.current.is_none() {
            let placement = Placement::new(&mask, &shown, if part.beam { &shown.beam } else { &shown.head });
            part.previous = Some(part.current.unwrap_or(placement));
            part.current = Some(placement);
            part.tick = drive.tick;
        }
        let (Some(before), Some(now)) = (part.previous, part.current) else { continue };
        let (a, b) = (before.transform(&kart), now.transform(&kart));
        transform.translation = a.translation.lerp(b.translation, t);
        transform.rotation = a.rotation.slerp(b.rotation, t);
        transform.scale = a.scale.lerp(b.scale, t);
        *visibility = if now.scale > 0.0 { Visibility::Inherited } else { Visibility::Hidden };

        if !part.beam {
            continue;
        }

        // The beam turns through its animation.
        let frame = shown.beam_frame as i16;
        if part.frame != frame {
            part.frame = frame;
            if let Some(pose) = animation_pose(&part.header, 0, frame) {
                let positions = mesh::model_positions(&part.header, &pose, [0x1000; 3]);
                for (handle, positions) in part.parts.iter().zip(positions) {
                    let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else { continue };
                    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                }
            }
        }

        // And fades out with the mask: in the colour, and in the alpha of the blended parts.
        if part.alpha != shown.beam_alpha {
            part.alpha = shown.beam_alpha;
            let fade = (1.0 - part.alpha as f32 / ALPHA_GONE).clamp(0.0, 1.0);
            let dim = |m: &mut StandardMaterial| {
                let masked = matches!(m.alpha_mode, AlphaMode::Mask(_));
                m.base_color = Color::linear_rgba(fade, fade, fade, if masked { 1.0 } else { fade });
            };
            for child in children {
                let Ok((standard, subtract)) = parts.get(*child) else { continue };
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
