//! Effects that face each view, drawn by the GPU. The CPU used to bake the corners of each sprite
//! from one camera's right and up, which is wrong for every other view (split screen). Now a mesh
//! gives what does not depend on the view, and a vertex shader puts the rest where the view that
//! draws it needs it:
//!
//! - quads (`billboard.wesl`): the centre of each sprite as its position and the corner's offset in
//!   the view's plane as [`ATTRIBUTE_CORNER`];
//! - streaks (`streak.wesl`): turned about their own axis to the eye ([`ATTRIBUTE_STREAK`],
//!   [`ATTRIBUTE_ALONG`]);
//! - whole models (`turned.wesl`, [`FacesView`]): the shield, the bursts, the camera-facing props;
//! - the skid marks' fade (`skid.rs`) and the wheels (`wheels.rs`) have their shaders by them.
//!
//! The materials all come from [`view_extension!`], and the shaders all end in `view_output.wesl`.

use bevy::asset::embedded_asset;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::MeshVertexAttribute;
use bevy::prelude::*;
use bevy::render::render_resource::VertexFormat;
use bevy::shader::load_shader_library;

/// The offset of a vertex from its sprite's centre, along the view's right (x) and up (y), in
/// world units. (A high id, as the docs of [`MeshVertexAttribute`] ask.)
pub const ATTRIBUTE_CORNER: MeshVertexAttribute =
    MeshVertexAttribute::new("Corner", 0x62_69_6c_6c, VertexFormat::Float32x2);

/// A vertex of a streak (`streak.wesl`): how far to its side it goes, in world units (x), and
/// which end of the streak it is on, 0 for the head and 1 for the tail (y).
pub const ATTRIBUTE_STREAK: MeshVertexAttribute =
    MeshVertexAttribute::new("Streak", 0x73_74_72_6b, VertexFormat::Float32x2);

/// From the head of a streak to its tail, in world units, the same for each vertex of it.
pub const ATTRIBUTE_ALONG: MeshVertexAttribute =
    MeshVertexAttribute::new("Along", 0x61_6c_6f_6e, VertexFormat::Float32x3);

/// Makes a [`MaterialExtension`](bevy::pbr::MaterialExtension) of which only the vertex stage is
/// ours (Bevy's fragment shader, alpha modes and depth bias stay), and the material of it.
///
/// ```ignore
/// view_extension!(
///     /// What the extension is.
///     Name => NameMaterial,
///     shader: "name.wesl",
///     subtract: false,
///     attributes: [Mesh::ATTRIBUTE_POSITION => 0, ...],
///     uniform: field: Type, // (optional) at binding 100
/// );
/// ```
///
/// `subtract` blends as `subtract.rs` does. The attributes are the vertex buffer, with the locations
/// the shader has them at. No camera here has a prepass, and the meshes cast no shadow.
macro_rules! view_extension {
    (
        $(#[$meta:meta])*
        $name:ident => $material:ident,
        shader: $shader:literal,
        subtract: $subtract:expr,
        attributes: [$($attribute:expr => $location:literal),+ $(,)?]
        $(, uniform: $field:ident: $ty:ty)? $(,)?
    ) => {
        $(#[$meta])*
        #[derive(
            bevy::asset::Asset,
            bevy::render::render_resource::AsBindGroup,
            bevy::reflect::Reflect,
            Debug,
            Clone,
            Default,
        )]
        pub struct $name {
            $(
                #[uniform(100)]
                #[reflect(ignore)]
                $field: $ty,
            )?
        }

        impl bevy::pbr::MaterialExtension for $name {
            fn vertex_shader() -> bevy::shader::ShaderRef {
                concat!(
                    "embedded://",
                    env!("CARGO_PKG_NAME"),
                    "/",
                    $shader
                )
                .into()
            }

            fn enable_prepass() -> bool {
                false
            }

            fn enable_shadows() -> bool {
                false
            }

            fn specialize(
                _pipeline: &bevy::pbr::MaterialExtensionPipeline,
                descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
                mesh_layout: &bevy::mesh::MeshVertexBufferLayoutRef,
                _key: bevy::pbr::MaterialExtensionKey<Self>,
            ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
                if $subtract {
                    crate::subtract::set_blend(descriptor);
                }
                let buffer = mesh_layout.0.get_layout(&[
                    $($attribute.at_shader_location($location)),+
                ])?;
                descriptor.vertex.buffers = vec![buffer];
                Ok(())
            }
        }

        pub type $material = bevy::pbr::ExtendedMaterial<StandardMaterial, $name>;
    };
}
pub(crate) use view_extension;

