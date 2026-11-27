//! Which of our own input (`Device::Primary`: the keys and every pad no other player has) we last
//! used, keys or a pad, for the prompts to show its buttons: a keyboard player with a pad plugged
//! in reads keys until they pick the pad up. (Not retail.)

use bevy::input::ButtonState;
use bevy::input::gamepad::{GamepadAxisChangedEvent, GamepadButtonChangedEvent};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;

use crate::drive::{Device, Drive, Pads};

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LastInput {
    #[default]
    Keys,
    Pad,
}

impl LastInput {
    /// Whether our own prompts show a pad's buttons.
    pub(crate) fn pad(self) -> bool {
        self == LastInput::Pad
    }

    /// Whether `device`'s prompts show a pad's buttons: a pad's always, a set of keys' never.
    pub(crate) fn pad_of(self, device: &Device) -> bool {
        match device {
            Device::Primary => self.pad(),
            Device::Keys(_) => false,
            Device::Pad(_) => true,
        }
    }
}

pub struct LastInputPlugin;

impl Plugin for LastInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastInput>()
            .add_systems(PreUpdate, track.after(bevy::input::InputSystems));
    }
}

/// A key or a mouse button pressed is the keys; a button pressed or a stick pushed of a pad that
/// is not another player's is a pad. From the messages, not `ButtonInput`: a reader may have
/// consumed the press before this sees it.
fn track(
    mut keys: MessageReader<KeyboardInput>,
    mut mouse: MessageReader<MouseButtonInput>,
    mut buttons: MessageReader<GamepadButtonChangedEvent>,
    mut axes: MessageReader<GamepadAxisChangedEvent>,
    pads: Pads,
    drive: Option<Res<Drive>>,
    mut last: ResMut<LastInput>,
) {
    // (`Pads` has none while the window is not focused.)
    let ours = |pad: Entity| {
        pads.entities().any(|(entity, _)| entity == pad)
            && !drive.iter().flat_map(|d| d.views()).any(|v| v.device.pad() == Some(pad))
    };
    let pressed = |state: ButtonState| state == ButtonState::Pressed;
    let button = buttons.read().any(|b| pressed(b.state) && ours(b.entity));
    let axis = axes.read().any(|a| a.value.abs() > crate::drive::MENU_STICK && ours(a.entity));
    let key = keys.read().any(|k| pressed(k.state)) | mouse.read().any(|m| pressed(m.state));
    let now = if button || axis {
        LastInput::Pad
    } else if key {
        LastInput::Keys
    } else {
        return;
    };
    // (Not set unchanged: readers may go by its change ticks.)
    last.set_if_neq(now);
}
