//! The welcome box in the middle of the screen, once the first level is up, and again on
//! `/welcome`: the pause menu's panel (`rect_menu`) around a legend of the bindings worth knowing.
//! (Not retail.)

use bevy::picking::events::PointerRelease;
use bevy::prelude::*;
use render::font::Font;

use crate::command::Action;
use crate::font::{CtrText, TexelSize};
use crate::pad_glyph::PadGlyph;
use crate::portrait_row::DIM;
use crate::prompt::Prompt;
use crate::rect_menu::TEXT;

const TITLE: &str = "WELCOME TO CTR-RS!";

/// The line only opens from the keyboard (`command::open`), so it's keys on a pad too.
const COMMANDS: &str = "{key:Enter} {key:/}";
const MENU: Prompt = Prompt::new(PadGlyph::Start, KeyCode::Escape, "menu");
/// Not in the legend but in the corner: it is about the box, not the game.
const DISMISS: Prompt = Prompt::new(PadGlyph::Triangle, KeyCode::Space, "dismiss");

/// Between the legend's two columns and its rows (room for the keycaps, which stick out of the
/// line), and above the dismiss hint, in texels.
const COLUMN_GAP: usize = 8;
const ROW_GAP: usize = 4;
const HINT_GAP: usize = 8;

#[derive(Resource, Default)]
struct Welcome {
    up: bool,
    /// Up once by itself: the first level is.
    greeted: bool,
}

#[derive(Component)]
struct WelcomeBox;

pub struct WelcomePlugin;

impl Plugin for WelcomePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Welcome>()
            .add_systems(
                PreUpdate,
                dismiss
                    .after(bevy::input::InputSystems)
                    .before(crate::pause::input)
                    .run_if(crate::drive::driving),
            )
            .add_systems(Update, (open, show).chain().run_if(crate::drive::driving));
    }
}

/// Behind the loading screen it would go unseen, hence the wait for it to go.
fn open(
    loading: Option<Res<crate::loading::Loading>>,
    mut actions: MessageReader<Action>,
    mut welcome: ResMut<Welcome>,
) {
    if actions.read().any(|a| matches!(a, Action::Welcome)) {
        welcome.up = true;
    }
    if !welcome.greeted && loading.is_none() {
        welcome.greeted = true;
        welcome.up = true;
    }
}

/// Every binding in the box takes it down. Only [`DISMISS`]'s is used up: the others are left to
/// do what the legend says they do, there and then.
fn dismiss(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut pads: Query<&mut Gamepad>,
    menu: Res<crate::pause::PauseMenu>,
    line: Res<crate::command::CommandLine>,
    sounds: Option<Res<crate::sound::Sounds>>,
    mut welcome: ResMut<Welcome>,
) {
    const KEYS: [KeyCode; 1] = [KeyCode::Space];
    const BUTTONS: [GamepadButton; 1] = [GamepadButton::North];
    if !welcome.up || menu.open {
        return;
    }
    // (The line opens on a click or a tap on the console too.)
    let passed = line.is_open()
        || [KeyCode::Enter, KeyCode::Escape].iter().any(|k| keys.just_pressed(*k))
        || pads.iter().any(|p| p.just_pressed(GamepadButton::Start));
    if passed {
        welcome.up = false;
        return;
    }
    let key = KEYS.iter().any(|k| keys.just_pressed(*k));
    let pad = (pads.iter()).any(|p| BUTTONS.iter().any(|b| p.just_pressed(*b)));
    if !key && !pad {
        return;
    }
    welcome.up = false;
    for k in KEYS {
        keys.clear_just_pressed(k);
    }
    for mut p in &mut pads {
        for b in BUTTONS {
            p.digital_mut().clear_just_pressed(b);
        }
    }
    if let Some(sounds) = sounds {
        sounds.menu(crate::sound::MENU_BACK_SFX);
    }
}

/// (Under the pause menu, which would cover it anyway, it goes until the menu does.)
fn show(
    mut commands: Commands,
    welcome: Res<Welcome>,
    menu: Res<crate::pause::PauseMenu>,
    last: Res<crate::last_input::LastInput>,
    size: Res<TexelSize>,
    boxes: Query<Entity, With<WelcomeBox>>,
    mut shown: Local<Option<(bool, TexelSize)>>,
) {
    // The panel is in pixels of the texel size it was built for, and the legend's buttons follow
    // the last input.
    let wanted = (welcome.up && !menu.open).then(|| (last.pad(), *size));
    if wanted == *shown {
        return;
    }
    *shown = wanted;
    for entity in &boxes {
        commands.entity(entity).despawn();
    }
    if let Some((pad, size)) = wanted {
        spawn(&mut commands, &size, pad);
    }
}

/// What to press, and what it does.
fn legend(pad: bool) -> Vec<(String, &'static str)> {
    #[cfg(target_arch = "wasm32")]
    if crate::web::has_touch() {
        // The page's touch controls (`web/play.html`): a tap on the console opens the line, and
        // the Menu button is top right.
        return vec![
            ("TAP CONSOLE, /".into(), "COMMANDS"),
            ("MENU (TOP RIGHT)".into(), "MENU"),
        ];
    }
    vec![(COMMANDS.into(), "COMMANDS"), (MENU.button(pad), "MENU")]
}

fn dismiss_hint(pad: bool) -> String {
    #[cfg(target_arch = "wasm32")]
    if crate::web::has_touch() {
        return "TAP TO DISMISS".into();
    }
    DISMISS.text(pad)
}

fn spawn(commands: &mut Commands, size: &TexelSize, pad: bool) {
    let version = CtrText::new(
        crate::version::VERSION.to_uppercase(),
        Font::Middle,
        crate::toast::gradient(DIM),
    );
    let (root, rows) = crate::rect_menu::spawn(commands, size, TITLE, Some(version), |node| {
        node.left = Val::Percent(50.0);
        node.top = Val::Percent(50.0);
    });
    commands
        .entity(root)
        .insert((
            WelcomeBox,
            UiTransform::from_translation(Val2::percent(-50.0, -50.0)),
            // Over the HUD, under the console (as the scoreboard).
            GlobalZIndex(i32::MAX - 4),
        ))
        // Any pointer: a touch has no press on what it starts over, and so no click.
        .observe(|mut release: On<PointerRelease>, mut welcome: ResMut<Welcome>| {
            release.propagate(false);
            welcome.up = false;
        });
    let grid = commands
        .spawn((
            Node {
                display: Display::Grid,
                grid_template_columns: vec![GridTrack::auto(), GridTrack::auto()],
                column_gap: size.x(COLUMN_GAP),
                row_gap: size.y(ROW_GAP),
                align_self: AlignSelf::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            ChildOf(rows),
        ))
        .id();
    for (keys, does) in legend(pad) {
        commands.spawn((
            CtrText::new(keys, Font::Middle, render::font::Gradient::WHITE),
            Node {
                justify_self: JustifySelf::End,
                ..default()
            },
            ChildOf(grid),
        ));
        commands.spawn((CtrText::new(does, Font::Middle, TEXT), ChildOf(grid)));
    }
    commands.spawn((
        CtrText::new(dismiss_hint(pad), Font::Middle, crate::toast::gradient(DIM)),
        Node {
            align_self: AlignSelf::FlexEnd,
            margin: UiRect::top(size.y(HINT_GAP)),
            ..default()
        },
        ChildOf(rows),
    ));
}
