//! Watching a stored solo run, the best's or the server record's: a solo race of its own
//! ([`CurrentRace::replay`]) whose kart is the run's racer and plays the run's pads, so that it
//! looks as the run did. What is of an attempt (the best, the ghosts, the submit, the ceremony)
//! goes by [`CurrentRace::attempt`], which a replay is not. Clicking the HUD's label of a run
//! watches it; Up and Down change the speed, Item goes back to an attempt. A banner at the bottom
//! says whose run it is, the speed and the keys, for as long as it plays.
//!
//! [`CurrentRace::replay`]: crate::race_on::CurrentRace::replay
//! [`CurrentRace::attempt`]: crate::race_on::CurrentRace::attempt

use bevy::picking::events::{PointerClick, PointerOut, PointerOver};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use race::Pad;
use race::solo::Run;
use render::font::{Font, Gradient};

use crate::command::Action;
use crate::controls::{Control, Controls};
use crate::font::CtrText;
// (The game clock's speeds, which the F3 panel's are too: a slow one stays smooth, as what is
// drawn between ticks goes by that clock.)
use crate::fx::{NORMAL, SPEEDS};
use crate::ghost::Source;
use crate::race_on::RaceOn;

pub struct Replay {
    pub run: Run,
    source: Source,
    /// Whose run it is (`world_record::our_name` if ours), and when it was set if known.
    who: String,
    date: Option<String>,
    /// Into [`SPEEDS`].
    speed: usize,
}

impl Replay {
    pub fn new(run: Run, source: Source, who: String, date: Option<String>) -> Replay {
        Replay {
            run,
            source,
            who,
            date,
            speed: NORMAL,
        }
    }

    /// The kart's pad of `tick`: the run's, and none once it has run out.
    pub fn pad(&self, tick: u32) -> Pad {
        // (A pad has no taps: `previous_held` is of no matter.)
        (self.run.demo.input(tick, 0)).map_or_else(Pad::default, |input| Pad::of_input(&input))
    }

    pub fn speed(&self) -> f32 {
        SPEEDS[self.speed]
    }
}

/// The banner's line of the speed and the keys.
fn keys_line(speed: f32, controls: &Controls) -> String {
    format!(
        "{speed}x   {} / {}: speed   {}: back",
        controls.describe(Control::Gas),
        controls.describe(Control::Brake),
        controls.describe(Control::Item),
    )
}

/// The banner of the replay of `.0` by `.1`: another replay's is taken away for its own.
#[derive(Component)]
struct Banner(Source, String);

#[derive(Component)]
struct BannerKeys;

/// Where the banner is: over the bottom edge, as the spectator's label (`names.rs`).
const BANNER_BOTTOM: f32 = 60.0;
const BANNER_BACKGROUND: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);
const BANNER_PADDING: UiRect = UiRect::axes(Val::Px(14.0), Val::Px(8.0));

/// The button that watches a run when clicked.
#[derive(Component, Clone, Copy)]
pub struct Watchable(pub Source);

/// The button, next to the run's time (`laps.rs`).
pub fn button(source: Source) -> impl Bundle {
    (
        Watchable(source),
        crate::icons::outlined(
            crate::icons::VIDEO_CAMERA,
            BUTTON_SIZE,
            BUTTON_COLOR,
            Node {
                margin: UiRect::left(Val::Px(BUTTON_GAP)),
                ..default()
            },
        ),
    )
}

/// The button's size and its gap from the time, in pixels.
const BUTTON_SIZE: f32 = 27.0;
const BUTTON_GAP: f32 = 8.0;
/// A bright orange, and a yellow under the pointer.
const BUTTON_COLOR: Color = Color::srgb_u8(0xff, 0xb0, 0x48);
const BUTTON_HOVER: Color = Color::srgb_u8(0xff, 0xec, 0x90);

pub struct ReplayPlugin;

impl Plugin for ReplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(click)
            .add_observer(hover)
            .add_observer(unhover)
            .add_systems(Update, (keys, banner).chain().run_if(crate::drive::driving));
    }
}

