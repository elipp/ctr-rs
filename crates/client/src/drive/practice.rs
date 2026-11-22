//! Shortcut practice. A mark is the race [`RUN_UP`] ticks before the point and the pads played
//! since; a run-up resumes the sim from that race and plays those pads, and the sim being
//! deterministic (`race::rollback::Simulation`) brings the kart to the point exactly as it came.
//! Only in a free drive of one player offline, without `/autopilot`: the user's scope, not a
//! limit of the approach (a lobby's sim is everyone's, though).

use std::collections::VecDeque;

use bevy::prelude::*;
use race::{Pad, Race};

use super::input::Pads;
use crate::controls::{Control, Controls};
use crate::kart::{KartBuilder, KartMotion};
use crate::props::Sim;
use crate::toast::Toast;

/// 2 seconds.
const RUN_UP: usize = 60;

/// `/practice set | run-up`, or the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PracticeAsk {
    SetPoint,
    RunUp,
}

struct Mark {
    race: Race,
    pads: Vec<Pad>,
}

#[derive(Resource, Default)]
pub(crate) struct Practice {
    /// The last [`RUN_UP`] ticks: the race before each, and the pad it was played with.
    recent: VecDeque<(Race, Pad)>,
    mark: Option<Mark>,
    /// How many of the mark's pads the run-up has played, while it plays.
    playing: Option<usize>,
    /// Keys are read each frame and the sim steps at fixed ticks: the ask waits for the next tick.
    wants: Option<PracticeAsk>,
}

/// A racer swapped in place (`scene::swap_character`) changes the race but not a kept copy of it,
/// and the copy's kart would be drawn with the new model's animation frames.
fn character(race: &Race) -> u8 {
    race.karts[0].driver.character
}

impl Practice {
    /// Before the tick's pads. True if the sim was put back: what is drawn has to start over.
    pub(super) fn step(
        &mut self,
        sim: &mut Sim,
        allowed: bool,
        controls: &Controls,
        toasts: &mut MessageWriter<Toast>,
    ) -> bool {
        let want = self.wants.take();
        if !allowed {
            self.recent.clear();
            self.playing = None;
            if want.is_some() {
                toasts.write(Toast::new("Shortcut practice is for a free drive alone", 2000));
            }
            return false;
        }
        match want {
            None => false,
            Some(PracticeAsk::SetPoint) => {
                let race = match self.recent.front() {
                    Some((race, _)) => race.clone(),
                    None => sim.race.state().clone(),
                };
                let pads = self.recent.iter().map(|&(_, pad)| pad).collect();
                self.mark = Some(Mark { race, pads });
                toasts.write(Toast::new("Practice point set", 1200));
                false
            }
            Some(PracticeAsk::RunUp) => {
                let Some(mark) = &self.mark else {
                    let key = controls.describe(Control::SetPoint);
                    toasts.write(Toast::new(format!("No point yet: {key} sets one"), 2000));
                    return false;
                };
                if !mark.pads.is_empty() {
                    let millis = mark.pads.len() as u64 * 1000 / crate::GAME_TICK_HZ as u64;
                    toasts.write(Toast::new("Run-up...", millis));
                }
                if character(&mark.race) != character(sim.race.state()) {
                    self.mark = None;
                    toasts.write(Toast::new("New racer: set the point again", 2000));
                    return false;
                }
                sim.resume(mark.race.clone());
                // (The run-up records the same ticks again.)
                self.recent.clear();
                self.playing = Some(0);
                true
            }
        }
    }

    /// The run-up's pad for this tick, while one plays.
    pub(super) fn pad(&mut self) -> Option<Pad> {
        let at = self.playing?;
        let pad = self.mark.as_ref().and_then(|mark| mark.pads.get(at)).copied();
        self.playing = pad.map(|_| at + 1);
        pad
    }

    /// `race` is the state before a tick, played with `pad`.
    pub(super) fn record(&mut self, race: &Race, pad: Pad) {
        if self.recent.back().is_some_and(|(last, _)| character(last) != character(race)) {
            self.recent.clear();
        }
        if self.recent.len() == RUN_UP {
            self.recent.pop_front();
        }
        self.recent.push_back((race.clone(), pad));
    }

    pub(crate) fn running_up(&self) -> bool {
        self.playing.is_some()
    }

    /// A new sim (another level): a mark is of the old one.
    pub(super) fn forget(&mut self) {
        *self = Practice::default();
    }
}

pub(super) fn input(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Pads,
    config: Res<crate::config::Config>,
    menu: Res<crate::pause::PauseMenu>,
    mut actions: MessageReader<crate::command::Action>,
    mut practice: ResMut<Practice>,
) {
    for action in actions.read() {
        if let crate::command::Action::Practice(ask) = action {
            practice.wants = Some(*ask);
        }
    }
    if menu.open || menu.takes_keys() {
        return;
    }
    let pressed = |control, button| {
        config.controls.just_pressed(control, &keys) || pads.iter().any(|p| p.just_pressed(button))
    };
    if pressed(Control::SetPoint, GamepadButton::LeftThumb) {
        practice.wants = Some(PracticeAsk::SetPoint);
    }
    if pressed(Control::RunUp, GamepadButton::RightThumb) {
        practice.wants = Some(PracticeAsk::RunUp);
    }
}

/// Our kart's see-through stand-in while a run-up plays (`kart::show` swaps the two). It is kept
/// while there is a mark, so that loading the racer does not hitch the run-up's start, and
/// follows our kart all along. It has no `KartSlot`, like a solo ghost (`ghost.rs`): the effects
/// stay our hidden kart's, which is in the same place.
#[derive(Component)]
pub(crate) struct RunUpKart {
    character: u8,
    /// The sim's tick it was pushed last: a lower one is a run-up's (or a restart's) jump.
    tick: u32,
}

pub(super) fn build(
    practice: Res<Practice>,
    sim: Res<Sim>,
    karts: Query<(Entity, &RunUpKart)>,
    mut builder: KartBuilder,
) {
    let wanted = practice.mark.as_ref().map(|mark| character(&mark.race));
    let mut have = false;
    for (entity, kart) in &karts {
        if Some(kart.character) == wanted && !have {
            have = true;
        } else {
            builder.commands.entity(entity).try_despawn();
        }
    }
    let Some(character) = wanted.filter(|_| !have) else {
        return;
    };
    let (model, wheel_icons) = crate::load_racer(character as usize);
    let kart = sim.kart();
    let fade = crate::ghost::Source::Best.fade();
    let entity = builder.spawn_ghost(&model, &wheel_icons, kart, fade);
    builder.commands.entity(entity).insert(RunUpKart {
        character,
        tick: sim.race.tick(),
    });
}

pub(super) fn step(sim: Res<Sim>, mut karts: Query<(&mut KartMotion, &mut RunUpKart)>) {
    let now = sim.race.tick();
    for (mut motion, mut kart) in &mut karts {
        if now < kart.tick {
            motion.reset(sim.kart());
        } else if now > kart.tick {
            motion.push(sim.kart());
        }
        kart.tick = now;
    }
}
