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

/// How far above the kart's position the name floats, in the game's units.
const LIFT: f32 = 25.0;

const FONT_SIZE: f32 = 18.0;

/// What the name is on, so that it reads over any track.
const BACKGROUND: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// The depth (the GTE's, in game units) a name shows in full up to, and the depths past which it
/// is cut to the length given, and beyond the last of which it is not shown. Retail cuts it from
/// 190 (6 letters) down to 2 letters at 906; ours are longer, and read from further.
const FULL_UNTIL: f32 = 1200.0;
const CUTS: [(f32, usize); 3] = [(1800.0, 8), (2400.0, 6), (3200.0, 4)];

/// Closer than this the name is not shown: the kart is in the camera's face.
const TOO_CLOSE: f32 = 150.0;

/// How much of a name shows at a depth of `z` game units from the camera: all of it near, fewer
/// letters the further it is, and none too close or too far.
fn shown(name: &str, z: f32) -> String {
    if z < TOO_CLOSE {
        return String::new();
    }
    if z < FULL_UNTIL {
        return name.to_string();
    }
    match CUTS.iter().find(|&&(until, _)| z < until) {
        Some(&(_, len)) => name.chars().take(len).collect(),
        None => String::new(),
    }
}

pub(crate) fn overhead(
    mut commands: Commands,
    lobby: Res<Lobby>,
    cameras: Query<(&Camera, &Transform), With<ChaseCamera>>,
    karts: Query<(&KartSlot, &Transform), Without<ChaseCamera>>,
    mut labels: Query<
        (Entity, &OverheadName, &mut Text, &mut Node, &mut Visibility),
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
                TextFont {
                    font_size: FONT_SIZE.into(),
                    ..default()
                },
                BackgroundColor(BACKGROUND),
                // Centered over the point, with its bottom edge on it.
                UiTransform::from_translation(Val2::percent(-50.0, -100.0)),
                Visibility::Hidden,
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                    ..default()
                },
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
            let name = net.names.get(net.watching).map_or("?", String::as_str);
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
    for (_, label, mut text, mut node, mut visibility) in &mut labels {
        let slot = label.0;
        let name = net.names.get(slot).map_or("", String::as_str);
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
            let line = shown(name, depth);
            if line.is_empty() {
                return None;
            }
            let screen = camera.world_to_viewport(&global, at).ok()?;
            Some((line, screen))
        })();
        match drawn {
            Some((line, screen)) => {
                if text.0 != line {
                    text.0 = line;
                }
                node.left = Val::Px(screen.x);
                node.top = Val::Px(screen.y);
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}
