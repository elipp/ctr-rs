//! The speedometer and jump meter, shown instead of the minimap (`UI_RenderFrame`,
//! `UI_JumpMeter_Draw`). As in retail, the choice outlasts the level.

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use render::mesh::Atlas;
use render::speedometer::{DIGIT_SIZE, HEIGHT, LEFT, TOP, WIDTH, dial, digits, needle};
use vehicle::calc::map_to_range;

use crate::config::Config;
use crate::controls::Control;
use crate::hud;
use crate::hud_view::{HudView, HudViews};
use crate::hud::{atlas_image, pack_icons};
use crate::pause::PauseMenu;

/// Whether the speedometer is showing instead of the minimap (`HudAndDebugFlags & 8`).
#[derive(Resource, Default)]
pub struct ShowSpeedometer(pub bool);

/// `hudStructPtr[UI_HUD_SLOT_JUMP_METER]`
const JUMP_X: i32 = 491;
const JUMP_Y: i32 = 206;
/// `UI_JUMP_METER_BAR_W`, `_BAR_H`, `_MAX`
const BAR_WIDTH: i32 = 12;
const BAR_HEIGHT: i32 = 38;
const JUMP_MAX: i32 = 0x960;
/// `UI_JUMP_METER_HANG_TIME_SMALL`, `_MEDIUM`, `_LARGE`
const HANG_SMALL: i32 = 640;
const HANG_MEDIUM: i32 = 960;
const HANG_LARGE: i32 = 1440;
/// `UI_JUMP_METER_NUMBER_Y_OFFSET`, `_BOX_X_OFFSET`, `_BOX_W`, `_BOX_H`
const NUMBER_Y: i32 = -45;
const NUMBER_X: i32 = -20;
const NUMBER_WIDTH: i32 = 34;
const NUMBER_HEIGHT: i32 = 10;
/// Where the digits are from the jump meter's slot: seconds, tenths and hundredths.
const DIGITS_X: [i32; 3] = [-16, -4, 4];
const DIGITS_Y: i32 = NUMBER_Y + 2;

/// `UI_JUMP_METER_BAR_EMPTY_COLOR`
const BAR_EMPTY: Color = Color::srgb_u8(0x80, 0x80, 0x80);

/// `UI_JUMP_METER_BAR_BLUE`, `_GREEN`, `_YELLOW`, `_RED`
fn bar_color(jump_meter: i32) -> Color {
    match jump_meter {
        m if m < HANG_SMALL => Color::srgb_u8(0, 0, 0xff),
        m if m < HANG_MEDIUM => Color::srgb_u8(0, 0xff, 0),
        m if m < HANG_LARGE => Color::srgb_u8(0xff, 0xff, 0),
        _ => Color::srgb_u8(0xff, 0, 0),
    }
}

/// The hang time as retail writes it: a "second" of the meter is 960 of its milliseconds.
fn jump_digits(jump_meter: i32) -> [usize; 3] {
    const SECOND: i32 = 0x3c0;
    const TENTH: i32 = 0x60;
    let seconds = jump_meter / SECOND;
    let rest = jump_meter - seconds * SECOND;
    let tenths = rest / TENTH;
    let hundredths = (rest - tenths * TENTH) * 100 / SECOND;
    [seconds, tenths, hundredths].map(|d| d.clamp(0, 9) as usize)
}

/// `ANG(305.20)`, `ANG(213.75)`, `ANG(157.5)`
const LOW_ANGLE: i32 = 3472;
const MIDDLE_ANGLE: i32 = 2432;
const HIGH_ANGLE: i32 = 1792;

