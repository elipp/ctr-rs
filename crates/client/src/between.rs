//! How far past the last played tick to draw.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::props::Sim;

/// The sim's tick after the last fixed step, and whether that step left it where it was.
#[derive(Resource, Default)]
struct Played {
    tick: u32,
    held: bool,
}

/// After every system of the fixed step, so that any of them may play the tick.
fn note(sim: Option<Res<Sim>>, mut played: ResMut<Played>) {
    let Some(sim) = sim else { return };
    let tick = sim.race.tick();
    played.held = tick == played.tick;
    played.tick = tick;
}

/// What is drawn between ticks goes by this, never by `Time<Fixed>` itself: a fixed step that
/// played no tick (a wait for a late player, a held race) would otherwise run the last tick's
/// motion over again, every step.
#[derive(SystemParam)]
pub struct Between<'w> {
    fixed: Res<'w, Time<Fixed>>,
    played: Res<'w, Played>,
}

impl Between<'_> {
    /// 1 while no tick is played: the next one carries on from there, as its `previous` is the
    /// `current` drawn now.
    pub fn fraction(&self) -> f32 {
        if self.played.held {
            1.0
        } else {
            self.fixed.overstep_fraction()
        }
    }
}

pub struct BetweenPlugin;

impl Plugin for BetweenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Played>()
            .add_systems(FixedPostUpdate, note);
    }
}
