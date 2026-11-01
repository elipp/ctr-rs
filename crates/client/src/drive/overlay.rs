//! The text on the screen of a race: wumpa, checkpoint split (a toast), boost reserves, readouts.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::wumpa_icon::{COUNTER_GAP, COUNTER_X, MIDDLE_Y};

#[derive(Component)]
pub(super) struct Hud;

/// `/debug hud`: the development text of the race, kept in the config (`Config::debug_hud`).
pub(crate) fn debug_text_command(
    mut actions: MessageReader<crate::command::Action>,
    mut config: ResMut<crate::config::Config>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    for action in actions.read() {
        if let crate::command::Action::DebugHud(setting) = action {
            config.debug_hud = setting.unwrap_or(!config.debug_hud);
            if let Err(why) = config.save() {
                toasts.write(crate::toast::Toast::new(why, 3000).color(Color::srgb(1.0, 0.4, 0.4)));
            }
        }
    }
}

/// The wumpa count: the number after the "x".
#[derive(Component)]
pub(crate) struct WumpaHud;

/// How long a checkpoint time's toast stays up, in milliseconds.
const SPLIT_SHOW_MS: u64 = 3000;

/// The width of the reserves' text, in pixels: "Reserves: " and 5 digits at the default font size.
const RESERVES_WIDTH: f32 = 150.0;

/// The boost reserves, as a number at the bottom right.
#[derive(Component)]
pub(super) struct ReservesHud;

/// "REC" or "DEMO" while a demo is being recorded or played.
#[derive(Component)]
pub(super) struct DemoHud;

pub(crate) fn spawn_hud(mut commands: Commands) {
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

/// Toasts the time of each checkpoint we pass (see `race::Race::last_split`) against the first
/// kart to pass it: green if we were the first there, else red and how far behind the first.
/// `announced` is the number of the last checkpoint toasted, so that a rollback that takes a
/// pass back and plays it again does not toast it twice.
pub(super) fn split_toast(
    sim: Res<crate::props::Sim>,
    lobby: Res<crate::lobby::Lobby>,
    mut announced: Local<usize>,
    mut toasts: MessageWriter<crate::toast::Toast>,
) {
    let Some((number, time, behind)) = lobby.online.as_ref().and(sim.race.last_split(sim.own))
    else {
        *announced = 0;
        return;
    };
    if number <= *announced {
        return;
    }
    *announced = number;
    let toast = if behind == 0 {
        crate::toast::Toast::new(split_time(time), SPLIT_SHOW_MS).color(Color::srgb(0.4, 1.0, 0.4))
    } else {
        crate::toast::Toast::new(format!("+{}", split_time(behind)), SPLIT_SHOW_MS)
            .color(Color::srgb(1.0, 0.3, 0.3))
    };
    toasts.write(toast);
}

/// The boost reserves, up to 10000, with the development text.
pub(super) fn reserves_hud(
    sim: Res<crate::props::Sim>,
    config: Res<crate::config::Config>,
    mut text: Query<&mut Text, With<ReservesHud>>,
) {
    for mut text in &mut text {
        text.0 = if config.debug_hud {
            format!("Reserves: {}", sim.driver().reserves)
        } else {
            String::new()
        };
    }
}

/// The development text (see `Config::debug_hud`): the frame rate and the time a frame takes
/// (smoothed over the last few frames), then where the kart is and what it does, the name of our
/// lobby if it is public, and the version of the build.
pub(super) fn hud(
    sim: Res<crate::props::Sim>,
    lobby: Res<crate::lobby::Lobby>,
    diagnostics: Res<DiagnosticsStore>,
    config: Res<crate::config::Config>,
    mut text: Query<&mut Text, With<Hud>>,
) {
    if !config.debug_hud {
        for mut text in &mut text {
            if !text.0.is_empty() {
                text.0.clear();
            }
        }
        return;
    }
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
        text.0 = format!(
            "{fps}\nspeed {:5}  {:?}{}{}{}{}{}\nbuild {}",
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
            match lobby.public_name() {
                Some(name) => format!("\nlobby \"{name}\" (public)"),
                None => String::new(),
            },
            crate::version::VERSION,
        );
    }
}
