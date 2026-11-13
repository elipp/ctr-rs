//! Our place in an online race, at the bottom left: the big digit (`render::place`) and its
//! suffix ("ST", "ND" ...) in the large font after its top. The sizes and places are measured
//! from retail screenshots.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use render::font::{Font, Gradient};
use render::place::{Digit, suffix};

use crate::font::CtrText;
use crate::hud;
use crate::hud_view::{self, HudView};

/// How big a unit of the digits' models is drawn, in pixels of retail's screen across and lines
/// down: in a retail screenshot the "1", 42 units wide and 55 tall, is about 52 pixels wide and
/// 54 lines tall.
const UNIT: Vec2 = Vec2::new(1.25, 1.0);

/// Where the digit is, from the left of the screen and from its bottom, in pixels of retail's
/// screen.
const LEFT: f32 = 8.0;
const BOTTOM: f32 = 4.0;

/// Where the suffix is: how far right of the digit, and how far down from its top.
const SUFFIX_GAP: f32 = 2.0;
const SUFFIX_DOWN: f32 = 9.0;

/// The digits 1 to 8 as images, with their sizes on screen in logical pixels.
#[derive(Resource)]
pub struct BigDigits(Vec<(Handle<Image>, Vec2)>);

impl BigDigits {
    pub fn new(digits: Vec<Digit>, images: &mut Assets<Image>) -> BigDigits {
        BigDigits(
            digits
                .into_iter()
                .map(|digit| {
                    let size = Vec2::from(digit.size()) * UNIT * hud::PIXEL;
                    (smooth_image(digit, images), size)
                })
                .collect(),
        )
    }
}

/// A digit's picture, filtered: it is drawn smaller than it is, and its edges are not pixels.
fn smooth_image(digit: Digit, images: &mut Assets<Image>) -> Handle<Image> {
    let atlas = digit.atlas;
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

/// The digit and the suffix.
#[derive(Component)]
struct PlaceHud;

#[derive(Component)]
struct BigDigit;

#[derive(Component)]
struct Suffix;

pub struct PlacePlugin;

impl Plugin for PlacePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(crate::drive::driving));
    }
}

fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    drive: Res<crate::drive::Drive>,
    lobby: Res<crate::lobby::Lobby>,
    digits: Res<BigDigits>,
    mut huds: Query<(&HudView, &mut Visibility, &Children), With<PlaceHud>>,
    mut digit: Query<(&mut ImageNode, &mut Node), With<BigDigit>>,
    mut suffixes: Query<&mut CtrText, With<Suffix>>,
) {
    for view in hud_view::missing(&sim, &drive, huds.iter().map(|(v, ..)| *v)) {
        // The first frame, or of a new level (which took the nodes away).
        spawn(&mut commands, view);
    }
    for (&HudView(view), mut visibility, children) in &mut huds {
        if view >= hud_view::views(&sim, &drive) {
            continue;
        }
        // `driverRank` is -1 until the kart has a place.
        let rank = hud_view::driver_of(&sim, &drive, view).driver_rank;
        let place = usize::try_from(rank).ok().map(|rank| rank + 1);
        let shown = place.filter(|&p| lobby.online.is_some() && p <= digits.0.len());
        visibility.set_if_neq(if shown.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let Some(place) = shown else {
            continue;
        };
        let (image, size) = &digits.0[place - 1];
        for &child in children {
            if let Ok((mut digit, mut node)) = digit.get_mut(child)
                && digit.image != *image
            {
                digit.image = image.clone();
                node.width = Val::Px(size.x);
                node.height = Val::Px(size.y);
            }
            if let Ok(mut text) = suffixes.get_mut(child)
                && text.text != suffix(place)
            {
                text.text = suffix(place).to_string();
            }
        }
    }
}

fn spawn(commands: &mut Commands, view: usize) {
    commands.spawn((
        PlaceHud,
        HudView(view),
        Node {
            position_type: PositionType::Absolute,
            left: hud::x(LEFT),
            bottom: hud::y(BOTTOM),
            align_items: AlignItems::FlexStart,
            column_gap: hud::x(SUFFIX_GAP),
            ..default()
        },
        Visibility::Hidden,
        Pickable::IGNORE,
        children![
            (
                BigDigit,
                ImageNode::default().with_mode(NodeImageMode::Stretch),
                Node::default(),
                Pickable::IGNORE,
            ),
            (
                Suffix,
                CtrText::new("", Font::Large, Gradient::RED).fixed(hud::PIXEL),
                Node {
                    margin: UiRect::top(hud::y(SUFFIX_DOWN)),
                    ..default()
                },
                Pickable::IGNORE,
            ),
        ],
    ));
}
