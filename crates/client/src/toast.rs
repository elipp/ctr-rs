//! A reusable HUD event: write a [`Toast`] and its text shows for a while in a given colour,
//! stacked under any other toasts that are still up, fading out over the last moments. The stack
//! starts under the weapon slot at the top of the screen.
//!
//! A text is not stacked twice: one that is up already stays up longer instead, and one written
//! again within [`REPEAT_MS`] of the last time is dropped.
//!
//! A toast is in the game's font (`font.rs`), in capitals and a gradient of its colour, if the
//! font has every character of it (a new line starts another line of glyphs); one that has others
//! (an address, a list of options) is in Bevy's.
//!
//! ```ignore
//! fn system(mut toasts: MessageWriter<Toast>) {
//!     toasts.write(Toast::new("Kart collisions on", 2000).color(Color::srgb(1.0, 0.8, 0.2)));
//!     toasts.write(Toast::new("3", 900).size(64.0).lowered(80.0));
//! }
//! ```

use std::collections::HashMap;
use std::time::Duration;

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;

/// How long the fade at the end of a toast takes.
const FADE_MS: u64 = 300;

/// How soon after a text was shown it is dropped if written again.
const REPEAT_MS: u64 = 1000;

/// The color of a toast unless it says otherwise.
pub const DEFAULT_COLOR: Color = Color::WHITE;

/// [`Toast::error`]'s, [`Toast::success`]'s and [`Toast::warning`]'s (the console's errors are
/// `console::ERROR_COLOR`).
pub const ERROR_COLOR: Color = Color::srgb(1.0, 0.4, 0.4);
pub const SUCCESS_COLOR: Color = Color::srgb(0.4, 1.0, 0.4);
pub const WARNING_COLOR: Color = Color::srgb(1.0, 0.6, 0.3);

/// The grey behind a toast, so that it reads over any scene (the fade scales its alpha).
const BACKGROUND: Color = Color::srgba(0.1, 0.1, 0.1, 0.45);

/// The font size of a toast unless it says otherwise.
const DEFAULT_SIZE: f32 = 28.0;

/// How wide a toast's text is at most, in logical pixels (as the HUD's, which is 1024 wide): a
/// longer one wraps at spaces.
const MAX_WIDTH: f32 = 640.0;
const PADDING: Vec2 = Vec2::new(12.0, 4.0);

/// Asks the HUD to show `text` for `millis` milliseconds: white and of the default size, unless
/// the builder methods say otherwise.
#[derive(Message, Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub millis: u64,
    pub color: Color,
    /// The font size in pixels.
    pub size: f32,
    /// Extra space above the toast in pixels, which moves the ones under it as well.
    pub lower: f32,
    /// In bold: the game's font's ([`CtrText::bold`]), or Inconsolata Bold.
    pub bold: bool,
}

impl Toast {
    pub fn new(text: impl Into<String>, millis: u64) -> Toast {
        Toast {
            text: text.into(),
            millis,
            color: DEFAULT_COLOR,
            size: DEFAULT_SIZE,
            lower: 0.0,
            bold: false,
        }
    }

    /// Something asked for didn't happen.
    pub fn error(text: impl Into<String>, millis: u64) -> Toast {
        Toast::new(text, millis).color(ERROR_COLOR)
    }

    pub fn success(text: impl Into<String>, millis: u64) -> Toast {
        Toast::new(text, millis).color(SUCCESS_COLOR)
    }

    /// Something went wrong that isn't ours to fix: someone left, a long rollback.
    pub fn warning(text: impl Into<String>, millis: u64) -> Toast {
        Toast::new(text, millis).color(WARNING_COLOR)
    }

    #[expect(dead_code, reason = "no toast is bold yet")]
    pub fn bold(mut self) -> Toast {
        self.bold = true;
        self
    }

    pub fn color(mut self, color: Color) -> Toast {
        self.color = color;
        self
    }

    pub fn size(mut self, size: f32) -> Toast {
        self.size = size;
        self
    }

    #[expect(dead_code, reason = "the start lights were the only lowered toasts")]
    pub fn lowered(mut self, pixels: f32) -> Toast {
        self.lower = pixels;
        self
    }
}

/// How much darker the bottom of a toast's gradient is than its top: as the white of the menus.
const GRADIENT_BOTTOM: f32 = 0xb0 as f32 / 0xff as f32;

/// The game's gradient for a toast of `color`: the colour at the top, darker at the bottom.
pub(crate) fn gradient(color: Color) -> Gradient {
    let c = color.to_srgba();
    let channels = |scale: f32| [c.red, c.green, c.blue].map(|v| (v * scale * 255.0).round() as u8);
    Gradient {
        top: channels(1.0),
        bottom: channels(GRADIENT_BOTTOM),
    }
}

/// Whether the game's font (the middle one, which toasts are in) can write each line of `text`.
fn in_game_font(text: &str) -> bool {
    !text.trim().is_empty()
        && text.lines().all(|line| crate::font::writes(Font::Middle, line))
}

/// The size of a texel of a toast in the game's font: the HUD's, or half of it for a small toast,
/// so that a texel is still a whole number of pixels on most screens.
pub(crate) fn texel_size(size: f32) -> Vec2 {
    let halves = (size / DEFAULT_SIZE * 2.0).round().max(1.0);
    crate::hud::PIXEL * halves / 2.0
}

/// The column that the toasts are stacked in.
#[derive(Component)]
struct ToastColumn;

/// A toast that is up, with the time it goes away.
#[derive(Component)]
struct Showing {
    text: String,
    until: Duration,
    color: Color,
}

pub struct ToastPlugin;

impl Plugin for ToastPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Toast>()
            .add_systems(Update, (debug_toast, show, expire).chain());
    }
}

