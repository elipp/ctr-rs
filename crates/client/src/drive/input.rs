//! The player's controls: keyboard and pads, read into the sim's input. The keyboard's keys are
//! the config's (`controls.rs`).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use race::Pad;
use vehicle::{Input, buttons};

use crate::config::Config;
use crate::controls::Control;

/// The pads, as long as the window has focus: a pad pushed while another window is in front is not
/// ours to read.
#[derive(SystemParam)]
pub(crate) struct Pads<'w, 's> {
    pads: Query<'w, 's, &'static Gamepad>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

impl Pads<'_, '_> {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Gamepad> {
        let focused = self.windows.iter().any(|w| w.focused);
        self.pads.iter().filter(move |_| focused)
    }
}

/// The keyboard, the pads, and the keys the config binds.
#[derive(SystemParam)]
pub(super) struct Controller<'w, 's> {
    pub keys: Res<'w, ButtonInput<KeyCode>>,
    pads: Pads<'w, 's>,
    config: Res<'w, Config>,
}

impl Controller<'_, '_> {
    pub fn read(&self, previous_held: u32) -> Input {
        read_input(&self.keys, &self.pads, &self.config, previous_held)
    }
}

fn read_input(
    keys: &ButtonInput<KeyCode>,
    pads: &Pads,
    config: &Config,
    previous_held: u32,
) -> Input {
    let down = |control: Control| config.controls.held(control, keys);
    let pad_down =
        |codes: &[GamepadButton]| pads.iter().any(|p| p.any_pressed(codes.iter().copied()));
    // Analog left stick of the first pad that has it pushed, deadzone already applied.
    let stick = pads
        .iter()
        .map(|p| p.left_stick())
        .find(|s| *s != Vec2::ZERO)
        .unwrap_or(Vec2::ZERO);

    let mut held = 0;
    if down(Control::Gas) || pad_down(&[GamepadButton::South]) {
        held |= buttons::CROSS;
    }
    // Square brakes. Reversing is the stick (or D-pad) pulled down (`stickLY`, see
    // `gas_and_brake`): alone, it reverses when the gas is not held, and with Square it
    // brakes into reverse. On the D-pad it leaves Square alone, so it does not end a
    // powerslide. The keyboard's brake is both, like Square plus Down on a pad.
    let key_reverse = down(Control::Brake);
    if key_reverse || pad_down(&[GamepadButton::West]) {
        held |= buttons::SQUARE;
    }
    // Up and Down aim a thrown weapon: a bomb or a potion goes backward with Down, a potion
    // forward with Up, and a potion is dropped with neither. The keyboard's Down is the brake,
    // and its Up a key of its own, as the gas is Up already.
    if down(Control::AimUp) || pad_down(&[GamepadButton::DPadUp]) {
        held |= buttons::UP;
    }
    if key_reverse || pad_down(&[GamepadButton::DPadDown]) {
        held |= buttons::DOWN;
    }
    if down(Control::Hop) || pad_down(&[GamepadButton::RightTrigger, GamepadButton::RightTrigger2])
    {
        held |= buttons::R1;
    }
    if down(Control::Hop2) || pad_down(&[GamepadButton::LeftTrigger, GamepadButton::LeftTrigger2]) {
        held |= buttons::L1;
    }
    // Circle fires the item.
    if down(Control::Item) || pad_down(&[GamepadButton::East]) {
        held |= buttons::CIRCLE;
    }
    let left = down(Control::Left) || pad_down(&[GamepadButton::DPadLeft]);
    let right = down(Control::Right) || pad_down(&[GamepadButton::DPadRight]);
    // Sticks: 0 is left, 0x80 the middle, 0xff right (and back for LY).
    let axis = |v: f32| (0x80 as f32 + v * 127.0).round().clamp(0.0, 255.0) as u8;
    let stick_lx = match (left, right) {
        (true, false) => 0,
        (false, true) => 0xff,
        _ => axis(stick.x),
    };
    // 0 is up and 0xff down. The D-pad pushes the stick all the way (retail steps it there).
    let stick_ly = if key_reverse || pad_down(&[GamepadButton::DPadDown]) {
        0xff
    } else {
        axis(-stick.y)
    };
    Input {
        buttons_held: held,
        buttons_tapped: held & !previous_held,
        stick_lx,
        stick_ly,
        stick_ry: 0x80,
    }
}

/// The pad of the race for a pad of the network.
pub(crate) fn pad_of(pad: &net::Pad) -> Pad {
    Pad {
        held: pad.held,
        lx: pad.lx,
        ly: pad.ly,
        ry: pad.ry,
    }
}
