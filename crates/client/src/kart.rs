//! A kart on screen. The one being driven and the other players' are the same thing here, and
//! [`KartBuilder::spawn`] is the one place that makes either: a root entity with the model's parts
//! and wheels, which [`KartMotion`] poses from the sim's kart of its [`KartSlot`], and the
//! effects that go with it (flames, shadow, tire marks, exhaust, sparks, masks). Those are
//! entities of their own that follow the kart they are [`Attached`] to, and every system that
//! draws one reads the kart's slot from there. Where a kart differs is only its render layer.

use bevy::ecs::system::SystemParam;
use std::time::Duration;

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

/// How far a kart is drawn from where the sim has it, and the turn of it: what a rollback moved
/// the kart by, which fades away in the ticks after instead of showing as a jump.
#[derive(Clone, Copy)]
struct Offset {
    translation: Vec3,
    rotation: Quat,
}

impl Offset {
    const NONE: Offset = Offset {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };

    /// The offset a tick later, `ease` of it being left.
    fn faded(self, ease: f32) -> Offset {
        Offset {
            translation: self.translation * ease,
            rotation: Quat::IDENTITY.slerp(self.rotation, ease),
        }
    }
}

/// How the offset of a rollback fades (`/net smoothing`): the time until a tenth of it is left.
/// It is for the drawing only, and every client sets its own.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub enum Smoothing {
    /// Follows the round trip to the kart's player: the later their pads, the bigger the
    /// correction, and the longer it takes to fade.
    #[default]
    Ping,
    /// Follows how many ticks the rollbacks of the kart replayed lately, which is how late its
    /// pads really are (the tick and the clocks count, not only the network). The rollback does
    /// not say whose pad it was, so every remote kart gets the same figure from it.
    Replay,
    Fixed(Duration),
}

impl std::fmt::Display for Smoothing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Smoothing::Ping => write!(f, "ping"),
            Smoothing::Replay => write!(f, "replay"),
            Smoothing::Fixed(d) => write!(f, "{} ms", d.as_millis()),
        }
    }
}

/// How the fade by ping follows the round trip: that many round trips, within the bounds, and
/// what a player with no ping yet gets.
const PING_ROUND_TRIPS: f32 = 1.5;
const PING_MIN: Duration = Duration::from_millis(100);
const PING_MAX: Duration = Duration::from_millis(330);
const PING_UNKNOWN: Duration = Duration::from_millis(200);

/// The fade by replays: the ticks replayed on average, and one more, within the bounds. The
/// average moves this much towards each new rollback, and starts at `REPLAY_START` ticks.
const REPLAY_MIN: Duration = Duration::from_millis(66);
const REPLAY_MAX: Duration = Duration::from_millis(330);
const REPLAY_WEIGHT: f32 = 0.3;
const REPLAY_START: f32 = 2.0;

/// What is left of an offset when its fade is over.
const EASE_LEFT: f32 = 0.1;

impl Smoothing {
    /// How much of an offset is left after a tick, for a kart whose player is `ping` away and
    /// whose rollbacks replayed `replayed` ticks on average. Under a tick of fade it is gone in
    /// one: the old jump, spread over the tick.
    pub fn ease(self, ping: Option<Duration>, replayed: f32) -> f32 {
        let tick = Duration::from_secs_f64(1.0 / crate::GAME_TICK_HZ);
        let duration = match self {
            Smoothing::Fixed(d) => d,
            Smoothing::Ping => ping.map_or(PING_UNKNOWN, |rtt| {
                rtt.mul_f32(PING_ROUND_TRIPS).clamp(PING_MIN, PING_MAX)
            }),
            Smoothing::Replay => tick.mul_f32(replayed + 1.0).clamp(REPLAY_MIN, REPLAY_MAX),
        };
        let ticks = duration.as_secs_f32() * crate::GAME_TICK_HZ as f32;
        if ticks < 1.0 {
            0.0
        } else {
            EASE_LEFT.powf(1.0 / ticks)
        }
    }
}

/// A rollback moves a kart at least this far (render units) before it is taken for a jump of
/// the kart itself (a mask grab, a respawn) and not a late pad; the more ticks it replays and the
/// faster the kart goes, the further a late pad can move it.
const EASE_MIN_DISTANCE: f32 = 4.0;
const EASE_TRAVEL_FACTOR: f32 = 3.0;

