//! The wheels of a kart as Bevy entities; the placement is `render::wheels`.

use crate::billboard::view_extension;
use crate::kart::{KartMotion, KartSlot};
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, MeshVertexAttribute, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, ShaderType, TextureDimension, TextureFormat, VertexFormat,
};
use loader::icons::Icon;
use loader::vram::Vram;
use render::mesh::SCALE;
use render::wheels::{
    DRIVER_INSTANCE_SCALE, SPRITE_INDEX, WHEEL_SIZE, WHEELS, radius, sprite_atlas, wheel_poses,
};

/// A wheel's axle (xyz) and radius in world units (w, 0 when the wheel is hidden), the same on each
/// of its vertices: `wheels.wesl` turns the square to the view.
const ATTRIBUTE_AXLE: MeshVertexAttribute =
    MeshVertexAttribute::new("WheelAxle", 0x77_68_61_78, VertexFormat::Float32x4);

/// To which side of the kart a wheel is (xyz), and whether it is a right-hand one (w: 1 or 0).
const ATTRIBUTE_SIDE: MeshVertexAttribute =
    MeshVertexAttribute::new("WheelSide", 0x77_68_73_64, VertexFormat::Float32x4);

/// What `wheels.wesl` needs to pick a picture: retail's table of them by the angle, and how many
/// the atlas has.
#[derive(ShaderType, Clone, Debug)]
pub struct WheelSprites {
    count: u32,
    push: f32,
    table: [UVec4; 33],
}

impl Default for WheelSprites {
    fn default() -> Self {
        Self::new(0)
    }
}

impl WheelSprites {
    fn new(count: usize) -> Self {
        let mut flat = [0u32; 132];
        for (slot, picture) in flat.iter_mut().zip(SPRITE_INDEX) {
            *slot = picture as u32;
        }
        WheelSprites {
            count: count as u32,
            push: NEAR_SIDE_PUSH,
            table: std::array::from_fn(|i| UVec4::from_slice(&flat[i * 4..i * 4 + 4])),
        }
    }
}

view_extension!(
    /// The wheels, turned to each view by the GPU.
    Wheel => WheelMaterial,
    shader: "wheels.wesl",
    subtract: false,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        ATTRIBUTE_AXLE => 8,
        ATTRIBUTE_SIDE => 9,
    ],
    uniform: sprites: WheelSprites,
);
pub struct WheelsPlugin;

impl Plugin for WheelsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "wheels.wesl");
        app.add_plugins(MaterialPlugin::<WheelMaterial>::default());
    }
}

/// One wheel of a kart: a mesh of a single square, turned to each view by the GPU. The entity sits at the wheel's centre, so that Bevy sorts the wheels among
/// themselves (and the smoke) by their depth, as the ordering table does in retail.
#[derive(Component)]
pub struct Wheels {
    /// Which wheel (`wheelLocal[wheel]`): front left, front right, rear left, rear right.
    wheel: usize,
    /// The kart entity, whose pose and [`KartMotion`] the wheels follow.
    kart: Entity,
    wheel_size: i32,
    /// The mesh of a wheel that is under water (`DrawTires`' secondary range): in the world's
    /// layer, behind the surface. The other one has it when it is not.
    under: bool,
}

impl Wheels {
    /// The kart they are the wheels of.
    pub fn kart(&self) -> Entity {
        self.kart
    }
}

/// The water line, in world units.
const WATERLINE: f32 = vehicle::force::WATERLINE_Y as f32 * SCALE;

/// Wheel squares are pushed this many radii towards the camera when they are on
/// the camera's side of the kart, so that the kart's body does not cut into them
/// (retail sorts the sprites among the body's polygons by depth instead).
const NEAR_SIDE_PUSH: f32 = 1.5;

pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WheelMaterial>,
    icons: &[Icon],
    vram: &Vram,
    kart: Entity,
    layer: usize,
) {
    let (atlas, uvs) = sprite_atlas(icons, vram);
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

    let image = images.add(image);
    let material = materials.add(WheelMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(image),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        },
        extension: Wheel {
            sprites: WheelSprites::new(uvs.len()),
        },
    });
    // A kart in the water draws the wheels that are under it in the world's layer, where the
    // surface tints them; the others stay in the kart's.
    for wheel in 0..WHEELS {
        for (under, layer) in [(false, layer), (true, 0)] {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4]);
            // Which corner of its picture a vertex is: the shader finds the picture's rows.
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                vec![[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            );
            mesh.insert_attribute(ATTRIBUTE_AXLE, vec![[0.0f32; 4]; 4]);
            mesh.insert_attribute(ATTRIBUTE_SIDE, vec![[0.0f32; 4]; 4]);
            // Corners are in UV order (top left, top right, bottom left, bottom right).
            mesh.insert_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]));
            commands.spawn((
                Wheels {
                    wheel,
                    kart,
                    wheel_size: WHEEL_SIZE,
                    under,
                },
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                RenderLayers::layer(layer),
                // The square moves with the view; the mesh's bounds are those of its first
                // frame.
                NoFrustumCulling,
            ));
        }
    }
}

/// Poses every wheel mesh, with the steering of its kart (a kart that stands still has none). The
/// view it is turned to is the GPU's business.
pub fn update(
    mut wheels: Query<(Entity, &Wheels, &Mesh3d, &mut Transform)>,
    karts: Query<(&Transform, Option<&KartMotion>, Option<&KartSlot>), Without<Wheels>>,
    sim: Option<Res<crate::props::Sim>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    for (entity, wheels, mesh, mut transform) in &mut wheels {
        let Ok((pose, motion, slot)) = karts.get(wheels.kart) else {
            // The kart is gone (another player left).
            commands.entity(entity).despawn();
            continue;
        };
        // The tires sit at the kart's squashed and stretched size.
        let (wheel_rotation, hazard_timer, scale) = motion
            .map_or((0, 0, [DRIVER_INSTANCE_SCALE; 3]), |m| {
                (m.wheel_rotation, m.hazard_timer, m.drawn_inst_scale())
            });
        let poses = wheel_poses(
            pose.translation,
            Mat3::from_quat(pose.rotation),
            scale,
            wheel_rotation,
            hazard_timer,
        );
        let in_water = slot
            .zip(sim.as_ref())
            .and_then(|(slot, sim)| sim.race.karts.get(slot.0))
            .is_some_and(|kart| kart.driver.split_line);
        let wheel = &poses[wheels.wheel];
        // The square is relative to the wheel's centre, which is what Bevy sorts it by.
        transform.translation = wheel.center;
        // A wheel is whole in one of its two meshes (the other has no size).
        let radius = if (in_water && wheel.center.y <= WATERLINE) != wheels.under {
            0.0
        } else {
            radius(wheels.wheel_size)
        };
        let axle = [wheel.axle.x, wheel.axle.y, wheel.axle.z, radius];
        let side = [
            wheel.side.x,
            wheel.side.y,
            wheel.side.z,
            wheel.right_hand as u32 as f32,
        ];
        crate::mesh_write::set_attributes(
            &mut meshes,
            &mesh.0,
            [
                (ATTRIBUTE_AXLE, vec![axle; 4].into()),
                (ATTRIBUTE_SIDE, vec![side; 4].into()),
            ],
        );
    }
}
