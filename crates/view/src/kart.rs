//! A kart on screen. The one being driven and the other players' are the same thing here, and
//! [`KartBuilder::spawn`] is the one place that makes either: a root entity with the model's parts
//! and wheels, which [`KartMotion`] poses from the sim's kart of its [`KartSlot`], and the
//! effects that go with it (flames, shadow, tire marks, exhaust, sparks, masks). Those are
//! entities of their own that follow the kart they are [`Attached`] to, and every system that
//! draws one reads the kart's slot from there. Where a kart differs is only its render layer.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use loader::icons::Icon;
use loader::model::{Model, ModelHeader};
use loader::vram::Vram;
use render::mesh::{self, PARTS, SCALE};
use vehicle::shadow::{GroundShadow, ground_shadow};

use crate::drive::{Kart, Pose, animation_pose, pose_of};
use crate::subtract::SubtractMaterial;

/// The slot in the race of the kart a root entity draws. (Ours follows `Sim::own`.)
#[derive(Component, Clone, Copy)]
pub struct KartSlot(pub usize);

/// Ties an effect to the kart entity it belongs to: it is despawned with it (by [`reap`]).
#[derive(Component, Clone, Copy)]
pub struct Attached(pub Entity);

/// Despawns the effects of karts that are gone.
pub fn reap(
    attached: Query<(Entity, &Attached)>,
    karts: Query<(), With<KartSlot>>,
    mut commands: Commands,
) {
    for (entity, Attached(kart)) in &attached {
        if !karts.contains(*kart) {
            commands.entity(entity).try_despawn();
        }
    }
}

/// What the drawing of a kart takes from its sim, pushed once per game tick.
#[derive(Component, Clone)]
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
    /// The ground shadow at the last two ticks.
    shadow_previous: Option<GroundShadow>,
    shadow_current: Option<GroundShadow>,
}

/// The furthest apart two ticks' frames of an animation can be and still be blended: an
/// animation plays on a frame or two a tick, so more is a jump (a new animation, a wrap round
/// to the first frame) that must not be blended through.
const MAX_BLEND_FRAMES: i16 = 4;

impl KartMotion {
    pub fn new(kart: &race::Kart) -> KartMotion {
        let pose = pose_of(&kart.driver);
        KartMotion {
            previous: pose,
            current: pose,
            wheel_rotation: 0,
            inst_scale: [0; 3],
            animation: (0, 0),
            previous_animation: (0, 0),
            shadow_previous: None,
            shadow_current: None,
        }
        .pushed(kart)
        .settled()
    }

    fn pushed(mut self, kart: &race::Kart) -> KartMotion {
        let d = &kart.driver;
        self.shadow_current = ground_shadow(d, &mut kart.coll.gte.clone());
        self.wheel_rotation = d.wheel_rotation;
        self.inst_scale = [d.inst_scale_x, d.inst_scale_y, d.inst_scale_z].map(i32::from);
        self.animation = (d.anim_index, d.anim_frame);
        self
    }

    /// No animation or shadow to blend from.
    fn settled(mut self) -> KartMotion {
        self.previous_animation = self.animation;
        self.shadow_previous = self.shadow_current;
        self
    }

    /// The kart after a tick.
    pub fn push(&mut self, kart: &race::Kart) {
        let previous = self.current;
        *self = KartMotion {
            previous,
            current: pose_of(&kart.driver),
            previous_animation: self.animation,
            shadow_previous: self.shadow_current,
            ..*self
        }
        .pushed(kart);
    }

    /// The kart was put somewhere else: it does not travel there.
    pub fn reset(&mut self, kart: &race::Kart) {
        *self = KartMotion::new(kart);
    }