/// The needle's angle (`UI_DrawSpeedNeedle`): low end to middle up to `accel_speed`, then middle
/// to high end up to `accel_speed + sacred_fire_speed`.
fn needle_angle(speed: i32, accel_speed: i32, sacred_fire_speed: i32) -> i32 {
    // Retail's scale: only its rounding matters here.
    let scale = |n: i32| n * 108000 / 64000;
    let (accel_int, speed_int) = (accel_speed >> 8, speed >> 8);
    let (from, to, angle_from, angle_to) = if accel_speed < speed {
        (
            scale(accel_int),
            scale((accel_speed + sacred_fire_speed) >> 8),
            MIDDLE_ANGLE,
            HIGH_ANGLE,
        )
    } else {
        (0, scale(accel_int), LOW_ANGLE, MIDDLE_ANGLE)
    };
    map_to_range(scale(speed_int), from, to, angle_from, angle_to)
}

/// The needle's corners (`render::speedometer::Needle`).
#[derive(ShaderType, Debug, Clone)]
struct NeedleCorners {
    tip: Vec2,
    tail: Vec2,
    bright_side: Vec2,
    dark_side: Vec2,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct NeedleMaterial {
    #[uniform(0)]
    corners: NeedleCorners,
}

impl NeedleMaterial {
    fn at(angle: i32) -> NeedleMaterial {
        let n = needle(angle);
        NeedleMaterial {
            corners: NeedleCorners {
                tip: n.tip.into(),
                tail: n.tail.into(),
                bright_side: n.bright_side.into(),
                dark_side: n.dark_side.into(),
            },
        }
    }
}

impl UiMaterial for NeedleMaterial {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_PKG_NAME"), "/speedometer.wgsl").into()
    }
}

/// Made once, and kept across levels, which take the nodes away. (The needle's material is each
/// view's own.)
#[derive(Resource)]
struct Art {
    dial: Handle<Image>,
    digits: Vec<Handle<Image>>,
}

/// The nodes that show only with the speedometer.
#[derive(Component)]
struct Part;

#[derive(Component)]
struct BarFill;

#[derive(Component)]
struct DigitSlot(usize);

pub struct SpeedometerPlugin;

impl Plugin for SpeedometerPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "speedometer.wgsl");
        app.add_plugins(UiMaterialPlugin::<NeedleMaterial>::default())
            .init_resource::<ShowSpeedometer>()
            .add_systems(Update, (toggle, show).chain().run_if(crate::drive::driving));
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    pads: crate::drive::Pads,
    config: Res<Config>,
    menu: Res<PauseMenu>,
    sim: Res<crate::props::Sim>,
    mut showing: ResMut<ShowSpeedometer>,
) {
    if menu.open || sim.race.tick < sim.race.intro_ticks {
        return;
    }
    let key = keys.any_just_pressed(config.controls.keys(Control::Speedometer).iter().copied());
    if key || pads.iter().any(|p| p.just_pressed(GamepadButton::North)) {
        showing.0 = !showing.0;
    }
}

