//! The names over the other players' karts (OnlineCTR's `DrawOverheadNames`), and the label of
//! the kart the camera follows when it is not ours.

use bevy::prelude::*;
use render::mesh::SCALE;

use crate::drive::ChaseCamera;
use crate::kart::KartSlot;
use crate::lobby::Lobby;

/// The name over the kart in a slot.
#[derive(Component)]
pub(crate) struct OverheadName(usize);

/// Who the camera follows, once we have finished.
#[derive(Component)]
pub(crate) struct WatchingLabel;

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

/// The box's corners: square. `BorderRadius::MAX` makes it a capsule, with round ends whatever
/// its size.
const CORNERS: BorderRadius = BorderRadius::ZERO;

/// The green of a name, and the box it is on, so that it reads over any track.
const COLOR: Color = Color::srgb(0.45, 1.0, 0.35);
const BACKGROUND: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);

/// Closer than this (the GTE's depth, in game units) the name is not shown: the kart is in the
/// camera's face. Further than [`TOO_FAR`], it is not shown either.
const TOO_CLOSE: f32 = 150.0;
const TOO_FAR: f32 = 3200.0;

pub(crate) fn overhead(
    mut commands: Commands,
    lobby: Res<Lobby>,
    screen: Res<crate::pixel::RenderSize>,
    cameras: Query<(&Camera, &Transform), With<ChaseCamera>>,
    karts: Query<(&KartSlot, &Transform), Without<ChaseCamera>>,
    mut labels: Query<
        (
            Entity,
            &OverheadName,
            &mut Text,
            &mut UiTransform,
            &mut Node,
            &mut Visibility,
        ),
        Without<WatchingLabel>,
    >,
    mut watching: Query<(Entity, &mut Text, &mut Visibility), With<WatchingLabel>>,
) {
    let Some(net) = &lobby.online else {
        for (entity, ..) in &labels {
            commands.entity(entity).despawn();
        }
        for (entity, ..) in &watching {
            commands.entity(entity).despawn();
        }
        return;
    };
    // A label for each player, and the one for the kart we watch.
    for slot in 0..net.names.len() {
        if labels.iter().all(|(_, label, ..)| label.0 != slot) {
            commands.spawn((
                OverheadName(slot),
                Text::new(""),
                crate::font::bold(TextFont::from_font_size(MAX_SIZE)),
                TextColor(COLOR),
                BackgroundColor(BACKGROUND),
                // Centered over the point, with its bottom edge on it.
                UiTransform::from_translation(Val2::percent(-50.0, -100.0)),
                Visibility::Hidden,
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(Val::Px(PADDING.x), Val::Px(PADDING.y)),
                    border_radius: CORNERS,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
    }
    if watching.is_empty() {
        commands.spawn((
            WatchingLabel,
            Text::new(""),
            TextFont {
                font_size: 26.0.into(),
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.9, 0.2)),
            TextLayout::justify(Justify::Center),
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(60.0),
                width: Val::Percent(100.0),
                ..default()
            },
        ));
    }
    let followed = net.watching != net.slot;
    for (_, mut text, mut visibility) in &mut watching {
        let line = if followed {
            let name = net.labels.get(net.watching).map_or("?", String::as_str);
            format!("{name}\nR1 / L1: change the camera")
        } else {
            String::new()
        };
        if text.0 != line {
            text.0 = line;
        }
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
    for (_, label, mut text, mut transform, mut node, mut visibility) in &mut labels {
        let slot = label.0;
        let name = net.labels.get(slot).map_or("", String::as_str);
        let at = karts
            .iter()
            .find(|(kart, _)| kart.0 == slot)
            .map(|(_, transform)| transform.translation + Vec3::Y * LIFT * SCALE);
        let drawn = (|| {
            if slot == net.watching || net.gone.get(slot).copied().unwrap_or(false) {
                return None;
            }
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
                if text.0 != name {
                    text.0 = name.to_string();
                }
                // The node is scaled about its middle: the bottom edge is kept on the point by
                // moving it down by the half of its height it lost.
                let scale = size / MAX_SIZE;
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
