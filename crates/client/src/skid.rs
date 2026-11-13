//! The tire marks on the ground as Bevy meshes, of every kart of the race. Where the tires left
//! them is `vehicle::skid`; this lays the segments out and fades them as
//! `VehGroundSkids_Main` does.

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, MeshVertexAttribute, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{ShaderType, VertexFormat};
use loader::icons::Icon;
use loader::vram::Vram;
use render::mesh::SCALE;
use vehicle::skid::{FULL_INTENSITY_DEPTH, fade_anchor, intensity, segments};

use crate::billboard::view_extension;
use crate::exhaust::icon_image;
use crate::kart::{Attached, KartSlot};

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

/// The kart's fade anchor, which each vertex of its marks carries (in world units): the view's
/// distance to it sets how bright the marks are (`skid.wesl`).
const ATTRIBUTE_ANCHOR: MeshVertexAttribute =
    MeshVertexAttribute::new("SkidAnchor", 0x73_6b_69_64, VertexFormat::Float32x3);

/// What `skid.wesl` needs of retail's fade: the distances, and `vehicle::skid::intensity` as a
/// table, the brightness at the full-intensity depth first and then past it for each count of
/// bits of the depth.
#[derive(ShaderType, Clone, Debug)]
pub struct SkidFade {
    scale: f32,
    cull: f32,
    near: i32,
    table: [UVec4; 9],
}

impl Default for SkidFade {
    fn default() -> Self {
        let mut flat = [0u32; 36];
        flat[0] = intensity(FULL_INTENSITY_DEPTH - 1).unwrap_or(0) as u32;
        for bits in 0..=31u32 {
            let past = if bits == 0 { 0 } else { 1i32 << (bits - 1) };
            flat[bits as usize + 1] = intensity(FULL_INTENSITY_DEPTH + past).unwrap_or(0) as u32;
        }
        SkidFade {
            scale: SCALE,
            cull: CULL_DISTANCE,
            near: FULL_INTENSITY_DEPTH,
            table: std::array::from_fn(|i| UVec4::from_slice(&flat[i * 4..i * 4 + 4])),
        }
    }
}

view_extension!(
    /// The skid marks, faded by the view that draws them: the vertex buffer is position, uv, color
    /// (how many times its brightness is halved, and the gain) and the anchor.
    Skid => SkidMaterial,
    shader: "skid.wesl",
    subtract: false,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        Mesh::ATTRIBUTE_COLOR => 5,
        ATTRIBUTE_ANCHOR => 8,
    ],
    uniform: fade: SkidFade,
);

view_extension!(
    /// [`Skid`], blended as `subtract.rs` does.
    SkidSubtract => SkidSubtractMaterial,
    shader: "skid.wesl",
    subtract: true,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        Mesh::ATTRIBUTE_COLOR => 5,
        ATTRIBUTE_ANCHOR => 8,
    ],
    uniform: fade: SkidFade,
);

pub struct SkidPlugin;

impl Plugin for SkidPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "skid.wesl");
        app.add_plugins(MaterialPlugin::<SkidMaterial>::default())
            .add_plugins(MaterialPlugin::<SkidSubtractMaterial>::default());
    }
}

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
    materials: &mut Assets<SkidMaterial>,
    subtract_materials: &mut Assets<SkidSubtractMaterial>,
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
    let dark = subtract_materials.add(SkidSubtractMaterial {
        base: base(AlphaMode::Blend),
        extension: SkidSubtract::default(),
    });
    let light = materials.add(SkidMaterial {
        base: base(AlphaMode::Add),
        extension: Skid::default(),
    });
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
    mesh.insert_attribute(ATTRIBUTE_ANCHOR, vec![[0.0f32; 3]; 4 * MAX_QUADS]);
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

/// Lays the karts' mark segments out on the ground. How bright they are is for the vertex shader:
/// each vertex gets its age's share and the kart's fade anchor, and each view fades by its own
/// camera.
pub fn update(
    marks: Query<(&Marks, &Attached, &Mesh3d)>,
    karts: Query<&KartSlot>,
    sim: Res<crate::props::Sim>,
    fixed: Res<Time<Fixed>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // The kart is drawn between its last two ticks; the marks' heads go with it.
    let behind = 1.0 - fixed.overstep_fraction();
    let point = |p: [i16; 3]| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);

    let shown: Vec<_> = sim
        .race
        .karts
        .iter()
        .map(|kart| {
            let d = &kart.driver;
            (point(fade_anchor(d)) * SCALE, segments(d))
        })
        .collect();
    let nothing = (Vec3::ZERO, Vec::new());

    for (kind, attached, mesh) in &marks {
        let slot = karts.get(attached.0).map_or(usize::MAX, |slot| slot.0);
        let (anchor, segments) = shown.get(slot).unwrap_or(&nothing);
        let mut positions = vec![[0.0f32; 3]; 4 * MAX_QUADS];
        let mut colors = vec![[0.0f32; 4]; 4 * MAX_QUADS];
        let mut anchors = vec![[0.0f32; 3]; 4 * MAX_QUADS];
        let gain = if kind.light { LIGHT_GAIN } else { DARK_GAIN };
        let wanted = segments
            .iter()
            .filter(|s| s.light == kind.light)
            .take(MAX_QUADS);
        for (n, s) in wanted.enumerate() {
            // A segment is as bright as its newer end, fading to its older one: the brightness is
            // halved `age` times at the newer end and one less at the older (the first segments of
            // a run have the same at both). Where that is gone, so is the segment.
            let (near, far) = (s.age as f32, s.age.saturating_sub(1) as f32);
            // A strip's corners are the newer end's two, then the older end's.
            let shifts = [near, near, far, far];
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
                colors[n * 4 + k] = [shifts[k], gain, 0.0, 1.0];
                anchors[n * 4 + k] = anchor.to_array();
            }
        }
        crate::mesh_write::set_attributes(
            &mut meshes,
            &mesh.0,
            [
                (Mesh::ATTRIBUTE_POSITION, positions.into()),
                (Mesh::ATTRIBUTE_COLOR, colors.into()),
                (ATTRIBUTE_ANCHOR, anchors.into()),
            ],
        );
    }
}
