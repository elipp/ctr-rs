//! The keyboard's driving keys, which the pause menu's Controls change (`pause.rs`) and the config
//! keeps (`config.rs`), by the names of Bevy's `KeyCode`s: `{"gas": ["ArrowUp"], ...}`. A control
//! the config leaves out has its default keys, and a name it does not know, or of a key that
//! cannot be bound, is passed over.
//!
//! A key is one control's: binding it to another takes it from the one that had it, which gets the
//! other's old keys if it is left with none. The keys the client already uses while driving
//! ([`taken`]) cannot be bound. The pads' buttons are fixed (`drive/input.rs`).

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// What a key can do while driving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// Cross.
    Gas,
    /// Square, and the stick pulled down: it brakes, and alone it reverses.
    Brake,
    Left,
    Right,
    /// R1 and L1: each hops, and a drift is held with the one that started it.
    Hop,
    Hop2,
    /// Circle: fires the item.
    Item,
    /// Up: a potion is thrown forward. (Down, to throw a bomb or a potion back, is the brake.)
    AimUp,
}

impl Control {
    pub const ALL: [Control; 8] = [
        Control::Gas,
        Control::Brake,
        Control::Left,
        Control::Right,
        Control::Hop,
        Control::Hop2,
        Control::Item,
        Control::AimUp,
    ];

    /// Its name in the config.
    fn name(self) -> &'static str {
        match self {
            Control::Gas => "gas",
            Control::Brake => "brake",
            Control::Left => "left",
            Control::Right => "right",
            Control::Hop => "hop",
            Control::Hop2 => "hop2",
            Control::Item => "item",
            Control::AimUp => "aim-up",
        }
    }

    /// Its name in the menu.
    pub fn label(self) -> &'static str {
        match self {
            Control::Gas => "Gas",
            Control::Brake => "Brake",
            Control::Left => "Left",
            Control::Right => "Right",
            Control::Hop => "Hop",
            Control::Hop2 => "Hop 2",
            Control::Item => "Item",
            Control::AimUp => "Aim up",
        }
    }

    fn defaults(self) -> &'static [KeyCode] {
        match self {
            Control::Gas => &[KeyCode::ArrowUp],
            Control::Brake => &[KeyCode::ArrowDown],
            Control::Left => &[KeyCode::ArrowLeft],
            Control::Right => &[KeyCode::ArrowRight],
            Control::Hop => &[KeyCode::Space, KeyCode::ShiftLeft, KeyCode::ShiftRight],
            Control::Hop2 => &[KeyCode::KeyQ, KeyCode::ControlLeft],
            Control::Item => &[KeyCode::KeyD],
            Control::AimUp => &[KeyCode::KeyE],
        }
    }

    fn index(self) -> usize {
        Control::ALL.iter().position(|&c| c == self).unwrap()
    }
}

/// The keys of each control, in [`Control::ALL`]'s order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    from = "BTreeMap<String, Vec<String>>",
    into = "BTreeMap<String, Vec<String>>"
)]
pub struct Controls([Vec<KeyCode>; Control::ALL.len()]);

impl Default for Controls {
    fn default() -> Controls {
        Controls(Control::ALL.map(|c| c.defaults().to_vec()))
    }
}

impl From<BTreeMap<String, Vec<String>>> for Controls {
    fn from(names: BTreeMap<String, Vec<String>>) -> Controls {
        let mut controls = Controls::default();
        for control in Control::ALL {
            if let Some(keys) = names.get(control.name()) {
                controls.0[control.index()] = (keys.iter())
                    .filter_map(|k| key_of(k))
                    .filter(|&k| taken(k).is_none())
                    .collect();
            }
        }
        controls
    }
}

impl From<Controls> for BTreeMap<String, Vec<String>> {
    fn from(controls: Controls) -> BTreeMap<String, Vec<String>> {
        Control::ALL
            .iter()
            .map(|&c| {
                let keys = controls.keys(c).iter().map(|k| format!("{k:?}")).collect();
                (c.name().to_string(), keys)
            })
            .collect()
    }
}

impl Controls {
    pub fn keys(&self, control: Control) -> &[KeyCode] {
        &self.0[control.index()]
    }

