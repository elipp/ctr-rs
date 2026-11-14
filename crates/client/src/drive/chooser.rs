//! The racer menu of each local player: Select on a pad (or the "Racer menu" key, C for the first
//! player and B for the keyboard's second set) opens a small list of racers in that player's part
//! of the window, in the style of the pause menu. It shows [`ROWS`] names at a time, scrolls as the
//! pick moves and loops round the 16 racers. Up and Down (Gas and Brake on the keys, the D-pad or
//! the stick) move the pick, Cross takes it (on the keys, Hop: they have no Cross) and Item
//! (Circle) or Select again leaves it.
//!
//! While it is open the player's kart gets a neutral pad (`drive_tick`): it coasts, as the first
//! player's does online while the pause menu is open.
//!
//! The first player's pick goes the way `/character` does (the lobby may refuse it, and says so).
//! The others' karts are swapped here, offline only (the change is not in the rollback's
//! snapshots). In a lobby that is not racing a pick goes to the session instead, and the kart
//! changes when the host's roster says so (`sync`): the host may have fixed the racers. The picks
//! are kept ([`Chosen`]), so that a new level or `/players` gives the players the racers they
//! picked.

use bevy::prelude::*;
use race::KartSpec;
use render::font::{Font, Gradient};

use super::input::Controller;
use super::{Drive, Extra, HudCamera};
use crate::command::Action;
use crate::font::{CtrText, Fonts, TexelSize};
use crate::hud_view::HudView;
use crate::kart::{KartBuilder, KartSlot};
use crate::lobby::Lobby;
use crate::pause::{BORDER, PANEL, PauseMenu};
use crate::props::Sim;
use crate::sound::{MENU_BACK_SFX, MENU_MOVE_SFX, MENU_TAKE_SFX, Sounds};
use crate::{CHARACTERS, player_layer};

/// The panel's padding left and right, in texels.
const PADDING: usize = 12;

/// How many names the menu shows at once.
const ROWS: usize = 4;

/// What one local player's menu keeps.
#[derive(Default)]
pub(crate) struct Chooser {
    pub open: bool,
    /// The racer picked.
    pick: usize,
    /// The racer on the first row: the list is a ring, and this is where it is cut.
    top: usize,
    /// Which way the left stick was pushed the last frame, to tap with it.
    stick: i32,
}

impl Chooser {
    /// The menu opens on `current`, a row down so that the name before it shows.
    fn open_on(&mut self, current: usize) {
        self.open = true;
        self.pick = current;
        self.top = (current + CHARACTERS.len() - 1) % CHARACTERS.len();
    }

    /// The pick moves `step` places round the ring, and the list scrolls if it left the rows.
    fn step(&mut self, step: i32) {
        let n = CHARACTERS.len();
        self.pick = (self.pick as i32 + step).rem_euclid(n as i32) as usize;
        if (self.pick + n - self.top) % n >= ROWS {
            self.top = if step > 0 {
                (self.pick + n - (ROWS - 1)) % n
            } else {
                self.pick
            };
        }
    }

    /// The racer on row `row`.
    fn racer(&self, row: usize) -> usize {
        (self.top + row) % CHARACTERS.len()
    }
}

/// The racers the local players picked, by view (the first is picked by `/character`, and is
/// [`Drive::character`]). A player that never picked has the one that follows the first's.
#[derive(Resource, Default)]
pub struct Chosen(Vec<Option<usize>>, Vec<Option<usize>>);

impl Chosen {
    fn set(&mut self, view: usize, character: usize) {
        if self.0.len() <= view {
            self.0.resize(view + 1, None);
        }
        self.0[view] = Some(character);
    }

    /// Picked in a lobby and sent, but not yet the host's word (it is in the roster once it is).
    fn want(&mut self, view: usize, character: usize) {
        if self.1.len() <= view {
            self.1.resize(view + 1, None);
        }
        self.1[view] = Some(character);
    }

    fn wanted(&self, view: usize) -> Option<usize> {
        self.1.get(view).copied().flatten()
    }

    /// The racer of local player `view`, when the first drives `first`.
    pub fn character(&self, view: usize, first: usize) -> usize {
        self.0
            .get(view)
            .copied()
            .flatten()
            .unwrap_or((first + view) % CHARACTERS.len())
    }
}

