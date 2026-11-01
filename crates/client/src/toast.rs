//! A reusable HUD event: write a [`Toast`] and its text shows for a while in a given colour,
//! stacked under any other toasts that are still up, fading out over the last moments. The stack
//! starts under the weapon slot at the top of the screen.
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

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;

/// How long the fade at the end of a toast takes.
const FADE_MS: u64 = 300;

/// The color of a toast unless it says otherwise.
pub const DEFAULT_COLOR: Color = Color::WHITE;

/// The grey behind a toast, so that it reads over any scene (the fade scales its alpha).
const BACKGROUND: Color = Color::srgba(0.1, 0.1, 0.1, 0.45);

/// The pixels between the weapon slot and the first toast.
const BELOW_WEAPON: f32 = 8.0;

/// The font size of a toast unless it says otherwise.
const DEFAULT_SIZE: f32 = 28.0;

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
}

impl Toast {
    pub fn new(text: impl Into<String>, millis: u64) -> Toast {
        Toast {
            text: text.into(),
            millis,
            color: DEFAULT_COLOR,
            size: DEFAULT_SIZE,
            lower: 0.0,
        }
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

/// Whether the game's font (the middle one, which toasts are in) can write `text`: it has no
/// character the font lacks (apart from the new lines, which divide the text into the lines it
/// writes).
fn in_game_font(text: &str) -> bool {
    !text.trim().is_empty()
        && text
            .chars()
            .all(|c| c == '\n' || render::font::drawable(Font::Middle, c))
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
    until: std::time::Duration,
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
                position_type: PositionType::Absolute,
                top: Val::Px(crate::weapon::WEAPON_BOTTOM + BELOW_WEAPON),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
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
    time: Res<Time<Real>>,
) {
    if toasts.is_empty() {
        return;
    }
    let column = columns
        .iter()
        .next()
        .unwrap_or_else(|| column(&mut commands));
    for toast in toasts.read() {
        let shown = commands
            .spawn((
                Showing {
                    until: time.elapsed() + std::time::Duration::from_millis(toast.millis),
                    color: toast.color,
                },
                Node {
                    margin: UiRect::top(Val::Px(toast.lower)),
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(4.0)),
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
            for line in toast.text.lines().filter(|line| !line.trim().is_empty()) {
                commands.spawn((
                    CtrText::new(line.to_string(), Font::Middle, gradient(toast.color))
                        .fixed(texel_size(toast.size)),
                    ChildOf(shown),
                ));
            }
        } else {
            commands.spawn((
                Text::new(toast.text.clone()),
                TextFont {
                    font_size: toast.size.into(),
                    ..default()
                },
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
    lines: Query<&Children, With<CtrText>>,
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
            if let Ok(mut color) = texts.get_mut(child) {
                color.0 = showing.color.with_alpha(showing.color.alpha() * alpha);
            }
            // The glyphs of the game's font carry the colour; the fade is their alpha.
            for &glyph in lines.get(child).into_iter().flatten() {
                if let Ok(mut image) = glyphs.get_mut(glyph) {
                    image.color = Color::WHITE.with_alpha(alpha);
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
        assert!(!in_game_font("Rollback: 3 ticks [x2]"));
        // A new line is a line of glyphs of its own.
        assert!(in_game_font("two\nlines"));
        assert!(!in_game_font("Usage:\n/race start | stop"));
        assert!(!in_game_font(" "));
    }

    #[test]
    fn white_is_the_menus_white() {
        assert_eq!(gradient(Color::WHITE), Gradient::WHITE);
        assert_eq!(texel_size(DEFAULT_SIZE), crate::hud::PIXEL);
        assert_eq!(texel_size(16.0), crate::hud::PIXEL / 2.0);
    }
}
