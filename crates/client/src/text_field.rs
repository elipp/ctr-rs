//! Editing state for a menu's one-line text field. It owns only the text being typed: the owner
//! decides where a `Done` goes, and drawing it is the menu's (`pause.rs`, `Kind::Field`).

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};

#[derive(Clone, Debug)]
pub struct TextField {
    pub text: String,
    before: String,
    /// In chars, not bytes.
    max: usize,
    allow: fn(char) -> bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    Typing,
    Done(String),
    /// Carries the text from before editing started.
    Cancelled(String),
}

impl TextField {
    pub fn new(text: impl Into<String>, max: usize, allow: fn(char) -> bool) -> TextField {
        let text: String = text.into();
        TextField {
            before: text.clone(),
            text,
            max,
            allow,
        }
    }

    /// Repeats are taken on purpose (held Backspace). Chars come from `event.text`, not the key
    /// code, so layouts and shifted chars work.
    pub fn key(&mut self, event: &KeyboardInput) -> Typed {
        if event.state != ButtonState::Pressed {
            return Typed::Typing;
        }
        match &event.logical_key {
            Key::Enter => return Typed::Done(self.text.clone()),
            Key::Escape => return Typed::Cancelled(self.before.clone()),
            Key::Backspace => {
                self.text.pop();
                return Typed::Typing;
            }
            _ => {}
        }
        for c in event.text.iter().flat_map(|text| text.chars()) {
            if (self.allow)(c) && self.text.chars().count() < self.max {
                self.text.push(c);
            }
        }
        Typed::Typing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::keyboard::KeyCode;
    use bevy::prelude::Entity;

    fn press(key: Key, text: Option<&str>) -> KeyboardInput {
        KeyboardInput {
            key_code: KeyCode::KeyA,
            logical_key: key,
            state: ButtonState::Pressed,
            text: text.map(Into::into),
            repeat: false,
            window: Entity::PLACEHOLDER,
        }
    }

    fn typed(field: &mut TextField, text: &str) {
        for c in text.chars() {
            let s = c.to_string();
            field.key(&press(Key::Character(s.as_str().into()), Some(&s)));
        }
    }

    #[test]
    fn it_takes_what_it_allows_up_to_its_length() {
        let mut field = TextField::new("", 4, |c| c.is_ascii_alphanumeric());
        typed(&mut field, "a b!cde");
        assert_eq!(field.text, "abcd");
        field.key(&press(Key::Backspace, None));
        assert_eq!(field.text, "abc");
        assert_eq!(field.key(&press(Key::Enter, None)), Typed::Done("abc".into()));
    }

    #[test]
    fn escape_gives_back_what_was_there() {
        let mut field = TextField::new("old", 10, |_| true);
        typed(&mut field, "new");
        assert_eq!(
            field.key(&press(Key::Escape, None)),
            Typed::Cancelled("old".into())
        );
    }
}