/// Opens and closes the menus, moves the picks, and takes them.
#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    controller: Controller,
    mut drive: ResMut<Drive>,
    mut sim: ResMut<Sim>,
    mut chosen: ResMut<Chosen>,
    lobby: Res<Lobby>,
    pause: Res<PauseMenu>,
    sounds: Option<Res<Sounds>>,
    karts: Query<(Entity, &KartSlot), With<Extra>>,
    mut builder: KartBuilder,
    mut actions: MessageWriter<Action>,
) {
    // The pause menu has the controls (and the pads' buttons are its own then).
    if pause.open || pause.capturing.is_some() {
        return;
    }
    let sound = |id| {
        if let Some(sounds) = &sounds {
            sounds.menu(id);
        }
    };
    let locked = lobby.online.is_some() || lobby.characters_locked();
    let taken: Vec<Entity> = drive.views.iter().filter_map(|v| v.device.pad()).collect();
    let mut picked = Vec::new();
    for view in 0..drive.views.len() {
        let device = drive.views[view].device.clone();
        let mut stick = drive.views[view].chooser.stick;
        let taps = controller.menu(&device, &taken, &mut stick);
        let current = if view == 0 {
            drive.character
        } else {
            drive.views[view].spec.map_or(0, |spec| spec.character)
        };
        let chooser = &mut drive.views[view].chooser;
        chooser.stick = stick;
        if !chooser.open {
            // (No menu in a race or a cup: the racers are fixed then.)
            if !taps.toggle || locked {
                continue;
            }
            chooser.open_on(current);
            sound(MENU_TAKE_SFX);
            continue;
        }
        let step = taps.down as i32 - taps.up as i32;
        if step != 0 {
            chooser.step(step);
            sound(MENU_MOVE_SFX);
        }
        if taps.take {
            chooser.open = false;
            picked.push((view, chooser.pick));
            sound(MENU_TAKE_SFX);
        } else if taps.back || taps.toggle {
            chooser.open = false;
            sound(MENU_BACK_SFX);
        }
    }
    for (view, character) in picked {
        if view == 0 {
            if character != drive.character {
                actions.write(Action::Character(character as u32));
            }
            continue;
        }
        if lobby.is_open() {
            // The kart changes when the host says so (`sync`): it may have fixed the racers.
            chosen.want(view, character);
            let acked = lobby.own_extras().unwrap_or_default();
            let extras: Vec<u8> = (1..drive.views.len())
                .map(|v| {
                    chosen
                        .wanted(v)
                        .or_else(|| acked.get(v - 1).map(|&c| c as usize))
                        .unwrap_or_else(|| chosen.character(v, drive.character))
                        as u8
                })
                .collect();
            lobby.set_extras(&extras);
        } else {
            chosen.set(view, character);
            swap(view, character, &mut drive, &mut sim, &karts, &mut builder);
        }
    }
}

/// Takes the host's word on our extras' racers: in a lobby, their karts follow the roster (our
/// entry of it), not the picks, which the host may have turned down.
pub(super) fn sync(
    mut drive: ResMut<Drive>,
    mut sim: ResMut<Sim>,
    mut chosen: ResMut<Chosen>,
    lobby: Res<Lobby>,
    karts: Query<(Entity, &KartSlot), With<Extra>>,
    mut builder: KartBuilder,
) {
    if !lobby.is_open() || lobby.characters_locked() {
        // (A pick that never came back is dropped.)
        chosen.1.clear();
        return;
    }
    // (The race has its own karts, from the start.)
    if lobby.online.is_some() {
        return;
    }
    let Some(acked) = lobby.own_extras() else {
        return;
    };
    for view in 1..drive.views.len() {
        let Some(&character) = acked.get(view - 1) else {
            continue;
        };
        let character = character as usize;
        if chosen.wanted(view) == Some(character) {
            chosen.1[view] = None;
        }
        if drive.views[view].spec.is_some_and(|s| s.character != character) {
            chosen.set(view, character);
            swap(view, character, &mut drive, &mut sim, &karts, &mut builder);
        }
    }
}

/// The kart of `view` is `character` from now on.
fn swap(
    view: usize,
    character: usize,
    drive: &mut Drive,
    sim: &mut Sim,
    karts: &Query<(Entity, &KartSlot), With<Extra>>,
    builder: &mut KartBuilder,
) {
    {
        let slot = drive.views[view].slot;
        if drive.views[view].spec.is_some_and(|s| s.character == character)
            || slot >= sim.race.karts.len()
        {
            return;
        }
        // The kart of the race is the new racer from where it is, as `scene::swap_character` does
        // for ours (and so offline only: the change is not in the rollback's snapshots).
        let (model, wheel_icons) = crate::load_racer(character);
        let frames = crate::scene::anim_num_frames(&model);
        let driver = &mut sim.race.state_mut().karts[slot].driver;
        driver.character = character as u8;
        driver.stats =
            vehicle::stats::Stats::for_class(vehicle::stats::CHARACTER_ENGINES[character]);
        driver.anim_num_frames = frames;
        // The new model's animation may be shorter than the frame the old one was on.
        let last = (vehicle::frame::num_anim_frames(driver, driver.anim_index) - 1).max(0);
        driver.anim_frame = driver.anim_frame.clamp(0, last as i16);
        drive.views[view].spec = Some(KartSpec {
            character,
            anim_num_frames: frames,
            bot: false,
            line: None,
        });
        for (entity, kart) in karts {
            if kart.0 == slot {
                builder.commands.entity(entity).try_despawn();
            }
        }
        let kart = builder.spawn_in(
            &model,
            &wheel_icons,
            Transform::default(),
            false,
            player_layer(view),
            Some((slot, &sim.race.karts[slot])),
        );
        builder.commands.entity(kart).insert(Extra);
    }
}

