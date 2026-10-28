//! The tire marks on the ground as Bevy meshes, of every kart of the race. Where the tires left
//! them is `vehicle::skid`; this lays the segments out and fades them as
//! `VehGroundSkids_Main` does.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use loader::icons::Icon;
use loader::vram::Vram;
use render::mesh::SCALE;
use vehicle::skid::{fade_anchor, intensity, segments};

use crate::exhaust::icon_image;
use crate::kart::{Attached, KartSlot};
use crate::subtract::{self, SubtractMaterial};

/// `VEH_GROUND_SKIDS_ICON_TIREMARK`: the icon's global index.
pub const TIREMARK_ICON: i32 = 0x2f;

/// Most segments one mesh draws: four tires, a segment per tick of the history.
const MAX_QUADS: usize = 32;

/// Marks farther than this from the camera on any axis are not drawn
/// (`VEH_GROUND_SKIDS_CULL_ABS_MAX`, in the GTE's quarter units).
const CULL_DISTANCE: f32 = 0x1771 as f32 / 4.0;

/// By-eye tuning of how strongly the marks blend, on top of retail's colour
/// (1.0 is the raw PS1 math; Bevy blends in linear light, so it reads lighter
/// than the original). Raise the dark gain to make the marks blacker; the light
/// marks are retail's quarter of the texel at 0.25.
const DARK_GAIN: f32 = 3.0;
const LIGHT_GAIN: f32 = 0.25;

/// Lifts the marks off the ground, in game units. The kart bounces by up to 3 units
/// (`screen_offset_y`) and the marks bounce with it, so they would dip under the ground.
const LIFT: f32 = 4.0;

/// A mesh of the marks of one kart, of one kind.
#[derive(Component)]
pub struct Marks {
    /// Terrain that forces marks makes light ones (the tpage's quarter-add blend), others dark.
    light: bool,
}

/// Makes the pair of meshes that draw the marks of `kart`, with the tire mark's picture. The
/// marks are laid out by `update`.
pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    icon: &Icon,
    vram: &Vram,
    kart: Entity,
) {
    let (image, _, _) = icon_image(icon, vram);
    let image = images.add(image);
    let base = |alpha_mode| StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(image.clone()),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        alpha_mode,
        // Retail sorts the marks among the track's polygons by an ordering table bias.
        depth_bias: 400.0,
        ..default()
    };
    let dark = subtract_materials.add(SubtractMaterial {
        base: base(AlphaMode::Blend),
        extension: subtract::Subtract {},
    });
    let light = materials.add(base(AlphaMode::Add));
    let placed = (Attached(kart), Transform::default(), NoFrustumCulling);
    commands.spawn((
        Marks { light: false },
        Mesh3d(new_mesh(meshes)),
        MeshMaterial3d(dark),
        placed.clone(),
    ));
    commands.spawn((
        Marks { light: true },
        Mesh3d(new_mesh(meshes)),
        MeshMaterial3d(light),
        placed,
    ));
}

/// An empty mesh with room for [`MAX_QUADS`] segments.
fn new_mesh(meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * MAX_QUADS]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4 * MAX_QUADS]);
    // Across the mark first, then along it, as the layout's corners go.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        (0..MAX_QUADS)
            .flat_map(|_| [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]])
            .collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32(
        (0..MAX_QUADS as u32)
            .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
            .collect(),
    ));
    meshes.add(mesh)
}

/// Lays the karts' mark segments out on the ground, as bright as their age and distance say.
pub fn update(
    marks: Query<(&Marks, &Attached, &Mesh3d)>,
    karts: Query<&KartSlot>,
    cameras: Query<
        &Transform,
        (
            With<Camera3d>,
            Without<Marks>,
            Without<crate::OverlayCamera>,
            Without<crate::sky::SkyCamera>,
        ),
    >,
    sim: Res<crate::props::Sim>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // The kart is drawn between its last two ticks; the marks' heads go with it.
    let behind = 1.0 - fixed.overstep_fraction();
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let eye = camera.translation / SCALE;
    let forward = camera.forward().as_vec3();
    let point = |p: [i16; 3]| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);

    // Each kart's marks, as bright as the distance of its newest mark says, and not drawn at all
    // when that is too far on any axis.
    let shown: Vec<_> = sim
        .race
        .karts
        .iter()
        .map(|kart| {
            let d = &kart.driver;
            let anchor = point(fade_anchor(d)) - eye;
            let bright = (anchor.abs().max_element() < CULL_DISTANCE)
                .then(|| intensity(anchor.dot(forward) as i32))
                .flatten();
            let segments = bright.map(|_| segments(d)).unwrap_or_default();
            (bright, segments)
        })
        .collect();
    let nothing = (None, Vec::new());

    for (kind, attached, mesh) in &marks {
        let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
            continue;
        };
        let slot = karts.get(attached.0).map_or(usize::MAX, |slot| slot.0);
        let (bright, segments) = shown.get(slot).unwrap_or(&nothing);
        let mut positions = vec![[0.0f32; 3]; 4 * MAX_QUADS];
        let mut colors = vec![[0.0f32; 4]; 4 * MAX_QUADS];
        let wanted = segments
            .iter()
            .filter(|s| s.light == kind.light)
            .take(MAX_QUADS);
        for (n, s) in wanted.enumerate() {
            let value = bright.unwrap_or(0);
            // A segment is as bright as its newer end, fading to its older one; where
            // that is gone, so is the segment.
            let (near, far) = (
                value >> s.age,
                if s.age == 0 {
                    value
                } else {
                    value >> (s.age - 1)
                },
            );
            if far == 0 {
                continue;
            }
            // A strip's corners are the newer end's two, then the older end's.
            let shades = [near, near, far, far].map(|v| {
                // The primitive's colour scales the texture.
                let gain = v as f32 / 128.0 * if s.light { LIGHT_GAIN } else { DARK_GAIN };
                [gain, gain, gain, 1.0]
            });
            let mut corners = s.corners.map(point);
            // The kart is drawn `behind` of a tick short of the newest tick, which is where the newest
            // strip and the head ahead of it are. They slide back with it, and the segment before
            // them, which ends at that strip, stretches to follow. Retail does not interpolate.
            if s.age <= 1 {
                let tick = |age| segments.iter().find(|o| o.tire == s.tire && o.age == age);
                if let (Some(newest), Some(older)) = (tick(0), tick(1)) {
                    let middle = |c: [[i16; 3]; 4]| (point(c[0]) + point(c[1])) / 2.0;
                    let slide = (middle(older.corners) - middle(newest.corners)) * behind;
                    for corner in &mut corners[if s.age == 0 { 0 } else { 2 }..] {
                        *corner += slide;
                    }
                }
            }
            for (k, corner) in corners.into_iter().enumerate() {
                positions[n * 4 + k] = ((corner + Vec3::Y * LIFT) * SCALE).to_array();
                colors[n * 4 + k] = shades[k];
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}