    /// Whether a key of `control` is down.
    pub fn held(&self, control: Control, keys: &ButtonInput<KeyCode>) -> bool {
        self.keys(control).iter().any(|&k| keys.pressed(k))
    }

    /// `key` is `control`'s only key. Another control that had it loses it, and takes
    /// `control`'s old keys if it has none left.
    pub fn bind(&mut self, control: Control, key: KeyCode) {
        let old = std::mem::replace(&mut self.0[control.index()], vec![key]);
        for other in Control::ALL.into_iter().filter(|&c| c != control) {
            let keys = &mut self.0[other.index()];
            if keys.contains(&key) {
                keys.retain(|&k| k != key);
                if keys.is_empty() {
                    keys.extend(old.iter().filter(|&&k| k != key));
                }
            }
        }
    }

    /// The keys of `control` as the menu shows them, `---` for none.
    pub fn describe(&self, control: Control) -> String {
        let keys = self.keys(control);
        if keys.is_empty() {
            return "---".into();
        }
        keys.iter()
            .map(|&k| label(k))
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

/// What the client already does with `key` while driving, if anything: such a key cannot be bound.
pub fn taken(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        KeyCode::Escape | KeyCode::KeyP => "opens the menu",
        KeyCode::Enter | KeyCode::NumpadEnter => "opens the command line",
        KeyCode::KeyR => "restarts the race",
        KeyCode::F5 | KeyCode::Digit1 => "records a demo",
        KeyCode::F6 | KeyCode::Digit2 => "plays the demo",
        KeyCode::F3 => "opens the effects panel",
        KeyCode::F7 | KeyCode::F8 => "stalls the client",
        KeyCode::Home => "resets the camera",
        KeyCode::BracketLeft | KeyCode::BracketRight | KeyCode::PageUp | KeyCode::PageDown => {
            "changes the level"
        }
        KeyCode::Comma | KeyCode::Period => "changes the racer",
        _ => return None,
    })
}

/// The key a name of the config is, if it is one that can be bound.
fn key_of(name: &str) -> Option<KeyCode> {
    KEYS.iter()
        .map(|&(key, _)| key)
        .find(|key| format!("{key:?}") == name)
}

/// Whether `key` can be bound at all: it has a name here, and is not [`taken`].
pub fn bindable(key: KeyCode) -> bool {
    KEYS.iter().any(|&(k, _)| k == key) && taken(key).is_none()
}

/// The name of `key` in the game's font, which has letters, digits and a little punctuation.
pub fn label(key: KeyCode) -> &'static str {
    KEYS.iter()
        .find(|&&(k, _)| k == key)
        .map_or("?", |&(_, label)| label)
}

