//! The text on the screen of a race: wumpa, checkpoint split, boost reserves, readouts.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::wumpa_icon::{COUNTER_GAP, COUNTER_X, MIDDLE_Y};

#[derive(Component)]
pub(super) struct Hud;

/// Whether the development text of the race shows: the speed under the frame rate,
/// and the boost reserves (`/debug hud`). The frame rate always shows, and the viewer's text does.
#[derive(Resource)]
pub(crate) struct DebugText(pub(crate) bool);

impl Default for DebugText {
    fn default() -> Self {
        DebugText(true)
    }
}

/// `/debug hud`.
pub(crate) fn debug_text_command(
    mut actions: MessageReader<crate::command::Action>,
    mut shown: ResMut<DebugText>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    for action in actions.read() {
        if let crate::command::Action::DebugHud(setting) = action {
            shown.0 = setting.unwrap_or(!shown.0);
            toasts.write(crate::toast::Toast::new(
                format!("Debug text {}", if shown.0 { "on" } else { "off" }),
                2000,
            ));
        }
    }
}

/// The wumpa count: the number after the "x".
#[derive(Component)]
pub(crate) struct WumpaHud;

/// The time at the checkpoint just passed: green if we were the first there, else red and how
/// far behind the first.
#[derive(Component)]
pub(super) struct SplitHud;

/// How long a checkpoint time stays up, in milliseconds of the race. (The last one, at the finish
/// line, stays.)
const SPLIT_SHOW_MS: i32 = 3000;

/// The width of the reserves' text, in pixels: "Reserves: " and 5 digits at the default font size.
const RESERVES_WIDTH: f32 = 150.0;

/// The boost reserves, as a number at the bottom right.
#[derive(Component)]
pub(super) struct ReservesHud;

/// "REC" or "DEMO" while a demo is being recorded or played.
#[derive(Component)]
pub(super) struct DemoHud;

pub(crate) fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        SplitHud,
        Text::new(""),
        TextFont {
            font_size: 30.0.into(),
            ..default()
        },
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(70.0),
            width: Val::Percent(100.0),
            ..default()
        },
    ));
    // The counter: a small "x" and the count in the big orange digits, level with the middle of
    // the fruit, right of it. The "x" is orange too, and its middle is the digits' (retail draws
    // it 4 lines down from their top, 8 tall, on digits 16 tall).
    let height = Font::Large.line_height() as f32;
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            margin: UiRect::left(hud::x(COUNTER_X)),
            top: hud::y(MIDDLE_Y - height / 2.0),
            align_items: AlignItems::Center,
            column_gap: hud::x(COUNTER_GAP as f32),
            ..default()
        },
        children![
            CtrText::new("x", Font::Middle, Gradient::ORANGE).fixed(hud::PIXEL),
            (
                WumpaHud,
                CtrText::new("0", Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
            ),
        ],
    ));
    commands.spawn((
        ReservesHud,
        Text::new(""),
        // Wide enough for the label and 5 digits, so that the label stays put as the number grows.
        TextLayout::justify(Justify::Left),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            right: Val::Px(20.0),
            width: Val::Px(RESERVES_WIDTH),
            ..default()
        },
    ));
    // The readouts for development go where the HUD of a race leaves room: the bottom left is
    // the place's and the top right the lap's. These are left of the minimap and the slide meter
    // (the widest map reaches 100 pixels of retail's screen in from the right edge), and under
    // the lap.
    commands.spawn((
        Hud,
        Text::new(""),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            right: hud::x(108.0),
            ..default()
        },
    ));
    commands.spawn((
        DemoHud,
        Text::new(""),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            top: hud::y(32.0),
            right: Val::Px(8.0),
            ..default()
        },
    ));
}

/// The wumpa count.
pub(super) fn wumpa_hud(
    sim: Res<crate::props::Sim>,
    mut counts: Query<&mut CtrText, With<WumpaHud>>,
) {
    let count = sim.driver().num_wumpas.to_string();
    for mut text in &mut counts {
        // A changed text puts its glyphs up again: only a new count changes it.
        if text.text != count {
            text.text = count.clone();
        }
    }
}

/// `m:ss.cc`, as a checkpoint time is told.
fn split_time(ms: i32) -> String {
    let ms = ms.max(0);
    format!("{}:{:02}.{:02}", ms / 60000, ms / 1000 % 60, ms % 1000 / 10)
}

/// Shows the time of the checkpoint we passed last (see `race::Race::last_split`) against the
/// first kart to pass it.
pub(super) fn split_hud(
    sim: Res<crate::props::Sim>,
    lobby: Res<crate::lobby::Lobby>,
    mut text: Query<(&mut Text, &mut TextColor), With<SplitHud>>,
) {
    let race = &*sim.race;
    let finished = sim.driver().actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
    let shown = lobby
        .online
        .as_ref()
        .and(race.last_split(sim.own))
        .filter(|&(_, time, _)| finished || race.event_ms - time < SPLIT_SHOW_MS);
    let (line, color) = match shown {
        Some((_, time, 0)) => (split_time(time), Color::srgb(0.4, 1.0, 0.4)),
        Some((_, _, behind)) => (
            format!("+{}", split_time(behind)),
            Color::srgb(1.0, 0.3, 0.3),
        ),
        None => (String::new(), Color::WHITE),
    };
    for (mut text, mut text_color) in &mut text {
        if text.0 != line {
            text.0 = line.clone();
        }
        text_color.0 = color;
    }
}

/// The boost reserves, always, up to 10000.
pub(super) fn reserves_hud(
    sim: Res<crate::props::Sim>,
    shown: Res<DebugText>,
    mut text: Query<&mut Text, With<ReservesHud>>,
) {
    for mut text in &mut text {
        text.0 = if shown.0 {
            format!("Reserves: {}", sim.driver().reserves)
        } else {
            String::new()
        };
    }
}

/// The bottom-left lines: the frame rate and the time a frame takes (smoothed over the last few
/// frames), then where the kart is and what it does, and the version of the build.
pub(super) fn hud(
    sim: Res<crate::props::Sim>,
    diagnostics: Res<DiagnosticsStore>,
    shown: Res<DebugText>,
    mut text: Query<&mut Text, With<Hud>>,
) {
    let d = sim.driver();
    let grounded = d.actions_flag_set & vehicle::actions::TOUCH_GROUND != 0;
    let read = |path| diagnostics.get(path).and_then(|d| d.smoothed());
    let fps = match (
        read(&FrameTimeDiagnosticsPlugin::FPS),
        read(&FrameTimeDiagnosticsPlugin::FRAME_TIME),
    ) {
        (Some(fps), Some(ms)) => format!("{fps:5.1} fps  {ms:5.2} ms"),
        _ => String::new(),
    };
    for mut text in &mut text {
        if !shown.0 {
            text.0 = fps.clone();
            continue;
        }
        text.0 = format!(
            "{fps}\nspeed {:5}  {:?}{}{}{}{}\nbuild {}",
            d.speed_approx,
            d.phys_state,
            if grounded { "" } else { "  airborne" },
            if d.wall_rub_timer != 0 { "  wall" } else { "" },
            if d.phys_state == vehicle::PhysState::PowerSlide {
                format!(
                    "  meter {} boosts {}",
                    d.turbo_meter_room_left, d.num_boosts_success
                )
            } else {
                String::new()
            },
            match d.last_drift_end {
                Some(why) => format!("  last drift ended: {why:?}"),
                None => String::new(),
            },
            crate::version::VERSION,
        );
    }
}