    /// Nothing moves between ticks now (the pause): stay where the last one put the kart.
    pub fn freeze(&mut self) {
        self.previous = self.current;
        *self = self.clone().settled();
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
    /// The ground shadow between the last two ticks (`t` of the way to the latest): its centre,
    /// orientation and the height that scales it.
    pub fn shadow(&self, t: f32) -> Option<(Vec3, Mat3, f32)> {
        let now = self.shadow_current?;
        let before = self.shadow_previous.unwrap_or(now);
        let place = |s: &GroundShadow| Vec3::new(s.x as f32, s.y as f32, s.z as f32) * SCALE;
        let orient = |s: &GroundShadow| {
            let column = |c: usize| {
                Vec3::new(
                    s.axes.m[0][c] as f32,
                    s.axes.m[1][c] as f32,
                    s.axes.m[2][c] as f32,
                ) / 4096.0
            };
            let (up, forward) = (column(1).normalize_or_zero(), column(2).normalize_or_zero());
            Quat::from_mat3(&Mat3::from_cols(up.cross(forward), up, forward))
        };
        Some((
            place(&before).lerp(place(&now), t),
            Mat3::from_quat(orient(&before).slerp(orient(&now), t)),
            before.height as f32 + (now.height - before.height) as f32 * t,
        ))
    }

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

/// The model of a kart at `placement`, in render layer `layer`, and its wheels. With a `motion`
/// the sim moves it. The model faces along +Z, like a driver's heading.
fn spawn_body(
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

/// Everything that makes a kart: the assets of its pieces and the means to spawn them.
#[derive(SystemParam)]
pub struct KartBuilder<'w, 's> {
    pub commands: Commands<'w, 's>,
    images: ResMut<'w, Assets<Image>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    subtract_materials: ResMut<'w, Assets<SubtractMaterial>>,
    textures: Res<'w, crate::Textures>,
    shadow_icons: Res<'w, crate::ShadowIcons>,
    smoke_icon: Res<'w, crate::SmokeIcon>,
    tiremark_icon: Res<'w, crate::TiremarkIcon>,
    flame_models: Res<'w, crate::FlameModels>,
    masks: Res<'w, crate::mask::MaskModels>,
}

impl KartBuilder<'_, '_> {
    /// A kart of `model` at `placement`. With a `sim` (the slot it draws and that slot's kart) it
    /// is driven, and has its effects; without, it is the model alone. Ours and the others' go
    /// through here alike. `ours` is the one difference: our kart is the [`Kart`], and so it
    /// is drawn over the track (in [`crate::KART_LAYER`]), while the others' are in the world's
    /// layer, where the walls hide them.
    pub fn spawn(
        &mut self,
        model: &Model,
        wheel_icons: &[Icon],
        placement: Transform,
        ours: bool,
        sim: Option<(usize, &race::Kart)>,
    ) -> Entity {
        let layer = if ours { crate::KART_LAYER } else { 0 };
        let vram = &self.textures.vram;
        let kart = spawn_body(
            model,
            wheel_icons,
            vram,
            placement,
            layer,
            sim.map(|(_, kart)| KartMotion::new(kart)),
            &mut self.commands,
            &mut self.images,
            &mut self.meshes,
            &mut self.materials,
            &mut self.subtract_materials,
        );
        if ours {
            self.commands.entity(kart).insert(Kart);
        }
        let Some((slot, _)) = sim else {
            return kart;
        };
        self.commands.entity(kart).insert(KartSlot(slot));
        crate::flame::spawn(
            &mut self.commands,
            &mut self.images,
            &mut self.meshes,
            &mut self.materials,
            &mut self.subtract_materials,
            &self.flame_models.0,
            vram,
            kart,
            layer,
        );
        if let Some(icons) = &self.shadow_icons.0 {
            crate::shadow::spawn(
                &mut self.commands,
                &mut self.images,
                &mut self.meshes,
                &mut self.subtract_materials,
                [&icons[0], &icons[1]],
                vram,
                kart,
            );
        }
        if let Some(icon) = &self.tiremark_icon.0 {
            crate::skid::spawn(
                &mut self.commands,
                &mut self.images,
                &mut self.meshes,
                &mut self.materials,
                &mut self.subtract_materials,
                icon,
                vram,
                kart,
            );
        }
        if let Some(icon) = &self.smoke_icon.0 {
            crate::exhaust::spawn(
                &mut self.commands,
                &mut self.images,
                &mut self.meshes,
                &mut self.materials,
                &mut self.subtract_materials,
                icon,
                vram,
                kart,
                layer,
            );
        }
        if let Some(masks) = &self.masks.0 {
            crate::mask::spawn(
                masks,
                vram,
                kart,
                layer,
                &mut self.commands,
                &mut self.images,
                &mut self.meshes,
                &mut self.materials,
                &mut self.subtract_materials,
            );
        }
        kart
    }
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
