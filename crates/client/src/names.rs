//! The names over the other players' karts (OnlineCTR's `DrawOverheadNames`) and over a solo
//! run's ghosts, and the label of the kart the camera follows when it is not ours.

use bevy::prelude::*;
use render::font::{Font, Gradient};
use render::mesh::SCALE;

use crate::controls::Control;
use crate::drive::ChaseCamera;
use crate::font::CtrText;
use crate::ghost::{GhostKart, Source};
use crate::kart::KartSlot;
use crate::pad_glyph::PadGlyph;
use crate::prompt::Prompt;

/// Whose kart a name is over.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plate {
    Slot(usize),
    Ghost(Source),
}

impl Plate {
    fn color(self) -> Color {
        match self {
            Plate::Slot(_) => COLOR,
            Plate::Ghost(Source::Best) => BEST_COLOR,
            Plate::Ghost(Source::World) => WORLD_COLOR,
        }
    }
}

/// The box of the name over a kart. Its children are the name's text: the [`NameOutline`]s
/// first, under the [`NameText`].
#[derive(Component)]
pub(crate) struct OverheadName(Plate);

/// The name itself, in its green.
#[derive(Component)]
pub(crate) struct NameText;

/// A copy of the name in the outline's colour, behind it and moved [`OUTLINE_WIDTH`] that way:
/// together, the copies outline its letters. (Bevy's text has no outline of its own.)
#[derive(Component)]
pub(crate) struct NameOutline(Vec2);

/// Who the camera follows, once we have finished: its [`WatchingLine`]s.
#[derive(Component)]
pub(crate) struct WatchingLabel;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchingLine {
    Name,
    /// What steps the camera: R1 and L1 (`drive::input`'s Hop and Hop 2).
    Hint,
}

/// How far above the kart's position the name floats, in the game's units, clear of the kart and
/// its driver (retail's 25 is down on the driver's head). The box's bottom edge is a further
/// [`GAP`] pixels over that.
const LIFT: f32 = 60.0;
const GAP: f32 = 4.0;

/// How tall a name is in the world, in the game's units: it is drawn as tall as that comes to on
/// the screen at the kart's depth, between [`MIN_SIZE`] and [`MAX_SIZE`] pixels. Text is laid out
/// at whole pixel sizes only, so a name is laid out at the biggest, and the node scaled down to its
/// size: smaller than it was laid out keeps it sharp, where bigger would blur it.
const NAME_HEIGHT: f32 = 13.0;
const MIN_SIZE: f32 = 14.0;
const MAX_SIZE: f32 = 30.0;

/// The padding of the box around a name, across and down, in pixels.
const PADDING: Vec2 = Vec2::new(9.0, 0.0);

/// The green of a name.
const COLOR: Color = Color::srgb(0.45, 1.0, 0.35);

/// A ghost's name, in the tint of its kart (`ghost::Fade`).
const BEST_COLOR: Color = Color::srgb(0.85, 0.85, 0.85);
const WORLD_COLOR: Color = Color::srgb(1.0, 0.85, 0.45);

