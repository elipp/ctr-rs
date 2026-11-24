//! A solo run's ghosts, raced against: each a `race::solo::Ghost` (a race of its own, fed a
//! stored run's pads) in `CurrentRace::ghosts`, its kart drawn see-through and greyed. There are
//! two: our personal best and the world record (the leaderboard's #1, `world_record.rs`).
//! Select or [`Control::Ghost`] cycles which are shown ([`GhostPick`]).
//!
//! The kart has a [`KartMotion`] but no `KartSlot`: everything that draws or hears a kart by its
//! slot in `Sim::race` (effects, sounds, camera, names) must never take it for one of the race's.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, ColorWrites, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use serde::{Deserialize, Serialize};

use crate::controls::Control;
use crate::kart::{KartBuilder, KartMotion};
use crate::race_on::RaceOn;
use crate::toast::Toast;

/// How a ghost's kart is drawn. Not retail's look (user: it only has to read as a ghost).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub alpha: f32,
    /// 1 keeps the colours, 0 is grey.
    pub saturation: f32,
    /// What the greyed colours are multiplied by: tells two ghosts apart when both are shown.
    pub tint: [f32; 3],
}

impl Fade {
    const BEST: Fade = Fade {
        alpha: 0.45,
        saturation: 0.35,
        tint: [1.0, 1.0, 1.0],
    };

    const WORLD: Fade = Fade {
        alpha: 0.45,
        saturation: 0.35,
        tint: [1.0, 0.85, 0.45],
    };

    fn grey(self, [r, g, b]: [f32; 3]) -> [f32; 3] {
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        let grey = [r, g, b].map(|c| luma + (c - luma) * self.saturation);
        std::array::from_fn(|i| grey[i] * self.tint[i])
    }

    pub fn color(self, [r, g, b, a]: [f32; 4]) -> [f32; 4] {
        let [r, g, b] = self.grey([r, g, b]);
        [r, g, b, a]
    }

    /// Greys a picture's texels; alpha is left alone (`alpha` goes on the material).
    pub fn rgba(self, rgba: &mut [u8]) {
        for texel in rgba.chunks_exact_mut(4) {
            let rgb = [0, 1, 2].map(|i| texel[i] as f32);
            for (out, c) in texel.iter_mut().zip(self.grey(rgb)) {
                *out = c.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Writes depth and no colour: a twin of each of the ghost's solid parts, on the same mesh, so
/// that the blended body only draws its frontmost surface. Blending writes no depth, so without
/// it a racer's face showed through the back of its head. The twin must keep the body's depth
/// bias (and transform), or the body would fail the depth test against it.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct DepthOnly {}

impl MaterialExtension for DepthOnly {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        for target in descriptor
            .fragment
            .iter_mut()
            .flat_map(|f| f.targets.iter_mut())
            .flatten()
        {
            target.write_mask = ColorWrites::empty();
        }
        Ok(())
    }
}

pub type DepthOnlyMaterial = ExtendedMaterial<StandardMaterial, DepthOnly>;

/// Which ghost: the source of its run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Our personal best (`pb.rs`).
    Best,
    /// The leaderboard's #1.
    World,
}

impl Source {
    pub const ALL: [Source; 2] = [Source::Best, Source::World];

    pub fn fade(self) -> Fade {
        match self {
            Source::Best => Fade::BEST,
            Source::World => Fade::WORLD,
        }
    }
}

/// A solo run's ghosts, those there are. A ghost only plays forward (`step`), so a restart must
/// build each again at tick 0, as `drive_tick` does `best`.
#[derive(Default)]
pub struct Ghosts {
    pub best: Option<race::solo::Ghost>,
    pub world: Option<race::solo::Ghost>,
}

impl Ghosts {
    pub fn get(&self, source: Source) -> Option<&race::solo::Ghost> {
        match source {
            Source::Best => self.best.as_ref(),
            Source::World => self.world.as_ref(),
        }
    }

    fn get_mut(&mut self, source: Source) -> Option<&mut race::solo::Ghost> {
        match source {
            Source::Best => self.best.as_mut(),
            Source::World => self.world.as_mut(),
        }
    }

    fn have(&self) -> Shown {
        Shown(Source::ALL.map(|s| self.get(s).is_some()))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Source, &race::solo::Ghost)> {
        Source::ALL.into_iter().filter_map(|s| Some((s, self.get(s)?)))
    }

    /// The ghosts `pick` shows of these.
    pub fn shown(&self, pick: GhostPick) -> Shown {
        pick.shown(self.have())
    }
}

/// A set of ghosts, by [`Source::ALL`]'s order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown([bool; 2]);

impl Shown {
    pub fn has(self, source: Source) -> bool {
        self.0[source as usize]
    }

    fn none(self) -> bool {
        self.0 == [false; 2]
    }

    fn said(self) -> &'static str {
        match self.0 {
            [true, true] => "Ghosts: both",
            [true, false] => "Ghost: your best",
            [false, true] => "Ghost: server record",
            [false, false] => "Ghosts off",
        }
    }
}

/// Which ghosts to race against, as picked: Select cycles it, and the config keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GhostPick {
    #[default]
    Best,
    World,
    Both,
    Off,
}

