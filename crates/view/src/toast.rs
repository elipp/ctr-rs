//! A reusable HUD event: write a [`Toast`] and its text shows for a while in a given colour,
//! stacked under any other toasts that are still up, fading out over the last moments.
//!
//! ```ignore
//! fn system(mut toasts: MessageWriter<Toast>) {
//!     toasts.write(Toast::new("Kart collisions on", 2000, Color::srgb(1.0, 0.8, 0.2)));
//! }
//! ```

use bevy::prelude::*;

/// How long the fade at the end of a toast takes.
const FADE_MS: u64 = 300;

/// Asks the HUD to show `text` in `color` for `millis` milliseconds.
#[derive(Message, Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub millis: u64,
    pub color: Color,
}

impl Toast {
    pub fn new(text: impl Into<String>, millis: u64, color: Color) -> Toast {
        Toast {
            text: text.into(),
            millis,
            color,
        }
    }
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
            .add_systems(Update, (show, expire).chain());
    }
}

fn column(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            ToastColumn,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(60.0),
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
        commands.spawn((
            Showing {
                until: time.elapsed() + std::time::Duration::from_millis(toast.millis),
                color: toast.color,
            },
            Text::new(toast.text.clone()),
            TextFont {
                font_size: 28.0.into(),
                ..default()
            },
            TextColor(toast.color),
            ChildOf(column),
        ));
    }
}

fn expire(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut showing: Query<(Entity, &Showing, &mut TextColor)>,
) {
    let now = time.elapsed();
    for (entity, showing, mut color) in &mut showing {
        let Some(left) = showing.until.checked_sub(now).filter(|d| !d.is_zero()) else {
            commands.entity(entity).despawn();
            continue;
        };
        let alpha = (left.as_millis() as f32 / FADE_MS as f32).min(1.0);
        color.0 = showing.color.with_alpha(showing.color.alpha() * alpha);
    }
}