/// The outline of a name's letters, so that it reads over any track: how thick, in the window's pixels whatever the name's size, and
/// its colour. It is made of copies of the name moved that far each way.
pub(crate) const OUTLINE_WIDTH: f32 = 2.0;
pub(crate) const OUTLINE_COLOR: Color = Color::BLACK;
pub(crate) const OUTLINE_DIRECTIONS: [Vec2; 8] = {
    let d = std::f32::consts::FRAC_1_SQRT_2;
    [
        Vec2::new(1.0, 0.0),
        Vec2::new(-1.0, 0.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(0.0, -1.0),
        Vec2::new(d, d),
        Vec2::new(d, -d),
        Vec2::new(-d, d),
        Vec2::new(-d, -d),
    ]
};

/// Closer than this (the GTE's depth, in game units) the name is not shown: the kart is in the
/// camera's face. Further than [`TOO_FAR`], it is not shown either.
const TOO_CLOSE: f32 = 150.0;
const TOO_FAR: f32 = 3200.0;

pub(crate) fn overhead(
    mut commands: Commands,
    race_on: Res<crate::race_on::RaceOn>,
    world_record: Res<crate::world_record::WorldRecord>,
    config: Res<crate::config::Config>,
    drive: Option<Res<crate::drive::Drive>>,
    screen: Res<crate::pixel::RenderSize>,
    cameras: Query<(&Camera, &Transform), With<ChaseCamera>>,
    karts: Query<(&KartSlot, &Transform), Without<ChaseCamera>>,
    ghosts: Query<
        (&GhostKart, &Transform, &Visibility),
        (Without<ChaseCamera>, Without<OverheadName>, Without<WatchingLabel>),
    >,
    mut labels: Query<
        (
            Entity,
            &OverheadName,
            Option<&Children>,
            &mut UiTransform,
            &mut Node,
            &mut Visibility,
        ),
        Without<WatchingLabel>,
    >,
    mut texts: Query<
        (&mut Text, Option<&NameOutline>, &mut Node),
        Without<OverheadName>,
    >,
    mut watching: Query<(Entity, &mut Visibility), With<WatchingLabel>>,
    mut watching_lines: Query<(&WatchingLine, &mut CtrText)>,
    last: Res<crate::last_input::LastInput>,
) {
    // (Not in a split screen: the plates are placed by the one camera.)
    let split = drive.is_some_and(|d| d.views().len() > 1);
    let Some(race) = race_on.get().filter(|_| !split) else {
        for (entity, ..) in &labels {
            commands.entity(entity).despawn();
        }
        for (entity, ..) in &watching {
            commands.entity(entity).despawn();
        }
        return;
    };
    // A label for each kart and each ghost, and the one for the kart we watch.
    let plates = (0..race.karts()).map(Plate::Slot).chain(
        (race.ghosts.iter()).map(|(source, _)| Plate::Ghost(source)),
    );
    for plate in plates {
        if labels.iter().all(|(_, label, ..)| label.0 != plate) {
            let font = crate::font::bold(TextFont::from_font_size(MAX_SIZE));
            commands
                .spawn((
                    OverheadName(plate),
                    // Centered over the point, with its bottom edge on it.
                    UiTransform::from_translation(Val2::percent(-50.0, -100.0)),
                    Visibility::Hidden,
                    Node {
                        position_type: PositionType::Absolute,
                        padding: UiRect::axes(Val::Px(PADDING.x), Val::Px(PADDING.y)),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|label| {
                    // (Drawn in this order: the outline under the name.)
                    for direction in OUTLINE_DIRECTIONS {
                        label.spawn((
                            NameOutline(direction),
                            Text::new(""),
                            font.clone(),
                            TextColor(OUTLINE_COLOR),
                            // Over the name, out of the layout: the name sizes the box.
                            Node {
                                position_type: PositionType::Absolute,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    }
                    label.spawn((
                        NameText,
                        Text::new(""),
                        font,
                        TextColor(plate.color()),
                        Pickable::IGNORE,
                    ));
                });
        }
    }
    if watching.is_empty() {
        commands
            .spawn((
                WatchingLabel,
                Visibility::Hidden,
                crate::bands::Band::Watching.node(),
                Pickable::IGNORE,
            ))
            .with_children(|label| {
                for line in [WatchingLine::Name, WatchingLine::Hint] {
                    label.spawn((
                        line,
                        CtrText::new("", Font::Middle, Gradient::YELLOW),
                        Pickable::IGNORE,
                    ));
                }
            });
    }
    let followed = race.watching != race.slot;
    let name = if followed {
        race.kart_label(race.watching).to_string()
    } else {
        String::new()
    };
    let pad = last.pad();
    let next = Prompt::bound(PadGlyph::R, config.controls.keys(Control::Hop), "");
    let back = Prompt::bound(PadGlyph::L, config.controls.keys(Control::Hop2), "");
    let hint = format!("{} / {} CAMERA", back.button(pad), next.button(pad));
    for (line, mut text) in &mut watching_lines {
        let wanted = match line {
            WatchingLine::Name => &name,
            WatchingLine::Hint => &hint,
        };
        if text.text != *wanted {
            text.text = wanted.clone();
        }
    }
    for (_, mut visibility) in &mut watching {
        *visibility = if followed {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let global = GlobalTransform::from(*camera_transform);
    let forward = camera_transform.forward();
    let up = camera_transform.up();
    // Where a point of the scene is on the window (the camera draws into the picture of the
    // scene, which may be smaller than the window).
    let on_window = |at: Vec3| {
        camera
            .world_to_viewport(&global, at)
            .ok()
            .map(|at| screen.to_window(at))
    };
    // (A ghost is drawn only if it is picked, `kart::show`, which runs before this.)
    let ghost_at = |source: Source| {
        ghosts
            .iter()
            .find(|(kart, _, visibility)| kart.source == source && **visibility != Visibility::Hidden)
            .map(|(_, transform, _)| transform.translation)
    };
    for (_, label, children, mut transform, mut node, mut visibility) in &mut labels {
        let (name, at) = match label.0 {
            Plate::Slot(slot) if slot == race.watching || race.gone(slot) => (String::new(), None),
            Plate::Slot(slot) => (
                race.kart_label(slot).to_string(),
                karts
                    .iter()
                    .find(|(kart, _)| kart.0 == slot)
                    .map(|(_, transform)| transform.translation),
            ),
            // (Our record has no ghost of its own: `WorldRecord::ghost`.)
            Plate::Ghost(Source::Best) => {
                let what = if world_record.is_best(race) { "SR" } else { "PB" };
                let who = crate::world_record::our_name(&config.nickname);
                (format!("{what} - {who}"), ghost_at(Source::Best))
            }
            Plate::Ghost(Source::World) => (
                world_record.of(race).map_or_else(String::new, |(entry, ..)| {
                    format!("SR - {}", crate::world_record::short_name(entry))
                }),
                ghost_at(Source::World),
            ),
        };
        let name = name.as_str();
        let at = at.map(|at| at + Vec3::Y * LIFT * SCALE);
        let drawn = (|| {
            let at = at?;
            let depth = forward.dot(at - camera_transform.translation) / SCALE;
            if name.is_empty() || !(TOO_CLOSE..TOO_FAR).contains(&depth) {
                return None;
            }
            let top = on_window(at + *up * NAME_HEIGHT * SCALE)?;
            let at = on_window(at)?;
            Some((at, at.distance(top).clamp(MIN_SIZE, MAX_SIZE)))
        })();
        match drawn {
            Some((at, size)) => {
                let scale = size / MAX_SIZE;
                for &child in children.into_iter().flatten() {
                    let Ok((mut text, outline, mut node)) = texts.get_mut(child) else {
                        continue;
                    };
                    if text.0 != name {
                        text.0 = name.to_string();
                    }
                    // On the name, where the box's padding puts it, moved the outline's width
                    // (in the window's pixels: undone by the box's scale) its way.
                    if let Some(NameOutline(direction)) = outline {
                        let offset = PADDING + *direction * OUTLINE_WIDTH / scale;
                        node.left = Val::Px(offset.x);
                        node.top = Val::Px(offset.y);
                    }
                }
                // The box is scaled about its middle: the bottom edge is kept on the point by
                // moving it down by the half of its height it lost.
                transform.scale = Vec2::splat(scale);
                transform.translation = Val2::percent(-50.0, -100.0 + (1.0 - scale) * 50.0);
                node.left = Val::Px(at.x);
                node.top = Val::Px(at.y - GAP);
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}