fn click(
    click: On<PointerClick>,
    watchables: Query<&Watchable>,
    mut actions: MessageWriter<Action>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    if let Ok(&Watchable(source)) = watchables.get(click.event_target()) {
        actions.write(Action::Watch(source));
    }
}

fn hover(over: On<PointerOver>, buttons: Query<&Children, With<Watchable>>, faces: Faces) {
    recolor(over.event_target(), BUTTON_HOVER, buttons, faces);
}

fn unhover(out: On<PointerOut>, buttons: Query<&Children, With<Watchable>>, faces: Faces) {
    recolor(out.event_target(), BUTTON_COLOR, buttons, faces);
}

type Faces<'w, 's> = Query<'w, 's, &'static mut TextColor, With<crate::icons::Face>>;

fn recolor(button: Entity, color: Color, buttons: Query<&Children, With<Watchable>>, mut faces: Faces) {
    let Ok(children) = buttons.get(button) else {
        return;
    };
    for &child in children {
        if let Ok(mut face) = faces.get_mut(child) {
            face.0 = color;
        }
    }
}

/// The taps of a menu of the player's own (`drive::Controller::menu`): Up and Down are the gas
/// and brake keys, the D-pad, or the left stick pushed; back is Item, or Circle.
fn keys(
    controller: crate::drive::Controller,
    drive: Res<crate::drive::Drive>,
    menu: Res<crate::pause::PauseMenu>,
    mut race_on: ResMut<RaceOn>,
    mut actions: MessageWriter<Action>,
    mut stick: Local<IVec2>,
) {
    if menu.open || menu.takes_keys() {
        return;
    }
    let Some(view) = drive.views().first() else {
        return;
    };
    // (Read even with no replay, so that a stick already pushed as one starts is no tap.)
    let taps = controller.menu(&view.device, &[], &mut stick);
    let Some(replay) = race_on.get_mut().and_then(|race| race.replay.as_mut()) else {
        return;
    };
    if taps.back {
        actions.write(Action::RestartRace);
        return;
    }
    let step = taps.up as isize - taps.down as isize;
    replay.speed = (replay.speed)
        .saturating_add_signed(step)
        .min(SPEEDS.len() - 1);
}

fn banner(
    mut commands: Commands,
    race_on: Res<RaceOn>,
    config: Res<crate::config::Config>,
    banners: Query<(Entity, &Banner)>,
    mut keys_text: Query<&mut CtrText, With<BannerKeys>>,
) {
    let replay = race_on.get().and_then(|race| race.replay.as_ref());
    let mut have = false;
    for (entity, banner) in &banners {
        let same = replay.is_some_and(|r| r.source == banner.0 && r.who == banner.1);
        if same && !have {
            have = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    let Some(replay) = replay else {
        return;
    };
    let line = keys_line(replay.speed(), &config.controls);
    if have {
        for mut text in &mut keys_text {
            if text.text != line {
                text.text = line.clone();
            }
        }
        return;
    }
    let who = match &replay.date {
        Some(date) => format!("{} {date}", replay.who),
        None => replay.who.clone(),
    };
    // (Across the window, to center the box in it.)
    commands.spawn((
        Banner(replay.source, replay.who.clone()),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(BANNER_BOTTOM),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        children![(
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                padding: BANNER_PADDING,
                ..default()
            },
            BackgroundColor(BANNER_BACKGROUND),
            Pickable::IGNORE,
            // (A nick is the game font's chars: `score::nick_char`.)
            children![
                (
                    Node {
                        column_gap: Val::Px(10.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                    children![
                        (
                            CtrText::new("Watching", Font::Middle, Gradient::YELLOW),
                            Pickable::IGNORE,
                        ),
                        (
                            CtrText::new(who, Font::Middle, Gradient::YELLOW),
                            Pickable::IGNORE,
                        ),
                    ],
                ),
                (
                    BannerKeys,
                    CtrText::new(line, Font::Middle, Gradient::WHITE),
                    Pickable::IGNORE,
                ),
            ],
        )],
    ));
}
