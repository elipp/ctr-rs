//! The player's controls: keyboard and pads, read into the sim's input.

use bevy::prelude::*;
use race::Pad;
use vehicle::{Input, buttons};

pub(super) fn read_input(keys: &ButtonInput<KeyCode>, pads: &Query<&Gamepad>, previous_held: u32) -> Input {
    let down = |codes: &[KeyCode]| codes.iter().any(|&k| keys.pressed(k));
    let pad_down =
        |codes: &[GamepadButton]| pads.iter().any(|p| p.any_pressed(codes.iter().copied()));
    // Analog left stick of the first pad that has it pushed, deadzone already applied.
    let stick = pads
        .iter()
        .map(|p| p.left_stick())
        .find(|s| *s != Vec2::ZERO)
        .unwrap_or(Vec2::ZERO);

    let mut held = 0;
    if down(&[KeyCode::ArrowUp]) || pad_down(&[GamepadButton::South]) {
        held |= buttons::CROSS;
    }
    // Square brakes. Reversing is the stick (or D-pad) pulled down (`stickLY`, see
    // `gas_and_brake`): alone, it reverses when the gas is not held, and with Square it
    // brakes into reverse. On the D-pad it leaves Square alone, so it does not end a
    // powerslide. The keyboard's S and Down are both, like Square plus Down on a pad.
    let key_reverse = down(&[KeyCode::ArrowDown]);
    if key_reverse || pad_down(&[GamepadButton::West]) {
        held |= buttons::SQUARE;
    }
    // Up and Down aim a thrown weapon: a bomb or a potion goes backward with Down, a potion
    // forward with Up, and a potion is dropped with neither. The keyboard's S is Down, as it is
    // for the brake; W is the gas, so Up is E.
    if down(&[KeyCode::KeyE]) || pad_down(&[GamepadButton::DPadUp]) {
        held |= buttons::UP;
    }
    if key_reverse || pad_down(&[GamepadButton::DPadDown]) {
        held |= buttons::DOWN;
    }
    if down(&[KeyCode::Space, KeyCode::ShiftLeft, KeyCode::ShiftRight])
        || pad_down(&[GamepadButton::RightTrigger, GamepadButton::RightTrigger2])
    {
        held |= buttons::R1;
    }
    if down(&[KeyCode::KeyQ, KeyCode::ControlLeft])
        || pad_down(&[GamepadButton::LeftTrigger, GamepadButton::LeftTrigger2])
    {
        held |= buttons::L1;
    }
    // The mask item is Circle, so a demo has it.
    if down(&[KeyCode::KeyD]) || pad_down(&[GamepadButton::East]) {
        held |= buttons::CIRCLE;
    }
    let left = down(&[KeyCode::ArrowLeft]) || pad_down(&[GamepadButton::DPadLeft]);
    let right = down(&[KeyCode::ArrowRight]) || pad_down(&[GamepadButton::DPadRight]);
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

