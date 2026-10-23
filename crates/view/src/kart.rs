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
    /// The same one tick earlier, to blend from.
    previous_animation: (u8, i16),
}

/// The furthest apart two ticks' frames of an animation can be and still be blended: an
/// animation plays on a frame or two a tick, so more is a jump (a new animation, a wrap round
/// to the first frame) that must not be blended through.
const MAX_BLEND_FRAMES: i16 = 4;

impl KartMotion {
    pub fn new(d: &Driver) -> KartMotion {
        let pose = pose_of(d);
        KartMotion {
            previous: pose,
            current: pose,
            wheel_rotation: 0,
            inst_scale: [0; 3],
            animation: (0, 0),
            previous_animation: (0, 0),
        }
        .pushed(d)
        .settled()
    }

    fn pushed(mut self, d: &Driver) -> KartMotion {
        self.wheel_rotation = d.wheel_rotation;
        self.inst_scale = [d.inst_scale_x, d.inst_scale_y, d.inst_scale_z].map(i32::from);
        self.animation = (d.anim_index, d.anim_frame);
        self
    }

    /// No animation to blend from.
    fn settled(mut self) -> KartMotion {
        self.previous_animation = self.animation;
        self
    }

    /// The kart after a tick.
    pub fn push(&mut self, d: &Driver) {
        let previous = self.current;
        *self = KartMotion {
            previous,
            current: pose_of(d),
            previous_animation: self.animation,
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

impl KartMotion {
    /// The frames to show and how far from the first to the second, `t` of the way through the
    /// tick. Frames of different animations, or far apart, are not blended: the latest shows.
    fn blend(&self, t: f32) -> ((u8, i16), (u8, i16), f32) {
        let (from, to) = (self.previous_animation, self.animation);
        if from.0 == to.0 && (to.1 - from.1).abs() <= MAX_BLEND_FRAMES && from != to {
            (from, to, t)
        } else {
            (to, to, 0.0)
        }
    }
}

/// The kart model's faces, to pose as [`KartMotion`] says: steering plays a frame that matches
/// the wheel, a jump or a spin-out others.
#[derive(Component)]
pub struct KartRig {
    header: ModelHeader,
    parts: [Option<Handle<Mesh>>; PARTS],
    /// The blend on the mesh: two frames and the bits of how far between them.
    shown: Option<((u8, i16), (u8, i16), u32)>,
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

/// Poses the models as the sims' animations say: the frame of the last tick and the one before
/// it, blended by how far the render is through the tick, so that the 30 Hz animation moves
/// smoothly at any frame rate.
pub fn animate(
    fixed: Res<Time<Fixed>>,
    mut karts: Query<(&KartMotion, &mut KartRig)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let t = fixed.overstep_fraction();
    for (motion, mut rig) in &mut karts {
        let (from, to, t) = motion.blend(t);
        let key = (from, to, t.to_bits());
        if rig.shown == Some(key) {
            continue;
        }
        let pose = |frame: (u8, i16)| {
            animation_pose(&rig.header, frame.0, frame.1).map(|frame| {
                mesh::model_positions(
                    &rig.header,
                    &frame,
                    [render::wheels::DRIVER_INSTANCE_SCALE as i16; 3],
                )
            })
        };
        let Some(mut positions) = pose(to) else {
            continue;
        };
        if from != to
            && let Some(before) = pose(from)
        {
            for (now, before) in positions.iter_mut().zip(&before) {
                for (now, before) in now.iter_mut().zip(before) {
                    *now = std::array::from_fn(|a| before[a] + (now[a] - before[a]) * t);
                }
            }
        }
        rig.shown = Some(key);
        for (handle, positions) in rig.parts.iter().zip(positions) {
            let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else {
                continue;
            };
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}
