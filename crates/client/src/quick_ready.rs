//! Quick play's ready check while a match waits for a race (its first too): Triangle (or the
//! speedometer's key) of our first player says we are ready for the next race (`Action::QuickReady`), or not after all
//! (`Action::QuickUnready`), until its countdown. The lobby list hints at it on our row
//! (`lobby_list.rs`). Leaving is the pause menu's. (Not retail.)

use bevy::prelude::*;

use crate::command::Action;
use crate::controls::Control;
use crate::drive::{Controller, Device, Drive};
use crate::lobby::Lobby;
use crate::race_on::RaceOn;

/// What the lobby list puts on our row while we may say we are ready, or not: the button to
/// press, and what it does.
#[derive(Resource, Default)]
pub(crate) struct ReadyHint(pub(crate) Option<String>);

pub struct QuickReadyPlugin;

impl Plugin for QuickReadyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReadyHint>().add_systems(
            PreUpdate,
            input
                .after(bevy::input::InputSystems)
                .before(crate::pause::input)
                .run_if(crate::drive::driving),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn input(
    // The controller reads the keys and the pads, which the Triangle taken is then cleared from.
    mut input: ParamSet<(Controller, ResMut<ButtonInput<KeyCode>>, Query<&mut Gamepad>)>,
    drive: Option<Res<Drive>>,
    config: Res<crate::config::Config>,
    lobby: Res<Lobby>,
    race_on: Res<RaceOn>,
    pause: Res<crate::pause::PauseMenu>,
    sounds: Option<Res<crate::sound::Sounds>>,
    last: Res<crate::last_input::LastInput>,
    mut hint: ResMut<ReadyHint>,
    mut actions: MessageWriter<Action>,
    mut stick: Local<IVec2>,
) {
    let view = drive.as_ref().and_then(|d| d.views().first());
    let wanted = lobby.matched_waiting(&race_on) && lobby.starting.is_none();
    let Some(view) = view.filter(|_| wanted) else {
        hint.0 = None;
        return;
    };
    let keys = match &view.device {
        Device::Keys(controls) => controls.keys(Control::Speedometer),
        _ => config.controls.keys(Control::Speedometer),
    };
    let does = if lobby.again { "not ready" } else { "ready" };
    let prompt = crate::prompt::Prompt::bound(crate::pad_glyph::PadGlyph::Triangle, keys, does);
    let text = Some(prompt.text(last.pad_of(&view.device)));
    if hint.0 != text {
        hint.0 = text;
    }
    if pause.open || pause.takes_keys() {
        return;
    }
    let others: Vec<Entity> =
        drive.iter().flat_map(|d| d.views().iter().filter_map(|v| v.device.pad())).collect();
    if !input.p0().menu(&view.device, &others, &mut stick).triangle {
        return;
    }
    actions.write(if lobby.again {
        Action::QuickUnready
    } else {
        Action::QuickReady
    });
    if let Some(sounds) = &sounds {
        sounds.menu(crate::sound::MENU_TAKE_SFX);
    }
    // (Not the speedometer's too.)
    for &k in config.controls.keys(Control::Speedometer) {
        input.p1().clear_just_pressed(k);
    }
    for mut p in &mut input.p2() {
        p.digital_mut().clear_just_pressed(GamepadButton::North);
    }
}