/// Filtered, unlike [`atlas_image`]'s: finer than retail's pixels, it is only one to one on a 2x
/// display.
fn smooth_image(atlas: Atlas, images: &mut Assets<Image>) -> Handle<Image> {
    let mut image = Image::new(
        Extent3d {
            width: atlas.width as u32,
            height: atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    images.add(image)
}

fn show(
    mut commands: Commands,
    hud: HudViews,
    showing: Res<ShowSpeedometer>,
    textures: Res<crate::Textures>,
    art: Option<Res<Art>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<NeedleMaterial>>,
    mut parts: Query<(&HudView, &mut Visibility), With<Part>>,
    needles: Query<(&HudView, &MaterialNode<NeedleMaterial>)>,
    mut fills: Query<(&HudView, &mut Node, &mut BackgroundColor), With<BarFill>>,
    mut slots: Query<(&HudView, &DigitSlot, &mut ImageNode)>,
    // The angle each view's needle was drawn at.
    mut drawn_angles: Local<Vec<Option<i32>>>,
) {
    let Some(art) = art else {
        let font = digits(&pack_icons(), &textures.vram).unwrap_or_default();
        commands.insert_resource(Art {
            dial: smooth_image(dial(), &mut images),
            digits: font
                .into_iter()
                .map(|d| atlas_image(d, &mut images))
                .collect(),
        });
        return;
    };
    let missing = hud.missing(parts.iter().map(|(v, _)| *v));
    if !missing.is_empty() {
        // (The new needles are at rest.)
        drawn_angles.clear();
        for view in missing {
            spawn(&mut commands, &art, &mut materials, view);
        }
        return;
    }
    let visibility = if showing.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (_, mut part) in &mut parts {
        part.set_if_neq(visibility);
    }
    if !showing.0 {
        return;
    }

    drawn_angles.resize(hud.len(), None);
    for (&HudView(view), needle) in &needles {
        let Some(kart) = hud.get(view) else {
            continue;
        };
        let d = kart.driver;
        let angle = needle_angle(
            d.speedometer_needle_value as i32,
            d.stats.accel_speed_class_stat as i32,
            d.stats.sacred_fire_speed as i32,
        );
        if drawn_angles[view] != Some(angle)
            && let Some(mut material) = materials.get_mut(&needle.0)
        {
            *material = NeedleMaterial::at(angle);
            drawn_angles[view] = Some(angle);
        }
    }

    for (&HudView(view), mut node, mut background) in &mut fills {
        let Some(kart) = hud.get(view) else {
            continue;
        };
        let jump = kart.driver.jump_meter as i32;
        // The outline covers the bar's top row.
        let height = hud::y((jump * BAR_HEIGHT / JUMP_MAX).clamp(0, BAR_HEIGHT - 1) as f32);
        if node.height != height {
            node.height = height;
        }
        background.set_if_neq(BackgroundColor(bar_color(jump)));
    }
    for (&HudView(view), slot, mut node) in &mut slots {
        let Some(kart) = hud.get(view) else {
            continue;
        };
        let jump = kart.driver.jump_meter as i32;
        let image = &art.digits[jump_digits(jump)[slot.0]];
        if node.image != *image {
            node.image = image.clone();
        }
    }
}

/// Retail screen coordinates, inside the `place` node.
fn at(x: i32, y: i32, width: i32, height: i32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: hud::x((x - LEFT) as f32),
        top: hud::y((y - TOP) as f32),
        width: hud::x(width as f32),
        height: hud::y(height as f32),
        ..default()
    }
}

/// The dial goes under the slide meter and the rest over it, so they are two nodes either side of
/// its `ZIndex`. Like the rest of the HUD they are placed from the window's bottom right.
fn spawn(
    commands: &mut Commands,
    art: &Art,
    materials: &mut Assets<NeedleMaterial>,
    view: usize,
) {
    let place = Node {
        position_type: PositionType::Absolute,
        right: hud::x((hud::WIDTH - LEFT - WIDTH as i32) as f32),
        bottom: hud::y((hud::HEIGHT - TOP - HEIGHT as i32) as f32),
        width: hud::x(WIDTH as f32),
        height: hud::y(HEIGHT as f32),
        ..default()
    };
    let picture =
        |image: &Handle<Image>| ImageNode::new(image.clone()).with_mode(NodeImageMode::Stretch);
    commands.spawn((
        Part,
        HudView(view),
        picture(&art.dial),
        place.clone(),
        ZIndex(0),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
    let overlay = commands
        .spawn((
            Part,
            HudView(view),
            place,
            ZIndex(crate::slide_meter::Z + 1),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    // Same outline as `slide_meter.rs`.
    let outlined = |node: Node| Node {
        width: match node.width {
            Val::Px(w) => Val::Px(w + hud::PIXEL.x),
            other => other,
        },
        height: match node.height {
            Val::Px(h) => Val::Px(h + hud::PIXEL.y),
            other => other,
        },
        border: UiRect::axes(hud::x(1.0), hud::y(1.0)),
        ..node
    };
    commands.spawn((
        outlined(at(JUMP_X, JUMP_Y - BAR_HEIGHT, BAR_WIDTH, BAR_HEIGHT)),
        BorderColor::all(Color::BLACK),
        BackgroundColor(BAR_EMPTY),
        Pickable::IGNORE,
        ChildOf(overlay),
        children![(
            BarFill,
            HudView(view),
            Node {
                position_type: PositionType::Absolute,
                left: Val::ZERO,
                right: Val::ZERO,
                bottom: Val::ZERO,
                height: Val::ZERO,
                ..default()
            },
            BackgroundColor(bar_color(0)),
            Pickable::IGNORE,
        )],
    ));
    commands.spawn((
        outlined(at(
            JUMP_X + NUMBER_X,
            JUMP_Y + NUMBER_Y,
            NUMBER_WIDTH,
            NUMBER_HEIGHT,
        )),
        BorderColor::all(Color::BLACK),
        BackgroundColor(Color::WHITE),
        Pickable::IGNORE,
        ChildOf(overlay),
    ));
    if !art.digits.is_empty() {
        for (n, x) in DIGITS_X.into_iter().enumerate() {
            commands.spawn((
                DigitSlot(n),
                HudView(view),
                picture(&art.digits[0]),
                at(
                    JUMP_X + x,
                    JUMP_Y + DIGITS_Y,
                    DIGIT_SIZE as i32,
                    DIGIT_SIZE as i32,
                ),
                Pickable::IGNORE,
                ChildOf(overlay),
            ));
        }
    }
    commands.spawn((
        MaterialNode(materials.add(NeedleMaterial::at(LOW_ANGLE))),
        HudView(view),
        at(LEFT, TOP, WIDTH as i32, HEIGHT as i32),
        Pickable::IGNORE,
        ChildOf(overlay),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCEL: i32 = 14640;
    const SACRED: i32 = 4096;

    #[test]
    fn the_needle_rests_at_the_low_end_and_tops_out_at_the_high_one() {
        assert_eq!(needle_angle(0, ACCEL, SACRED), LOW_ANGLE);
        assert_eq!(needle_angle(ACCEL + SACRED, ACCEL, SACRED), HIGH_ANGLE);
        assert_eq!(needle_angle(i16::MAX as i32, ACCEL, SACRED), HIGH_ANGLE);
    }

    #[test]
    fn the_class_top_speed_is_the_middle_of_the_band() {
        assert_eq!(needle_angle(ACCEL, ACCEL, SACRED), MIDDLE_ANGLE);
    }

    #[test]
    fn the_needle_moves_steadily_towards_the_high_end() {
        let angles: Vec<i32> = (0..=ACCEL + SACRED)
            .step_by(256)
            .map(|speed| needle_angle(speed, ACCEL, SACRED))
            .collect();
        assert!(angles.windows(2).all(|w| w[1] <= w[0]), "{angles:?}");
    }

    #[test]
    fn the_hang_time_reads_in_the_meters_seconds() {
        assert_eq!(jump_digits(0), [0, 0, 0]);
        assert_eq!(jump_digits(960), [1, 0, 0]);
        assert_eq!(jump_digits(960 + 96 * 3 + 48), [1, 3, 5]);
        assert_eq!(jump_digits(JUMP_MAX), [2, 5, 0]);
    }

    #[test]
    fn the_bar_changes_colour_at_each_size_of_boost() {
        assert_eq!(bar_color(HANG_SMALL - 1), Color::srgb_u8(0, 0, 0xff));
        assert_eq!(bar_color(HANG_SMALL), Color::srgb_u8(0, 0xff, 0));
        assert_eq!(bar_color(HANG_MEDIUM), Color::srgb_u8(0xff, 0xff, 0));
        assert_eq!(bar_color(HANG_LARGE), Color::srgb_u8(0xff, 0, 0));
    }
}
