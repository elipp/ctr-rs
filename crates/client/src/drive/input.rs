//! The player's controls: keyboard and pads, read into the sim's input. The keyboard's keys are
//! the config's (`controls.rs`).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use race::Pad;
use vehicle::{Input, buttons};

use crate::config::Config;
use crate::controls::{Control, Controls};

/// The pads, as long as the window has focus: a pad pushed while another window is in front is not
/// ours to read.
#[derive(SystemParam)]
pub(crate) struct Pads<'w, 's> {
    pads: Query<'w, 's, (Entity, &'static Gamepad)>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

impl Pads<'_, '_> {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Gamepad> {
        self.entities().map(|(_, pad)| pad)
    }

    /// The pads with their entities, which tell them apart.
    pub(crate) fn entities(&self) -> impl Iterator<Item = (Entity, &Gamepad)> {
        let focused = self.windows.iter().any(|w| w.focused);
        self.pads.iter().filter(move |_| focused)
    }
}

/// What a local player drives with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Device {
    /// Our own: the keys the config binds (the pause menu changes them), and every pad that
    /// is not another player's.
    Primary,
    /// A set of keys of its own, and no pad.
    Keys(Controls),
    /// One pad, and no keys.
    Pad(Entity),
}

impl Device {
    /// The pad this device is, which no other player reads.
    pub(crate) fn pad(&self) -> Option<Entity> {
        match self {
            Device::Pad(pad) => Some(*pad),
            _ => None,
        }
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
    /// The keys and the pads `device` reads. `taken` are the pads that are other players' (the
    /// primary device leaves them alone).
    fn sources<'a>(
        &'a self,
        device: &'a Device,
        taken: &[Entity],
    ) -> (Option<&'a Controls>, Vec<&'a Gamepad>) {
        match device {
            Device::Primary => (
                Some(&self.config.controls),
                (self.pads.entities())
                    .filter(|(entity, _)| !taken.contains(entity))
                    .map(|(_, pad)| pad)
                    .collect(),
            ),
            Device::Keys(controls) => (Some(controls), Vec::new()),
            Device::Pad(wanted) => (
                None,
                (self.pads.entities())
                    .filter(|(entity, _)| entity == wanted)
                    .map(|(_, pad)| pad)
                    .collect(),
            ),
        }
    }

    /// What `device` is giving now.
    pub fn read(&self, device: &Device, taken: &[Entity], previous_held: u32) -> Input {
        let (controls, pads) = self.sources(device, taken);
        read_input(&self.keys, controls, &pads, previous_held)
    }

    /// What `device` tapped this frame, to work a menu with: the driving keys and buttons, as
    /// the pause menu's are not for one player. `stick` is which way the left stick was
    /// pushed last frame (-1, 0 or 1 up), which the caller keeps to tap with it.
    pub fn menu(&self, device: &Device, taken: &[Entity], stick: &mut i32) -> MenuTaps {
        let (controls, pads) = self.sources(device, taken);
        let key = |control| controls.is_some_and(|c| c.just_pressed(control, &self.keys));
        let pad = |button| pads.iter().any(|p| p.just_pressed(button));
        let y = pads
            .iter()
            .map(|p| p.left_stick().y)
            .find(|y| *y != 0.0)
            .unwrap_or(0.0);
        let pushed = (y > MENU_STICK) as i32 - (y < -MENU_STICK) as i32;
        let tapped = |way: i32| pushed == way && *stick != way;
        let (stick_up, stick_down) = (tapped(1), tapped(-1));
        *stick = pushed;
        MenuTaps {
            up: key(Control::Gas) || pad(GamepadButton::DPadUp) || stick_up,
            down: key(Control::Brake) || pad(GamepadButton::DPadDown) || stick_down,
            take: key(Control::Hop) || pad(GamepadButton::South),
            back: key(Control::Item) || pad(GamepadButton::East),
            toggle: key(Control::RacerMenu) || pad(GamepadButton::Select),
        }
    }
}

/// How far the left stick is pushed to tap a menu: as in the pause menu.
const MENU_STICK: f32 = 0x60 as f32 / 127.0;

/// The taps of a device in a menu of its player's own.
#[derive(Clone, Copy, Default)]
pub(super) struct MenuTaps {
    pub up: bool,
    pub down: bool,
    /// Hop, or Cross.
    pub take: bool,
    /// Item, or Circle.
    pub back: bool,
    /// The racer menu's key, or Select: opens it and closes it.
    pub toggle: bool,
}

fn read_input(
    keys: &ButtonInput<KeyCode>,
    controls: Option<&Controls>,
    pads: &[&Gamepad],
    previous_held: u32,
) -> Input {
    let down = |control: Control| controls.is_some_and(|c| c.held(control, keys));
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
