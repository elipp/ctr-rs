//! Quick play's ready check: the match found (`Lobby::unanswered`) in a box in the middle of the
//! screen, as the welcome box, until it is answered or it runs out. (Not retail.)

use bevy::picking::events::PointerRelease;
use bevy::prelude::*;
use render::font::Font;

use crate::command::Action;
use crate::font::{CtrText, TexelSize};
use crate::lobby::{Lobby, Queue};
use crate::pad_glyph::PadGlyph;
use crate::portrait_row::DIM;
use crate::prompt::Prompt;
use crate::rect_menu::TEXT;

const TITLE: &str = "MATCH FOUND";

const ACCEPT: Prompt = Prompt::new(PadGlyph::Start, KeyCode::Enter, "accept");
/// Not answering in time is a decline too (`score::matching`): this only says so sooner.
const DECLINE: Prompt = Prompt::new(PadGlyph::Triangle, KeyCode::Escape, "decline");

/// Between the rows of the specs, above the hints, and between the two hints, in texels.
const ROW_GAP: usize = 4;
const HINT_GAP: usize = 8;
const HINTS_APART: usize = 16;

#[derive(Component)]
struct MatchBox;

pub struct MatchFoundPlugin;

impl Plugin for MatchFoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            answer
                .after(bevy::input::InputSystems)
                .before(crate::pause::input)
                .run_if(crate::drive::driving),
        )
        .add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// Under the menu the box is not up: the menu's own keys are its. Both bindings are used up,
/// so that neither opens the menu (nor Enter the line) as well.
fn answer(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut pads: Query<&mut Gamepad>,
    menu: Res<crate::pause::PauseMenu>,
    line: Res<crate::command::CommandLine>,
    lobby: Res<Lobby>,
    sounds: Option<Res<crate::sound::Sounds>>,
    mut actions: MessageWriter<Action>,
) {
    const ANSWERS: [(KeyCode, GamepadButton, bool); 2] = [
        (KeyCode::Enter, GamepadButton::Start, true),
        (KeyCode::Escape, GamepadButton::North, false),
    ];
    if lobby.unanswered().is_none() || menu.open || menu.takes_keys() || line.is_open() {
        return;
    }
    let Some(accept) = ANSWERS
        .iter()
        .find(|(key, button, _)| {
            keys.just_pressed(*key) || pads.iter().any(|p| p.just_pressed(*button))
        })
        .map(|a| a.2)
    else {
        return;
    };
    for (key, button, _) in ANSWERS {
        keys.clear_just_pressed(key);
        for mut p in &mut pads {
            p.digital_mut().clear_just_pressed(button);
        }
    }
    answered(accept, sounds, &mut actions);
}

fn answered(
    accept: bool,
    sounds: Option<Res<crate::sound::Sounds>>,
    actions: &mut MessageWriter<Action>,
) {
    if let Some(sounds) = sounds {
        sounds.menu(if accept { crate::sound::MENU_TAKE_SFX } else { crate::sound::MENU_BACK_SFX });
    }
    actions.write(Action::AnswerMatch(accept));
}

/// What the box is built from: it is built anew when any of it changes (the seconds left, every
/// second).
#[derive(Clone, PartialEq)]
struct Shown {
    specs: Vec<String>,
    left: u32,
    pad: bool,
    size: TexelSize,
}

fn show(
    mut commands: Commands,
    lobby: Res<Lobby>,
    menu: Res<crate::pause::PauseMenu>,
    last: Res<crate::last_input::LastInput>,
    size: Res<TexelSize>,
    boxes: Query<Entity, With<MatchBox>>,
    mut shown: Local<Option<Shown>>,
) {
    let wanted = match &lobby.queue {
        Some(Queue::Found { found, left, accepted: false, .. }) if !menu.open => Some(Shown {
            specs: specs(found),
            left: *left,
            pad: last.pad(),
            size: *size,
        }),
        _ => None,
    };
    if wanted == *shown {
        return;
    }
    for entity in &boxes {
        commands.entity(entity).despawn();
    }
    if let Some(wanted) = &wanted {
        spawn(&mut commands, wanted);
    }
    *shown = wanted;
}

/// (No nicks: who is in it is not what the answer should be about.)
fn specs(found: &net::Found) -> Vec<String> {
    let rules = &found.rules;
    let mut specs = vec![
        crate::maps::name(found.track).to_uppercase(),
        format!("{} LAPS, {} PLAYERS", rules.laps, found.nicks.len()),
    ];
    match rules.masked_weapons.count_ones() {
        0 => {}
        1 => specs.push("1 WEAPON LEFT OUT".into()),
        n => specs.push(format!("{n} WEAPONS LEFT OUT")),
    }
    if rules.even_odds {
        specs.push("EVEN ODDS".into());
    }
    specs
}

#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
fn hint(prompt: Prompt, tap: &str, pad: bool) -> String {
    // A tap on the hint is what answers there (`spawn`).
    #[cfg(target_arch = "wasm32")]
    if crate::web::has_touch() {
        return tap.into();
    }
    prompt.text(pad)
}

fn spawn(commands: &mut Commands, shown: &Shown) {
    let size = &shown.size;
    let left = CtrText::new(
        format!("{} S TO ANSWER", shown.left),
        Font::Middle,
        crate::toast::gradient(DIM),
    );
    let (root, rows) = crate::rect_menu::spawn(commands, size, TITLE, Some(left), |node| {
        node.left = Val::Percent(50.0);
        node.top = Val::Percent(50.0);
    });
    commands
        .entity(root)
        .insert((
            MatchBox,
            UiTransform::from_translation(Val2::percent(-50.0, -50.0)),
            // Over the HUD, under the console (as the welcome box).
            GlobalZIndex(i32::MAX - 4),
        ))
        // A click beside the hints answers nothing, but goes no further either.
        .observe(|mut release: On<PointerRelease>| release.propagate(false));
    let specs = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: size.y(ROW_GAP),
                ..default()
            },
            ChildOf(rows),
        ))
        .id();
    for spec in &shown.specs {
        commands.spawn((CtrText::new(spec.clone(), Font::Middle, TEXT), ChildOf(specs)));
    }
    let hints = commands
        .spawn((
            Node {
                justify_content: JustifyContent::SpaceBetween,
                column_gap: size.x(HINTS_APART),
                margin: UiRect::top(size.y(HINT_GAP)),
                ..default()
            },
            ChildOf(rows),
        ))
        .id();
    for (prompt, tap, accept) in [(ACCEPT, "TAP TO ACCEPT", true), (DECLINE, "TAP TO DECLINE", false)] {
        // Any pointer: a touch has no press on what it starts over, and so no click.
        commands
            .spawn((
                CtrText::new(hint(prompt, tap, shown.pad), Font::Middle, crate::toast::gradient(DIM)),
                ChildOf(hints),
            ))
            .observe(
                move |mut release: On<PointerRelease>,
                      sounds: Option<Res<crate::sound::Sounds>>,
                      mut actions: MessageWriter<Action>| {
                    release.propagate(false);
                    answered(accept, sounds, &mut actions);
                },
            );
    }
}
