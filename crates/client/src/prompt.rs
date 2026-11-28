//! A button and what it does, as a [`CtrText`](crate::font::CtrText)'s markup: the pad's glyph or
//! the key's cap, whichever the reader uses (`last_input`). Every "press this to do that" line
//! goes through here, so they all read alike. (Not retail.)

use bevy::prelude::*;

use crate::pad_glyph::PadGlyph;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Prompt {
    pad: PadGlyph,
    /// None: no key does it (a control left unbound), so the pad's glyph shows for the keys too.
    key: Option<KeyCode>,
    does: &'static str,
}

impl Prompt {
    pub(crate) const fn new(pad: PadGlyph, key: KeyCode, does: &'static str) -> Prompt {
        Prompt {
            pad,
            key: Some(key),
            does,
        }
    }

    /// For a rebindable control: its first key.
    pub(crate) fn bound(pad: PadGlyph, keys: &[KeyCode], does: &'static str) -> Prompt {
        Prompt {
            pad,
            key: keys.first().copied(),
            does,
        }
    }

    /// The button alone: `pad` says whether the reader is on a pad (`LastInput::pad`/`pad_of`).
    pub(crate) fn button(self, pad: bool) -> String {
        match self.key.filter(|_| !pad) {
            Some(key) => format!("{{key:{}}}", crate::controls::label(key)),
            None => format!("{{{}}}", self.pad.name()),
        }
    }

    /// (Not uppercased whole: a keycap keeps its label's case. The game's font has capitals only,
    /// but Bevy's, which a toast may fall back to, doesn't.)
    pub(crate) fn text(self, pad: bool) -> String {
        format!("{} {}", self.button(pad), self.does.to_uppercase())
    }
}