impl GhostPick {
    const CYCLE: [GhostPick; 4] = [GhostPick::Best, GhostPick::World, GhostPick::Both, GhostPick::Off];

    /// The ghosts shown of those there are (`have`). A pick of none there are shows all there are,
    /// so that a pick kept from a race that had the world record still shows our best without it.
    fn shown(self, have: Shown) -> Shown {
        let wanted = match self {
            GhostPick::Best => [true, false],
            GhostPick::World => [false, true],
            GhostPick::Both => [true, true],
            GhostPick::Off => return Shown([false; 2]),
        };
        let shown = Shown(std::array::from_fn(|i| wanted[i] && have.0[i]));
        if shown.none() { have } else { shown }
    }

    /// The next pick that shows something else of `have`: one that would look the same is
    /// skipped, so that with our best alone Select is on and off.
    fn next(self, have: Shown) -> GhostPick {
        let now = self.shown(have);
        let at = Self::CYCLE.iter().position(|&p| p == self).unwrap();
        (1..Self::CYCLE.len())
            .map(|i| Self::CYCLE[(at + i) % Self::CYCLE.len()])
            .find(|p| p.shown(have) != now)
            .unwrap_or(self)
    }
}

/// The kart of the `source` ghost of `CurrentRace::ghosts`, of `character`. `tick` is the ghost
/// race's tick it was pushed last: a ghost that is behind that was put back on the grid.
#[derive(Component)]
pub struct GhostKart {
    pub source: Source,
    character: usize,
    tick: u32,
}

pub struct GhostPlugin;

impl Plugin for GhostPlugin {
    fn build(&self, app: &mut App) {
        // (`step` runs in `drive_tick`'s chain: `drive/mod.rs`.)
        app.add_plugins(MaterialPlugin::<DepthOnlyMaterial>::default())
            .add_systems(Update, (build, toggle, hint).run_if(crate::drive::driving));
    }
}

/// Spawns each ghost's kart when the race has the ghost, takes it away when it hasn't (or it is
/// another racer's: a new best of another character).
fn build(race_on: Res<RaceOn>, karts: Query<(Entity, &GhostKart)>, mut builder: KartBuilder) {
    let ghosts = race_on.get().map(|race| &race.ghosts);
    let ghost = |source| ghosts.and_then(|g| g.get(source));
    let mut have = Shown([false; 2]);
    for (entity, kart) in &karts {
        let same = ghost(kart.source)
            .is_some_and(|g| g.kart().driver.character as usize == kart.character);
        if same && !have.has(kart.source) {
            have.0[kart.source as usize] = true;
        } else {
            builder.commands.entity(entity).try_despawn();
        }
    }
    for source in Source::ALL {
        let Some(ghost) = ghost(source).filter(|_| !have.has(source)) else {
            continue;
        };
        let kart = ghost.kart();
        let character = kart.driver.character as usize;
        let (model, wheel_icons) = crate::load_racer(character);
        let entity = builder.spawn_ghost(&model, &wheel_icons, kart, source.fade());
        builder.commands.entity(entity).insert(GhostKart {
            source,
            character,
            tick: ghost.race().tick,
        });
    }
}

