//! A solo run's personal best, raced against: `race::solo::Ghost` (a race of its own, fed the
//! stored run's pads) kept in `CurrentRace::ghost`, its kart drawn see-through and greyed.
//!
//! The kart has a [`KartMotion`] but no `KartSlot`: everything that draws or hears a kart by its
//! slot in `Sim::race` (effects, sounds, camera, names) must never take it for one of the race's.

use bevy::prelude::*;

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
}

impl Fade {
    pub const GHOST: Fade = Fade {
        alpha: 0.45,
        saturation: 0.35,
    };

    fn grey(self, [r, g, b]: [f32; 3]) -> [f32; 3] {
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        [r, g, b].map(|c| luma + (c - luma) * self.saturation)
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

/// Whether ghosts are drawn: Select, or [`Control::Ghost`], in a solo run. Kept across races.
#[derive(Resource)]
pub struct GhostShown(pub bool);

impl Default for GhostShown {
    fn default() -> GhostShown {
        GhostShown(true)
    }
}

/// The kart of `CurrentRace::ghost`, of `character`. `tick` is the ghost race's tick it was
/// pushed last: a ghost that is behind that was put back on the grid.
#[derive(Component)]
pub struct GhostKart {
    character: usize,
    tick: u32,
}

pub struct GhostPlugin;

impl Plugin for GhostPlugin {
    fn build(&self, app: &mut App) {
        // (`step` runs in `drive_tick`'s chain: `drive/mod.rs`.)
        app.init_resource::<GhostShown>()
            .add_systems(Update, (build, toggle).run_if(crate::drive::driving));
    }
}

/// Spawns the ghost's kart when the race has a ghost, takes it away when it hasn't (or it is
/// another racer's: a new best of another character).
fn build(
    race_on: Res<RaceOn>,
    karts: Query<(Entity, &GhostKart)>,
    mut builder: KartBuilder,
) {
    let ghost = race_on.get().and_then(|race| race.ghost.as_ref());
    let mut have = false;
    for (entity, kart) in &karts {
        if ghost.is_some_and(|g| g.kart().driver.character as usize == kart.character) && !have {
            have = true;
        } else {
            builder.commands.entity(entity).try_despawn();
        }
    }
    let Some(ghost) = ghost.filter(|_| !have) else {
        return;
    };
    let kart = ghost.kart();
    let character = kart.driver.character as usize;
    let (model, wheel_icons) = crate::load_racer(character);
    let entity = builder.spawn_ghost(&model, &wheel_icons, kart);
    builder.commands.entity(entity).insert(GhostKart {
        character,
        tick: ghost.race().tick,
    });
}

/// Plays the ghost up to the race's tick, after `drive_tick` played it. A run that played no tick
/// (paused, waiting on the grid) freezes the ghost's drawing as `drive_tick` freezes the karts'.
pub(crate) fn step(
    track: Res<crate::Track>,
    sim: Res<crate::props::Sim>,
    mut race_on: ResMut<RaceOn>,
    mut karts: Query<(&mut KartMotion, &mut GhostKart)>,
) {
    let Some(ghost) = race_on.get_mut().and_then(|race| race.ghost.as_mut()) else {
        return;
    };
    while ghost.race().tick < sim.race.tick {
        ghost.tick(&track.0);
    }
    let now = ghost.race().tick;
    for (mut motion, mut kart) in &mut karts {
        if now < kart.tick {
            motion.reset(ghost.kart());
        } else if now > kart.tick {
            motion.push(ghost.kart());
        } else {
            motion.freeze();
        }
        kart.tick = now;
    }
}

/// Select, which opens the racer menu outside a race (where the racers are fixed), or the
/// ghost's key.
fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    pads: crate::drive::Pads,
    config: Res<crate::config::Config>,
    menu: Res<crate::pause::PauseMenu>,
    race_on: Res<RaceOn>,
    mut shown: ResMut<GhostShown>,
    mut toasts: MessageWriter<Toast>,
) {
    if menu.open || race_on.solo().is_none() {
        return;
    }
    let key = keys.any_just_pressed(config.controls.keys(Control::Ghost).iter().copied());
    if key || pads.iter().any(|p| p.just_pressed(GamepadButton::Select)) {
        shown.0 = !shown.0;
        let has = race_on.get().is_some_and(|race| race.ghost.is_some());
        let said = match (shown.0, has) {
            (false, _) => "Ghost off",
            (true, true) => "Ghost on",
            (true, false) => "Ghost on (no personal best to race yet)",
        };
        toasts.write(Toast::new(said, 1500));
    }
}