/// What the drawing of a kart takes from its sim, pushed once per game tick.
#[derive(Component, Clone)]
pub struct KartMotion {
    previous: Pose,
    current: Pose,
    /// What was added to each of the two poses to draw them.
    previous_offset: Offset,
    offset: Offset,
    /// How much of the offset is left after a tick (see [`Smoothing::ease`]).
    ease: f32,
    /// The ticks that the rollbacks of the kart replayed, on average.
    replayed_average: f32,
    /// `Driver::wheelRotation`: the front wheels' steering.
    pub wheel_rotation: i16,
    /// `Driver::hazardTimer`: the wheels wobble with it.
    pub hazard_timer: i16,
    /// The instance scale: the squash and stretch the wheels follow.
    pub inst_scale: [i32; 3],
    /// `Driver::animIndex` and `animFrame`: which pose of the model to show.
    animation: (u8, i16),
    /// The same one tick earlier, to blend from.
    previous_animation: (u8, i16),
    /// The ground shadow at the last two ticks.
    shadow_previous: Option<GroundShadow>,
    shadow_current: Option<GroundShadow>,
    /// The kart is not drawn: a plant has eaten it (`HIDE_MODEL`).
    hidden: bool,
    /// The kart is burnt (`Driver::burnTimer`): its body and driver are black.
    burnt: bool,
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
            previous_offset: Offset::NONE,
            offset: Offset::NONE,
            ease: 0.0,
            replayed_average: REPLAY_START,
            wheel_rotation: 0,
            hazard_timer: 0,
            inst_scale: [0; 3],
            animation: (0, 0),
            previous_animation: (0, 0),
            shadow_previous: None,
            shadow_current: None,
            hidden: false,
            burnt: false,
        }
        .pushed(kart)
        .settled()
    }

    fn pushed(mut self, kart: &race::Kart) -> KartMotion {
        let d = &kart.driver;
        self.shadow_current = ground_shadow(d, &mut kart.coll.gte.clone());
        self.wheel_rotation = d.wheel_rotation;
        self.hazard_timer = d.hazard_timer;
        self.inst_scale = [d.inst_scale_x, d.inst_scale_y, d.inst_scale_z].map(i32::from);
        self.animation = (d.anim_index, d.anim_frame);
        self.hidden = d.phys_state == vehicle::PhysState::PlantEaten;
        self.burnt = d.burn_timer != 0;
        self
    }

    /// The kart is not drawn (a plant has eaten it).
    pub fn is_hidden(&self) -> bool {
        self.hidden
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
            previous_offset: self.offset,
            offset: self.offset.faded(self.ease),
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
        self.previous_offset = self.offset;
        *self = self.clone().settled();
    }

    /// The ticks that the rollbacks of the kart replayed, on average.
    pub fn replayed_average(&self) -> f32 {
        self.replayed_average
    }

    /// How the offsets of the next ticks fade (see [`Smoothing::ease`]).
    pub fn set_ease(&mut self, ease: f32) {
        self.ease = ease;
    }

    /// A rollback of `replayed` ticks put the kart right: it was at `predicted` (the pose of the
    /// tick the sim was at) and the sim has it elsewhere now, as `kart` says. The drawing stays
    /// where it was and eases to the new place over the next ticks, unless the kart was moved too
    /// far for that.
    pub fn absorb(&mut self, predicted: &Pose, kart: &race::Kart, replayed: u32) {
        self.replayed_average += (replayed as f32 - self.replayed_average) * REPLAY_WEIGHT;
        let now = pose_of(&kart.driver);
        let per_tick = self.current.translation.distance(self.previous.translation);
        let furthest = EASE_MIN_DISTANCE.max(EASE_TRAVEL_FACTOR * per_tick * replayed as f32);
        if predicted.translation.distance(now.translation) > furthest {
            let average = self.replayed_average;
            self.reset(kart);
            self.replayed_average = average;
            return;
        }
        self.offset = Offset {
            translation: self.offset.translation + predicted.translation - now.translation,
            rotation: self.offset.rotation * predicted.rotation * now.rotation.inverse(),
        };
    }

    /// Between the last two ticks, `t` of the way to the latest.
    pub fn pose(&self, t: f32) -> Pose {
        let (from, to) = (&self.previous_offset, &self.offset);
        Pose {
            translation: (self.previous.translation + from.translation)
                .lerp(self.current.translation + to.translation, t),
            rotation: (from.rotation * self.previous.rotation)
                .slerp(to.rotation * self.current.rotation, t),
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
        // (The ground does not move with a rollback's offset, but where the kart is drawn does.)
        let eased = self
            .previous_offset
            .translation
            .lerp(self.offset.translation, t);
        Some((
            place(&before).lerp(place(&now), t) + Vec3::new(eased.x, 0.0, eased.z),
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
    pub(crate) parts: [Option<Handle<Mesh>>; PARTS],
    /// The blend on the mesh: two frames and the bits of how far between them.
    shown: Option<((u8, i16), (u8, i16), u32)>,
    /// Whether the vertex colours are the burnt black ones.
    burnt: bool,
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
                burnt: false,
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
    shields: Res<'w, crate::shield::ShieldModels>,
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
            let water = crate::exhaust::water_icons(&self.textures.dram);
            crate::exhaust::spawn(
                &mut self.commands,
                &mut self.images,
                &mut self.meshes,
                &mut self.materials,
                &mut self.subtract_materials,
                icon,
                &water,
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
        if let Some(shields) = &self.shields.0 {
            crate::shield::spawn(
                shields,
                vram,
                kart,
                if ours { crate::SHIELD_LAYER } else { 0 },
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
pub fn show(
    fixed: Res<Time<Fixed>>,
    lobby: Res<crate::lobby::Lobby>,
    mut karts: Query<(&mut Transform, &mut Visibility, &KartMotion, Has<Kart>)>,
) {
    let t = fixed.overstep_fraction();
    // A spectator has no kart of its own: the others' are all drawn, and ours is not.
    let spectating = lobby.online.as_ref().is_some_and(|net| net.spectating());
    for (mut transform, mut visibility, motion, ours) in &mut karts {
        let want = if motion.hidden || (ours && spectating) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != want {
            *visibility = want;
        }
        let pose = motion.pose(t);
        transform.translation = pose.translation;
        transform.rotation = pose.rotation;
        transform.scale = pose.scale;
    }
}

/// What marks a part that [`hide_parts`] hid with its kart.
#[derive(Component)]
pub struct HiddenWithKart;

/// A kart that is not drawn (a plant ate it) takes everything that belongs to it along: its
/// wheels, shadow, smoke, tire marks, mask and shield. This is the one place that says so; the
/// parts only decide when they show otherwise. When the kart is back the parts show as they
/// would (each decides that again on its next update).
pub fn hide_parts(
    mut commands: Commands,
    karts: Query<&KartMotion>,
    mut attached: Query<
        (Entity, &Attached, &mut Visibility, Has<HiddenWithKart>),
        Without<crate::wheels::Wheels>,
    >,
    mut wheels: Query<(
        Entity,
        &crate::wheels::Wheels,
        &mut Visibility,
        Has<HiddenWithKart>,
    )>,
) {
    let mut hide = |entity: Entity, kart: Entity, visibility: &mut Visibility, hidden: bool| {
        let Ok(motion) = karts.get(kart) else { return };
        if motion.is_hidden() {
            *visibility = Visibility::Hidden;
            if !hidden {
                commands.entity(entity).insert(HiddenWithKart);
            }
        } else if hidden {
            *visibility = Visibility::Inherited;
            commands.entity(entity).remove::<HiddenWithKart>();
        }
    };
    for (entity, Attached(kart), mut visibility, hidden) in &mut attached {
        hide(entity, *kart, &mut visibility, hidden);
    }
    // (The wheels belong to a kart that need not be a driven one, so they are not `Attached`,
    // which the despawning goes by.)
    for (entity, wheels, mut visibility, hidden) in &mut wheels {
        hide(entity, wheels.kart(), &mut visibility, hidden);
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
        if rig.burnt != motion.burnt
            && let Some(frame) = animation_pose(&rig.header, motion.animation.0, motion.animation.1)
        {
            // The colours are the same in every pose; a burnt kart has none.
            let colors = mesh::model_vertices(
                &rig.header,
                &frame,
                [render::wheels::DRIVER_INSTANCE_SCALE as i16; 3],
            );
            for (handle, (_, colors)) in rig.parts.iter().zip(colors) {
                let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) else {
                    continue;
                };
                let colors = if motion.burnt {
                    vec![[0.0, 0.0, 0.0, 1.0]; colors.len()]
                } else {
                    colors
                };
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            }
            rig.burnt = motion.burnt;
        }
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