/// Plays the ghosts up to the race's tick, after `drive_tick` played it. A run that played no tick
/// (paused, waiting on the grid) freezes the ghosts' drawing as `drive_tick` freezes the karts'.
pub(crate) fn step(
    track: Res<crate::Track>,
    sim: Res<crate::props::Sim>,
    mut race_on: ResMut<RaceOn>,
    mut karts: Query<(&mut KartMotion, &mut GhostKart)>,
) {
    let Some(ghosts) = race_on.get_mut().map(|race| &mut race.ghosts) else {
        return;
    };
    for source in Source::ALL {
        if let Some(ghost) = ghosts.get_mut(source) {
            while ghost.race().tick < sim.race.tick {
                ghost.tick(&track.0);
            }
        }
    }
    for (mut motion, mut kart) in &mut karts {
        let Some(ghost) = ghosts.get(kart.source) else {
            continue;
        };
        let now = ghost.race().tick;
        if now < kart.tick {
            motion.reset(ghost.kart());
        } else if now > kart.tick {
            motion.push(ghost.kart());
        }
        kart.tick = now;
    }
}

/// Select, which opens the racer menu outside a race (where the racers are fixed), or the
/// ghost's key: the next [`GhostPick`].
fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    pads: crate::drive::Pads,
    mut config: ResMut<crate::config::Config>,
    menu: Res<crate::pause::PauseMenu>,
    race_on: Res<RaceOn>,
    mut toasts: MessageWriter<Toast>,
) {
    let Some(race) = race_on.get().filter(|race| race.attempt().is_some()) else {
        return;
    };
    // An end screen that offers a submit has Select for it.
    if menu.open || race.solo_end.is_some_and(|end| end.submit.is_some()) {
        return;
    }
    if !select_pressed(&keys, &pads, &config) {
        return;
    }
    let have = race.ghosts.have();
    let said = if have.none() {
        "No ghost to race yet"
    } else {
        let pick = config.ghosts.next(have);
        config.ghosts = pick;
        pick.shown(have).said()
    };
    config.ghost_hint_done = true;
    if let Err(why) = config.save() {
        warn!("{why}");
    }
    toasts.write(Toast::new(said, 1500));
}

/// Until the ghost's key has been pressed once, a solo run that has a ghost says at each start
/// (the grid, or a restart back to it) what the key does.
fn hint(
    sim: Res<crate::props::Sim>,
    race_on: Res<RaceOn>,
    config: Res<crate::config::Config>,
    pads: crate::drive::Pads,
    mut last_tick: Local<Option<u32>>,
    mut toasts: MessageWriter<Toast>,
) {
    let ghosts = (race_on.get())
        .filter(|race| race.attempt().is_some())
        .map(|race| &race.ghosts);
    if config.ghost_hint_done || ghosts.is_none_or(|g| g.have().none()) {
        *last_tick = None;
        return;
    }
    let tick = sim.race.tick;
    if last_tick.is_none_or(|last| tick < last) {
        let Some(button) = select_label(&pads, &config) else {
            return;
        };
        toasts.write(Toast::new(format!("{button}: ghosts"), 4000));
    }
    *last_tick = Some(tick);
}

/// How Select and the ghost's key are named to the player: the end screen's submit
/// (`scores.rs`) is on them too. None if neither can be pressed.
pub fn select_label(pads: &crate::drive::Pads, config: &crate::config::Config) -> Option<String> {
    // (Whether a pad is plugged in, not which one drives: a keyboard player with a pad in reads
    // Select too.)
    let key = config.controls.keys(Control::Ghost).first().copied();
    let key = key.map(crate::controls::label);
    match (pads.iter().next().is_some(), key) {
        (true, Some(key)) => Some(format!("Select / {key}")),
        (true, None) => Some("Select".to_string()),
        (false, Some(key)) => Some(key.to_string()),
        (false, None) => None,
    }
}

/// Select or the ghost's key, just pressed.
pub fn select_pressed(
    keys: &ButtonInput<KeyCode>,
    pads: &crate::drive::Pads,
    config: &crate::config::Config,
) -> bool {
    keys.any_just_pressed(config.controls.keys(Control::Ghost).iter().copied())
        || pads.iter().any(|p| p.just_pressed(GamepadButton::Select))
}