view_extension!(
    /// A quad that is turned to the view.
    Billboard => BillboardMaterial,
    shader: "billboard.wesl",
    subtract: false,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        Mesh::ATTRIBUTE_COLOR => 5,
        ATTRIBUTE_CORNER => 8,
    ],
);

view_extension!(
    /// [`Billboard`], blended as `subtract.rs` does.
    BillboardSubtract => BillboardSubtractMaterial,
    shader: "billboard.wesl",
    subtract: true,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        Mesh::ATTRIBUTE_COLOR => 5,
        ATTRIBUTE_CORNER => 8,
    ],
);

view_extension!(
    /// A streak that is turned about its own axis to face the view, instead of a quad that is turned
    /// to it: the sparks.
    Streak => StreakMaterial,
    shader: "streak.wesl",
    subtract: false,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_COLOR => 5,
        ATTRIBUTE_STREAK => 8,
        ATTRIBUTE_ALONG => 9,
    ],
);

view_extension!(
    /// A whole model that is turned to the view (`turned.wesl`): the entity's rotation is relative
    /// to the view's.
    Turned => TurnedMaterial,
    shader: "turned.wesl",
    subtract: false,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        Mesh::ATTRIBUTE_COLOR => 5,
    ],
);

view_extension!(
    /// [`Turned`], blended as `subtract.rs` does.
    TurnedSubtract => TurnedSubtractMaterial,
    shader: "turned.wesl",
    subtract: true,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        Mesh::ATTRIBUTE_COLOR => 5,
    ],
);

/// On the root of a model made by `spawn_parts`: its parts are drawn turned to each view
/// ([`turned`]), and its own rotation is relative to the view's (the identity for "faces it").
/// The parts must be its children.
#[derive(Component)]
pub struct FacesView;

/// Gives the parts of a [`FacesView`] model the materials that turn them. They are made as the
/// ordinary ones, and are swapped the frame they appear. (Their bounds are of the unturned model,
/// which is not where they are drawn: not culled.)
fn turned(
    mut commands: Commands,
    roots: Query<(), With<FacesView>>,
    standard: Query<(Entity, &MeshMaterial3d<StandardMaterial>, &ChildOf), Added<MeshMaterial3d<StandardMaterial>>>,
    subtract: Query<
        (Entity, &MeshMaterial3d<crate::subtract::SubtractMaterial>, &ChildOf),
        Added<MeshMaterial3d<crate::subtract::SubtractMaterial>>,
    >,
    standard_assets: ResMut<Assets<StandardMaterial>>,
    subtract_assets: ResMut<Assets<crate::subtract::SubtractMaterial>>,
    mut turned_assets: ResMut<Assets<TurnedMaterial>>,
    mut turned_subtract_assets: ResMut<Assets<TurnedSubtractMaterial>>,
) {
    for (entity, handle, parent) in &standard {
        if !roots.contains(parent.parent()) {
            continue;
        }
        let Some(base) = standard_assets.get(&handle.0).cloned() else {
            continue;
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((
                MeshMaterial3d(turned_assets.add(TurnedMaterial {
                    base,
                    extension: Turned {},
                })),
                NoFrustumCulling,
            ));
    }
    for (entity, handle, parent) in &subtract {
        if !roots.contains(parent.parent()) {
            continue;
        }
        let Some(base) = subtract_assets.get(&handle.0).map(|m| m.base.clone()) else {
            continue;
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<crate::subtract::SubtractMaterial>>()
            .insert((
                MeshMaterial3d(turned_subtract_assets.add(TurnedSubtractMaterial {
                    base,
                    extension: TurnedSubtract {},
                })),
                NoFrustumCulling,
            ));
    }
}

pub struct BillboardPlugin;

impl Plugin for BillboardPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "view_output.wesl");
        embedded_asset!(app, "billboard.wesl");
        embedded_asset!(app, "streak.wesl");
        embedded_asset!(app, "turned.wesl");
        app.add_plugins(MaterialPlugin::<BillboardMaterial>::default())
            .add_plugins(MaterialPlugin::<BillboardSubtractMaterial>::default())
            .add_plugins(MaterialPlugin::<StreakMaterial>::default())
            .add_plugins(MaterialPlugin::<TurnedMaterial>::default())
            .add_plugins(MaterialPlugin::<TurnedSubtractMaterial>::default())
            .add_systems(PostUpdate, turned.before(bevy::camera::visibility::VisibilitySystems::CheckVisibility));
    }
}
