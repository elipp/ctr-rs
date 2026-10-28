//! The time and the laps of an online race, as retail draws them. TIME is at the top left: the
//! race's time since the green light, and under it a row for each lap so far ("L1" and its
//! time; the last one is the lap being driven). LAP is at the top right: the lap out of the
//! race's. The times are `m:ss:cc`. The places and colours are measured from retail screenshots.
//!
//! A free drive has neither: it has no laps to count and no end.

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;

/// Where the blocks are: both from the top, TIME from the left and LAP from the right, in lines
/// and pixels of retail's screen.
const TOP: f32 = 3.0;
const TIME_LEFT: f32 = 10.0;
const LAP_RIGHT: f32 = 20.0;

/// The lines between a label and what it labels, and between two lap rows.
const LINE_GAP: f32 = 1.0;

/// How far right of the "L1" its time starts, in pixels of retail's screen.
const LAP_TIME_X: f32 = 42.0;

/// The space above the lap rows, in lines.
const ROWS_DOWN: f32 = 1.0;

/// The race's time (`m:ss:cc`) from milliseconds.
fn clock(ms: i32) -> String {
    let ms = ms.max(0);
    format!("{}:{:02}:{:02}", ms / 60000, ms / 1000 % 60, ms % 1000 / 10)
}

/// The TIME block.
#[derive(Component)]
struct TimeBlock;

/// The LAP block.
#[derive(Component)]
struct LapBlock;

/// The race's time.
#[derive(Component)]
struct Total;

/// The lap out of the race's.
#[derive(Component)]
struct LapCount;

/// The column of lap rows, and how many it has.
#[derive(Component)]
struct LapRows(usize);

/// The "L1" of row `.0`, and its time.
#[derive(Component)]
struct RowLabel(usize);

#[derive(Component)]
struct RowTime(usize);

pub struct LapsPlugin;

impl Plugin for LapsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// A line of the game's text at the HUD's size.
fn text(text: &str, font: Font, gradient: Gradient) -> CtrText {
    CtrText::new(text, font, gradient).fixed(hud::PIXEL)
}

fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    lobby: Res<crate::lobby::Lobby>,
    mut blocks: Query<&mut Visibility, Or<(With<TimeBlock>, With<LapBlock>)>>,
    mut rows: Query<(Entity, &mut LapRows)>,
    mut texts: Query<(
        &mut CtrText,
        Has<Total>,
        Has<LapCount>,
        Option<&RowLabel>,
        Option<&RowTime>,
    )>,
) {
    if blocks.is_empty() {
        // The first frame, or of a new level (which took the nodes away).
        spawn(&mut commands);
        return;
    }
    let shown = if lobby.online.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut blocks {
        visibility.set_if_neq(shown);
    }
    if lobby.online.is_none() {
        return;
    }
    let race = &*sim.race;
    let driver = sim.driver();
    let finished = driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
    let done = &race.lap_times[sim.own];
    // The time stops when we cross the line of the last lap: then it is the laps' times.
    let total = if finished {
        done.iter().sum()
    } else {
        race.event_ms
    };
    // A kart that finished stays on its last lap.
    let laps = race.laps;
    let lap = (driver.lap_index + 1).min(laps);
    // The laps done, and the one being driven (the first counts from the green light).
    let mut times: Vec<i32> = done.clone();
    if !finished {
        times.push(race.event_ms - driver.lap_time);
    }
    if let Ok((entity, mut count)) = rows.single_mut()
        && count.0 != times.len()
    {
        count.0 = times.len();
        commands.entity(entity).despawn_related::<Children>();
        for (i, &ms) in times.iter().enumerate() {
            spawn_row(&mut commands, entity, i, &clock(ms));
        }
    }
    for (mut text, is_total, is_count, label, time) in &mut texts {
        let wanted = if is_total {
            clock(total)
        } else if is_count {
            format!("{lap}/{laps}")
        } else if let Some(RowLabel(i)) = label {
            format!("L{}", i + 1)
        } else if let Some(&RowTime(i)) = time {
            times.get(i).map_or_else(String::new, |&ms| clock(ms))
        } else {
            continue;
        };
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

/// Both blocks, hidden, with no lap rows.
fn spawn(commands: &mut Commands) {
    let block = |left: Option<f32>, right: Option<f32>, align| Node {
        position_type: PositionType::Absolute,
        left: left.map_or(Val::Auto, hud::x),
        right: right.map_or(Val::Auto, hud::x),
        top: hud::y(TOP),
        flex_direction: FlexDirection::Column,
        align_items: align,
        row_gap: hud::y(LINE_GAP),
        ..default()
    };
    commands.spawn((
        TimeBlock,
        block(Some(TIME_LEFT), None, AlignItems::FlexStart),
        Visibility::Hidden,
        Pickable::IGNORE,
        children![
            text("Time", Font::Middle, Gradient::ORANGE),
            (Total, text("0:00:00", Font::Large, Gradient::ORANGE)),
            (
                LapRows(0),
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: hud::y(LINE_GAP),
                    margin: UiRect::top(hud::y(ROWS_DOWN)),
                    ..default()
                },
            ),
        ],
    ));
    commands.spawn((
        LapBlock,
        block(None, Some(LAP_RIGHT), AlignItems::FlexEnd),
        Visibility::Hidden,
        Pickable::IGNORE,
        children![
            text("Lap", Font::Middle, Gradient::WHITE),
            (LapCount, text("", Font::Large, Gradient::WHITE)),
        ],
    ));
}

/// Row `i` of the lap rows: the red "L1" and the white time after it.
fn spawn_row(commands: &mut Commands, rows: Entity, i: usize, time: &str) {
    commands.spawn((
        Node::default(),
        ChildOf(rows),
        children![
            (
                RowLabel(i),
                text(&format!("L{}", i + 1), Font::Middle, Gradient::RED),
                // The time starts at the same place in every row.
                Node {
                    min_width: hud::x(LAP_TIME_X),
                    ..default()
                },
            ),
            (RowTime(i), text(time, Font::Middle, Gradient::WHITE)),
        ],
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_is_minutes_seconds_and_hundredths() {
        assert_eq!(clock(0), "0:00:00");
        assert_eq!(clock(49_880), "0:49:88");
        assert_eq!(clock(61_005), "1:01:00");
        assert_eq!(clock(-32), "0:00:00");
    }
}