/// The top node of a player's menu.
#[derive(Component)]
pub(super) struct ChooserUi;

/// A name of a menu: row `row` of the one of player `view`.
#[derive(Component)]
pub(super) struct ChooserRow {
    view: usize,
    row: usize,
}

/// What row `row` of `chooser` says: the racer, in white when it is the pick.
fn look(chooser: &Chooser, row: usize) -> (&'static str, Gradient) {
    let racer = chooser.racer(row);
    let gradient = if racer == chooser.pick {
        Gradient::WHITE
    } else {
        Gradient::ORANGE
    };
    (CHARACTERS[racer], gradient)
}

/// Puts each open menu up (in the player's part of the window, which `hud_view::target` sees to)
/// and takes it down when it closes, and has the rows show the racers round the pick.
pub(super) fn show(
    mut commands: Commands,
    drive: Res<Drive>,
    size: Res<TexelSize>,
    fonts: Res<Fonts>,
    cameras: Query<(Entity, &HudCamera)>,
    roots: Query<(Entity, &HudView), With<ChooserUi>>,
    mut rows: Query<(&ChooserRow, &mut CtrText)>,
) {
    let views = drive.views();
    let mut have = Vec::new();
    for (entity, &HudView(view)) in &roots {
        let open = views.get(view).is_some_and(|v| v.chooser.open);
        if !open || size.is_changed() {
            commands.entity(entity).despawn();
        } else {
            have.push(view);
        }
    }
    if !size.is_changed() {
        for (view, local) in views.iter().enumerate() {
            if !local.chooser.open || have.contains(&view) {
                continue;
            }
            // Where the player's part of the window is, from the first frame: with several
            // players the camera is looked up here, not left to `hud_view::target`, which would
            // show the menu in the middle of the window for a frame.
            let camera = if views.len() > 1 {
                match cameras.iter().find(|(_, c)| c.0 == view) {
                    Some((camera, _)) => Some(camera),
                    // (Not up yet: the next frame.)
                    None => continue,
                }
            } else {
                None
            };
            let widest = CHARACTERS
                .iter()
                .map(|name| fonts.atlas(Font::Middle).layout(name).width)
                .max()
                .unwrap_or(0);
            let root = spawn(&mut commands, view, &local.chooser, &size, widest);
            if let Some(camera) = camera {
                commands.entity(root).insert(UiTargetCamera(camera));
            }
        }
    }
    for (row, mut text) in &mut rows {
        let Some(local) = views.get(row.view) else {
            continue;
        };
        let (name, gradient) = look(&local.chooser, row.row);
        if text.text != name {
            text.text = name.to_string();
        }
        if text.gradient != gradient {
            text.gradient = gradient;
        }
    }
}

/// The menu of player `view`: a panel in the middle of the player's part, with its title and the
/// rows.
fn spawn(
    commands: &mut Commands,
    view: usize,
    chooser: &Chooser,
    size: &TexelSize,
    widest: usize,
) -> Entity {
    // Under the pause menu.
    let root = commands
        .spawn((
            ChooserUi,
            HudView(view),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(i32::MAX - 2),
        ))
        .id();
    let panel = commands
        .spawn((
            ChildOf(root),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                // As wide as the widest name (and its padding and border), so that the panel
                // does not change its width as the names go by.
                width: size.x(widest + 2 * PADDING + 2),
                padding: UiRect::axes(size.x(PADDING), size.y(4)),
                border: UiRect::axes(size.x(1), size.y(1)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(BORDER),
        ))
        .id();
    commands.spawn((
        ChildOf(panel),
        CtrText::new(format!("Player {}", view + 1), Font::Middle, Gradient::ORANGE),
        Node {
            margin: UiRect::bottom(size.y(3)),
            ..default()
        },
    ));
    for row in 0..ROWS {
        let (name, gradient) = look(chooser, row);
        commands.spawn((
            ChildOf(panel),
            ChooserRow { view, row },
            CtrText::new(name, Font::Middle, gradient),
            Node {
                margin: UiRect::vertical(size.y(2)),
                ..default()
            },
        ));
    }
    root
}