/// `/debug toast <text>`.
fn debug_toast(
    mut actions: MessageReader<crate::command::Action>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        if let crate::command::Action::DebugToast(text) = action {
            toasts.write(Toast::new(text.clone(), 4000));
        }
    }
}

fn column(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            ToastColumn,
            Node {
                row_gap: Val::Px(4.0),
                ..crate::bands::Band::Toasts.node()
            },
            // Toasts are not to eat clicks.
            Pickable::IGNORE,
        ))
        .id()
}

fn show(
    mut commands: Commands,
    mut toasts: MessageReader<Toast>,
    columns: Query<Entity, With<ToastColumn>>,
    mut showing: Query<&mut Showing>,
    mut last_shown: Local<HashMap<String, Duration>>,
    time: Res<Time<Real>>,
    fonts: Option<Res<crate::font::Fonts>>,
) {
    if toasts.is_empty() {
        return;
    }
    let now = time.elapsed();
    let repeat = Duration::from_millis(REPEAT_MS);
    last_shown.retain(|_, at| now.saturating_sub(*at) < repeat);
    let column = columns
        .iter()
        .next()
        .unwrap_or_else(|| column(&mut commands));
    for toast in toasts.read() {
        if last_shown.contains_key(&toast.text) {
            continue;
        }
        last_shown.insert(toast.text.clone(), now);
        let until = now + Duration::from_millis(toast.millis);
        if let Some(mut up) = showing.iter_mut().find(|s| s.text == toast.text) {
            up.until = up.until.max(until);
            continue;
        }
        let shown = commands
            .spawn((
                Showing {
                    text: toast.text.clone(),
                    until,
                    color: toast.color,
                },
                Node {
                    margin: UiRect::top(Val::Px(toast.lower)),
                    padding: UiRect::axes(Val::Px(PADDING.x), Val::Px(PADDING.y)),
                    // (Bevy's text wraps to it by itself.)
                    max_width: Val::Px(MAX_WIDTH + 2.0 * PADDING.x),
                    // (The lines of a toast in the game's font are its children.)
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(BACKGROUND),
                ChildOf(column),
            ))
            .id();
        if in_game_font(&toast.text) {
            let texels = texel_size(toast.size);
            let lines = match &fonts {
                Some(fonts) => {
                    crate::font::wrap(fonts, Font::Middle, texels, toast.bold, &toast.text, MAX_WIDTH)
                }
                None => toast.text.lines().map(str::to_string).collect(),
            };
            for line in lines.into_iter().filter(|line| !line.trim().is_empty()) {
                let text = CtrText::new(line, Font::Middle, gradient(toast.color)).fixed(texels);
                let text = if toast.bold { text.bold() } else { text };
                commands.spawn((text, ChildOf(shown)));
            }
        } else {
            let font = TextFont::from_font_size(toast.size);
            commands.spawn((
                Text::new(toast.text.clone()),
                if toast.bold { crate::font::bold(font) } else { font },
                TextColor(toast.color),
                ChildOf(shown),
            ));
        }
    }
}

fn expire(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut showing: Query<(Entity, &Showing, &mut BackgroundColor, &Children)>,
    mut texts: Query<&mut TextColor>,
    lines: Query<(), With<CtrText>>,
    descendants: Query<&Children>,
    mut glyphs: Query<&mut ImageNode>,
) {
    let now = time.elapsed();
    for (entity, showing, mut background, children) in &mut showing {
        let Some(left) = showing.until.checked_sub(now).filter(|d| !d.is_zero()) else {
            commands.entity(entity).despawn();
            continue;
        };
        let alpha = (left.as_millis() as f32 / FADE_MS as f32).min(1.0);
        background.0 = BACKGROUND.with_alpha(BACKGROUND.alpha() * alpha);
        for &child in children {
            if !lines.contains(child) {
                if let Ok(mut color) = texts.get_mut(child) {
                    color.0 = showing.color.with_alpha(showing.color.alpha() * alpha);
                }
                continue;
            }
            // The glyphs of the game's font carry the colour; the fade is their alpha. A keycap's
            // label is a black `Text` under its glyph.
            for glyph in descendants.iter_descendants(child) {
                if let Ok(mut image) = glyphs.get_mut(glyph) {
                    image.color = Color::WHITE.with_alpha(alpha);
                }
                if let Ok(mut color) = texts.get_mut(glyph) {
                    color.0 = Color::BLACK.with_alpha(alpha);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_font_takes_what_it_has() {
        assert!(in_game_font("Race starts in 5"));
        assert!(in_game_font("Lobby created! Join command copied"));
        assert!(!in_game_font("Usage: /race start | restart | stop"));
        assert!(in_game_font("Rollback: 3 ticks (x2)"));
        assert!(in_game_font("Lap #1"));
        assert!(in_game_font("Rollback: 3 ticks [x2]"));
        assert!(!in_game_font("Rollback: 3 ticks [x2 | x3]"));
        // A new line is a line of glyphs of its own.
        assert!(in_game_font("two\nlines"));
        assert!(!in_game_font("Usage:\n/race start | stop"));
        assert!(!in_game_font(" "));
        assert!(in_game_font("{select} GHOSTS"));
        assert!(in_game_font("{key:Esc} dismiss"));
        assert!(!in_game_font("{nope} dismiss"));
    }

    #[test]
    fn white_is_the_menus_white() {
        assert_eq!(gradient(Color::WHITE), Gradient::WHITE);
        assert_eq!(texel_size(DEFAULT_SIZE), crate::hud::PIXEL);
        assert_eq!(texel_size(16.0), crate::hud::PIXEL / 2.0);
    }
}