/// The keys that have names: those that can be bound, and the [`taken`] ones.
const KEYS: &[(KeyCode, &str)] = &[
    (KeyCode::Escape, "Escape"),
    (KeyCode::Enter, "Enter"),
    (KeyCode::NumpadEnter, "Num Enter"),
    (KeyCode::Home, "Home"),
    (KeyCode::PageUp, "Page Up"),
    (KeyCode::PageDown, "Page Down"),
    (KeyCode::BracketLeft, "Left bracket"),
    (KeyCode::BracketRight, "Right bracket"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::F3, "F3"),
    (KeyCode::F5, "F5"),
    (KeyCode::F6, "F6"),
    (KeyCode::F7, "F7"),
    (KeyCode::F8, "F8"),
    (KeyCode::KeyA, "A"),
    (KeyCode::KeyB, "B"),
    (KeyCode::KeyC, "C"),
    (KeyCode::KeyD, "D"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
    (KeyCode::KeyI, "I"),
    (KeyCode::KeyJ, "J"),
    (KeyCode::KeyK, "K"),
    (KeyCode::KeyL, "L"),
    (KeyCode::KeyM, "M"),
    (KeyCode::KeyN, "N"),
    (KeyCode::KeyO, "O"),
    (KeyCode::KeyP, "P"),
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyR, "R"),
    (KeyCode::KeyS, "S"),
    (KeyCode::KeyT, "T"),
    (KeyCode::KeyU, "U"),
    (KeyCode::KeyV, "V"),
    (KeyCode::KeyW, "W"),
    (KeyCode::KeyX, "X"),
    (KeyCode::KeyY, "Y"),
    (KeyCode::KeyZ, "Z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::ArrowUp, "Up"),
    (KeyCode::ArrowDown, "Down"),
    (KeyCode::ArrowLeft, "Left"),
    (KeyCode::ArrowRight, "Right"),
    (KeyCode::Space, "Space"),
    (KeyCode::ShiftLeft, "L Shift"),
    (KeyCode::ShiftRight, "R Shift"),
    (KeyCode::ControlLeft, "L Ctrl"),
    (KeyCode::ControlRight, "R Ctrl"),
    (KeyCode::AltLeft, "L Alt"),
    (KeyCode::AltRight, "R Alt"),
    (KeyCode::Tab, "Tab"),
    (KeyCode::CapsLock, "Caps"),
    (KeyCode::Backspace, "Backspace"),
    (KeyCode::Insert, "Insert"),
    (KeyCode::Delete, "Delete"),
    (KeyCode::End, "End"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::Semicolon, "Semicolon"),
    (KeyCode::Quote, "'"),
    (KeyCode::Backquote, "Backquote"),
    (KeyCode::Backslash, "Backslash"),
    (KeyCode::Slash, "/"),
    (KeyCode::Numpad0, "Num 0"),
    (KeyCode::Numpad1, "Num 1"),
    (KeyCode::Numpad2, "Num 2"),
    (KeyCode::Numpad3, "Num 3"),
    (KeyCode::Numpad4, "Num 4"),
    (KeyCode::Numpad5, "Num 5"),
    (KeyCode::Numpad6, "Num 6"),
    (KeyCode::Numpad7, "Num 7"),
    (KeyCode::Numpad8, "Num 8"),
    (KeyCode::Numpad9, "Num 9"),
    (KeyCode::NumpadAdd, "Num +"),
    (KeyCode::NumpadSubtract, "Num -"),
    (KeyCode::NumpadMultiply, "Num Star"),
    (KeyCode::NumpadDivide, "Num /"),
    (KeyCode::NumpadDecimal, "Num ."),
    (KeyCode::F1, "F1"),
    (KeyCode::F2, "F2"),
    (KeyCode::F4, "F4"),
    (KeyCode::F9, "F9"),
    (KeyCode::F10, "F10"),
    (KeyCode::F11, "F11"),
    (KeyCode::F12, "F12"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_a_key_takes_it_from_the_control_that_had_it() {
        let mut controls = Controls::default();
        controls.bind(Control::Gas, KeyCode::Space);
        assert_eq!(controls.keys(Control::Gas), [KeyCode::Space]);
        assert_eq!(
            controls.keys(Control::Hop),
            [KeyCode::ShiftLeft, KeyCode::ShiftRight]
        );
    }

    #[test]
    fn a_control_left_without_keys_takes_the_others_old_ones() {
        let mut controls = Controls::default();
        controls.bind(Control::Gas, KeyCode::KeyD);
        assert_eq!(controls.keys(Control::Gas), [KeyCode::KeyD]);
        assert_eq!(controls.keys(Control::Item), [KeyCode::ArrowUp]);
    }

    #[test]
    fn the_config_keeps_the_keys_by_name_and_fills_in_what_it_lacks() {
        let mut controls = Controls::default();
        controls.bind(Control::Gas, KeyCode::KeyW);
        let text = serde_json::to_string(&controls).unwrap();
        assert!(text.contains(r#""gas":["KeyW"]"#), "{text}");
        assert_eq!(serde_json::from_str::<Controls>(&text).unwrap(), controls);

        let text = r#"{"left": ["KeyA", "NoSuchKey"], "item": ["KeyR"]}"#;
        let read: Controls = serde_json::from_str(text).unwrap();
        assert_eq!(read.keys(Control::Left), [KeyCode::KeyA]);
        assert!(read.keys(Control::Item).is_empty());
        assert_eq!(read.keys(Control::Gas), [KeyCode::ArrowUp]);
    }

    #[test]
    fn the_defaults_are_bindable_and_not_taken() {
        for control in Control::ALL {
            for &key in control.defaults() {
                assert!(bindable(key), "{key:?}");
            }
        }
    }
}
