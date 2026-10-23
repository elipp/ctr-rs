//! A bar for the HUD: a track with a fill that is anchored at its right edge and shrinks
//! towards it as the value drops, in a colour that follows the value along a ramp
//! (green, yellow, red, say).

use bevy::prelude::*;

/// What the bar shows: `value` is 0 (empty) to 1 (full).
#[derive(Component)]
pub struct Bar {
    pub value: f32,
    /// Colour stops as `(value, colour)`, in ascending order of value; the fill is the blend of the
    /// two stops its value lies between.
    pub ramp: Vec<(f32, Color)>,
}

impl Bar {
    pub fn new(value: f32, ramp: Vec<(f32, Color)>) -> Bar {
        Bar { value, ramp }
    }

    /// Red below, yellow in the middle, green above.
    pub fn traffic_light() -> Vec<(f32, Color)> {
        vec![
            (0.0, Color::srgb(0.9, 0.1, 0.1)),
            (0.5, Color::srgb(0.95, 0.85, 0.1)),
            (1.0, Color::srgb(0.2, 0.85, 0.2)),
        ]
    }

    /// The colour of the fill at the current value.
    pub fn color(&self) -> Color {
        let value = self.value.clamp(0.0, 1.0);
        let Some(&(_, first)) = self.ramp.first() else {
            return Color::WHITE;
        };
        let mut previous = (self.ramp[0].0, first);
        for &(at, color) in &self.ramp {
            if value <= at {
                let span = at - previous.0;
                let t = if span > 0.0 {
                    (value - previous.0) / span
                } else {
                    1.0
                };
                return previous.1.mix(&color, t);
            }
            previous = (at, color);
        }
        previous.1
    }
}

/// The part of a bar that is filled.
#[derive(Component)]
pub struct BarFill;

const TRACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.55);
const BORDER: Color = Color::srgb(0.85, 0.85, 0.85);

/// Spawns a bar. `node` places and sizes it (a border is added).
pub fn spawn(commands: &mut Commands, node: Node, bar: Bar) -> Entity {
    let color = bar.color();
    let entity = commands.spawn((
        bar,
        Node {
            border: UiRect::all(Val::Px(2.0)),
            ..node
        },
        BackgroundColor(TRACK),
        BorderColor::all(BORDER),
    ));
    let bar = entity.id();
    commands.spawn((
        BarFill,
        ChildOf(bar),
        Node {
            // Anchored right: the bar empties towards the right edge.
            position_type: PositionType::Absolute,
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            height: Val::Percent(100.0),
            width: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(color),
    ));
    bar
}

/// Sizes and colours the fills of the bars whose value changed.
pub fn update(
    bars: Query<(&Bar, &Children), Changed<Bar>>,
    mut fills: Query<(&mut Node, &mut BackgroundColor), With<BarFill>>,
) {
    for (bar, children) in &bars {
        for &child in children {
            if let Ok((mut node, mut background)) = fills.get_mut(child) {
                node.width = Val::Percent(bar.value.clamp(0.0, 1.0) * 100.0);
                background.0 = bar.color();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(value: f32) -> Srgba {
        Bar::new(value, Bar::traffic_light()).color().to_srgba()
    }

    #[test]
    fn the_ramp_blends_between_its_stops() {
        assert_eq!(color(0.0), Srgba::new(0.9, 0.1, 0.1, 1.0));
        assert_eq!(color(1.0), Srgba::new(0.2, 0.85, 0.2, 1.0));
        let middle = color(0.5);
        assert!((middle.red - 0.95).abs() < 1e-6 && (middle.green - 0.85).abs() < 1e-6);
        let quarter = color(0.25);
        assert!((quarter.red - 0.925).abs() < 1e-6);
        // Out of range values stay on the ends.
        assert_eq!(color(-1.0), color(0.0));
        assert_eq!(color(2.0), color(1.0));
    }
}
