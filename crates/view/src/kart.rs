//! A kart on screen. The one being driven and the other players' are the same thing here: a
//! root entity with the model's parts and wheels, which [`KartMotion`] poses from whatever sim
//! drives it (`drive.rs` for ours, `remote.rs` for the others).

use bevy::prelude::*;
use loader::icons::Icon;
use loader::model::{Model, ModelHeader};
use loader::vram::Vram;
use render::mesh::{self, PARTS};
use vehicle::Driver;

use crate::drive::{Pose, animation_pose, pose_of};
use crate::subtract::SubtractMaterial;

/// What the drawing of a kart takes from its sim, pushed once per game tick.
#[derive(Component)]
pub struct KartMotion {
    previous: Pose,
    current: Pose,
    /// `Driver::wheelRotation`: the front wheels' steering.
    pub wheel_rotation: i16,
    /// The instance scale: the squash and stretch the wheels follow.
    pub inst_scale: [i32; 3],
    /// `Driver::animIndex` and `animFrame`: which pose of the model to show.
    animation: (u8, i16),
}

impl KartMotion {
    pub fn new(d: &Driver) -> KartMotion {
        let pose = pose_of(d);
        KartMotion {
            previous: pose,
            current: pose,
            wheel_rotation: 0,
            inst_scale: [0; 3],
            animation: (0, 0),
        }
        .pushed(d)
    }

    fn pushed(mut self, d: &Driver) -> KartMotion {
        self.wheel_rotation = d.wheel_rotation;
        self.inst_scale = [d.inst_scale_x, d.inst_scale_y, d.inst_scale_z].map(i32::from);
        self.animation = (d.anim_index, d.anim_frame);
        self
    }

    /// The kart after a tick.
    pub fn push(&mut self, d: &Driver) {
        let previous = self.current;
        *self = KartMotion {
            previous,
            current: pose_of(d),
            ..*self
        }
        .pushed(d);
    }

    /// The kart was put somewhere else: it does not travel there.
    pub fn reset(&mut self, d: &Driver) {
        *self = KartMotion::new(d);
    }

    /// Between the last two ticks, `t` of the way to the latest.
    pub fn pose(&self, t: f32) -> Pose {
        Pose {
            translation: self.previous.translation.lerp(self.current.translation, t),
            rotation: self.previous.rotation.slerp(self.current.rotation, t),
            scale: self.previous.scale.lerp(self.current.scale, t),
        }
    }
}

/// The kart model's faces, to pose as [`KartMotion`] says: steering plays a frame that matches
/// the wheel, a jump or a spin-out others.
#[derive(Component)]
pub struct KartRig {
    header: ModelHeader,
    parts: [Option<Handle<Mesh>>; PARTS],
    shown: Option<(u8, i16)>,
}

/// A kart of `model` at `placement`, in render layer `layer`. With a `motion` the sim moves it.
/// The model faces along +Z, like a driver's heading.
pub fn spawn(
    model: &Model,
    wheel_icons: &[Icon],
    vram: &Vram,
    placement: Transform,
    layer: usize,
    motion: Option<KartMotion>,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) -> Entity {
    let header = &model.headers[0];
    let frame = header
        .animations
        .first()
        .map(|a| &a.frames[a.frames.len() / 2])
        .or(header.static_frame.as_ref())
        .expect("racer model has a pose");
    let data = mesh::model_mesh(
        header,
        frame,
        vram,
        [render::wheels::DRIVER_INSTANCE_SCALE as i16; 3],
    );
    let base = crate::kart_material(data.atlas, images);

    let root = commands.spawn((placement, Visibility::default())).id();
    if !wheel_icons.is_empty() {
        crate::wheels::spawn(
            commands,
            images,
            meshes,
            materials,
            wheel_icons,
            vram,
            root,
            layer,
        );
    }
    let parts = crate::spawn_parts(
        data.parts,
        &base,
        Transform::default(),
        Some(root),
        commands,
        meshes,
        materials,
        subtract_materials,
        layer,
    );
    if let Some(motion) = motion {
        commands.entity(root).insert((
            motion,
            KartRig {
                header: header.clone(),
                parts,
                shown: None,
            },
        ));
    }
    root
}

/// Puts every kart where its sim has it, between the last two ticks.
pub fn show(fixed: Res<Time<Fixed>>, mut karts: Query<(&mut Transform, &KartMotion)>) {
    let t = fixed.overstep_fraction();
    for (mut transform, motion) in &mut karts {
        let pose = motion.pose(t);
        transform.translation = pose.translation;
        transform.rotation = pose.rotation;
        transform.scale = pose.scale;
    }
}

/// Poses the models as the sims' animations say, when that changes.
pub fn animate(mut karts: Query<(&KartMotion, &mut KartRig)>, mut meshes: ResMut<Assets<Mesh>>) {
    for (motion, mut rig) in &mut karts {
        if rig.shown == Some(motion.animation) {
            continue;
        }
        let Some(frame) = animation_pose(&rig.header, motion.animation.0, motion.animation.1)
        else {
            continue;
        };
        rig.shown = Some(motion.animation);
        let positions = mesh::model_positions(
            &rig.header,
            &frame,
            [render::wheels::DRIVER_INSTANCE_SCALE as i16; 3],
        );
        for (handle, positions) in rig.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else {
                continue;
            };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}
