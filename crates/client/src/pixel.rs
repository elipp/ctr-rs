//! A retro look: the scene is drawn at a fraction of the window's resolution, and each of its
//! pixels is shown as a square of whole pixels (`/config set pixel-scale`). The cameras of the
//! scene draw into one small image instead of the window, and a camera of its own puts that image
//! on the window, unfiltered. The HUD is not part of it: the UI, and the cameras marked
//! [`OnWindow`] (the HUD's figures), draw on the window at its own resolution, over the picture.
//!
//! [`RenderSize`] is the size of what the scene's cameras draw into, which is the window's when
//! the scale is 1: what is drawn in the scene by "the screen" reads it, and not the window's. A
//! spot in the scene's picture is put on the window by [`RenderSize::to_window`].

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::PrimaryWindow;

use crate::config::Config;

/// The render layer of the picture on the window, which only its camera draws.
const LAYER: usize = 6;

/// The most the scale can be.
pub const MAX_SCALE: u32 = 8;

/// What the scene's cameras draw into: the size in pixels, and how many of them make a logical
/// one.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RenderSize {
    pub physical: UVec2,
    pub scale_factor: f32,
    /// How many of the window's logical pixels one logical pixel of the picture is shown as.
    pub shown: f32,
    /// The window's logical size.
    pub window: Vec2,
}

impl RenderSize {
    pub fn logical(&self) -> Vec2 {
        self.physical.as_vec2() / self.scale_factor
    }

    pub fn height(&self) -> f32 {
        self.logical().y
    }

    /// Where a spot of the picture (in its logical pixels, as `Camera::world_to_viewport` gives
    /// it) is on the window, in the window's. The picture is centred on the window, and may stick
    /// out of it a little on each side.
    pub fn to_window(&self, at: Vec2) -> Vec2 {
        (at - self.logical() / 2.0) * self.shown + self.window / 2.0
    }
}

/// Marks the picture on the window and its camera, which outlive the scene.
#[derive(Component)]
pub struct Present;

/// A camera that always draws on the window, at its resolution, and not into the picture of the
/// scene: the HUD's. Its order must be over [`PRESENT_ORDER`], so that it draws over the picture.
#[derive(Component)]
pub struct OnWindow;

/// A camera that draws into an image of its own, which is left as it is: a picture of a track's.
#[derive(Component)]
pub struct OwnTarget;

/// The order of the camera that puts the picture on the window: after the scene's cameras, which
/// draw into the picture (the players' are at `10 * view`, `drive::split`), and before the HUD's
/// ([`OnWindow`]), which draw over it. (Without the scale there is no such camera, and the HUD's
/// are over the scene's all the same.)
pub const PRESENT_ORDER: isize = 100;

/// The orders of the HUD's cameras, over the picture. In a split screen each player has one of
/// each (the order is the first plus the view): the one that draws what goes under the figures
/// (the weapon's glows), the one that draws the figures (the fruit and the tracker), and the one
/// that draws the rest of the HUD over them (`drive::HudCamera`).
pub const UNDER_ORDER: isize = PRESENT_ORDER + 1;
pub const FIGURES_ORDER: isize = PRESENT_ORDER + 10;
pub const HUD_ORDER: isize = PRESENT_ORDER + 20;
/// The order of the camera that the window's own UI goes to in a split screen (the menus, the
/// console, the toasts): over every player's HUD, which has cameras of their own.
pub const UI_ORDER: isize = PRESENT_ORDER + 50;

/// The image that the scene's cameras draw into, while the scale is not 1.
#[derive(Resource, Default)]
struct Target(Option<Handle<Image>>);

pub struct PixelPlugin;

impl Plugin for PixelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RenderSize {
            physical: UVec2::ONE,
            scale_factor: 1.0,
            shown: 1.0,
            window: Vec2::ONE,
        });
        // (The browser has no pixel scale, the page's `render-scale` sizes the canvas instead: the
        // scale is 1 there, and the picture is only for a split screen.)
        app.init_resource::<Target>().add_systems(PreUpdate, follow);
    }
}

/// What the cameras draw into at a pixel scale of `scale`, in `window`.
fn render_size(window: &Window, scale: u32) -> RenderSize {
    let (physical, scale_factor) = if scale == 1 {
        (window.physical_size(), window.scale_factor())
    } else {
        // Rounded up, so that the picture covers the window, and the rest is cut off.
        let full = window.physical_size();
        (UVec2::new(full.x.div_ceil(scale), full.y.div_ceil(scale)), 1.0)
    };
    RenderSize {
        physical: physical.max(UVec2::ONE),
        scale_factor,
        shown: scale as f32 * scale_factor / window.scale_factor(),
        window: window.size(),
    }
}

/// Keeps `size` as `wanted`, without marking it changed when it already is.
fn set_size(size: &mut ResMut<RenderSize>, wanted: RenderSize) {
    if size.physical != wanted.physical
        || size.scale_factor != wanted.scale_factor
        || size.shown != wanted.shown
        || size.window != wanted.window
    {
        **size = wanted;
    }
}

/// The camera that puts the picture on the window, and the picture, `shown` that big.
fn spawn_present(
    commands: &mut Commands,
    config: &Config,
    image: Handle<Image>,
    shown: Vec2,
) -> Entity {
    commands.spawn((
        Present,
        Sprite {
            custom_size: Some(shown),
            ..Sprite::from_image(image)
        },
        RenderLayers::layer(LAYER),
    ));
    commands
        .spawn((
            Present,
            Camera2d,
            Camera {
                order: PRESENT_ORDER,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            // The HUD's cameras draw on the same window after it, and the cameras of a window
            // must multisample alike to share its picture (`config::msaa` keeps it so).
            config.msaa(),
            // The picture's colours are what the scene's cameras made.
            bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            RenderLayers::layer(LAYER),
        ))
        .id()
}

fn target_image(size: UVec2) -> Image {
    let (width, height) = (size.x.max(1), size.y.max(1));
    // WebGL2 has no view formats (a texture seen as another format): the sRGB texture itself,
    // which the cameras and the sprite see the same way as the view of the other.
    #[cfg(target_arch = "wasm32")]
    let mut image =
        Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    #[cfg(not(target_arch = "wasm32"))]
    let mut image = Image::new_target_texture(
        width,
        height,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Sizes the picture to the window and the scale, points the scene's cameras at it (or back at
/// the window when the scale is 1), and puts it on the window.
///
/// When the size of the picture or how it is shown changes, all of it is made again (a new
/// image, a new camera and sprite to show it) instead of changed in place, and every camera
/// measures its target again, as on a resize: with the image replaced under its handle and the
/// same camera turned on and off, the window stopped being presented after a change of scale,
/// until the window was resized.
fn follow(
    config: Res<Config>,
    mut target: ResMut<Target>,
    mut size: ResMut<RenderSize>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cameras: Query<
        (Entity, &Camera, &mut RenderTarget, &mut Projection, Has<OnWindow>),
        (Without<Present>, Without<OwnTarget>),
    >,
    presenting: Query<Entity, (With<Present>, With<Camera>)>,
    present: Query<Entity, With<Present>>,
    mut commands: Commands,
    default_ui: Query<Entity, With<IsDefaultUiCamera>>,
    drive: Option<Res<crate::drive::Drive>>,
    mut last: Local<Option<(UVec2, u32, f32, bool)>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = config.pixel_scale.clamp(1, MAX_SCALE);
    let wanted = render_size(window, scale);
    set_size(&mut size, wanted);

    // A split screen always has the picture, even at scale 1: the views are in parts of one
    // texture that the cameras of a window share, and the cameras over it (the HUD's) draw that
    // texture whole, which has only the last view's part right. The picture is a target of its own
    // that they do not share.
    let split = drive.is_some_and(|d| d.views().len() > 1);
    let on = scale > 1 || split;
    let key = (wanted.physical, scale, window.scale_factor(), on);
    let moved = *last != Some(key);
    *last = Some(key);

    // The camera that presents, while the scale is on.
    let mut presenter = presenting.iter().next();
    if moved {
        for entity in &present {
            commands.entity(entity).despawn();
        }
        // (The old image goes when the last camera lets go of its handle, below.)
        target.0 = None;
        presenter = None;
        if on {
            let image = images.add(target_image(wanted.physical));
            let shown = wanted.physical.as_vec2() * scale as f32 / window.scale_factor();
            presenter = Some(spawn_present(&mut commands, &config, image.clone(), shown));
            target.0 = Some(image);
        }
    }

    // The camera that draws on the window last.
    let mut last_drawn: Option<(Entity, isize)> = None;
    let mut drawn = |entity: Entity, order: isize| {
        if last_drawn.is_none_or(|(_, last)| order >= last) {
            last_drawn = Some((entity, order));
        }
    };
    if let Some(entity) = presenter {
        drawn(entity, PRESENT_ORDER);
    }
    for (entity, camera, mut render_target, mut projection, hud) in &mut cameras {
        // The HUD's draw on the window either way, and the scene's into the picture when it is on.
        let picture = target.0.as_ref().filter(|_| !hud);
        let right = match (&*render_target, picture) {
            (RenderTarget::Image(drawn), Some(image)) => drawn.handle == *image,
            (RenderTarget::Image(_), None) => false,
            (_, picture) => picture.is_none(),
        };
        if !right {
            *render_target = match picture {
                Some(image) => RenderTarget::Image(image.clone().into()),
                None => RenderTarget::default(),
            };
        }
        if !right || moved {
            // Bevy only measures a camera's target again when the window or the image say they
            // changed, or the projection does: not for another target, and the UI and the
            // projections go by that size.
            projection.set_changed();
        }
        if picture.is_none() {
            drawn(entity, camera.order);
        }
    }

    // Bevy's UI goes to the highest camera that draws into the window if none is marked: the
    // camera drawn on the window last takes it, so that it is over the rest and at the window's
    // resolution. (The one that presents may be spawned only now, or despawned.)
    let ui = last_drawn.map(|(entity, _)| entity);
    for entity in &default_ui {
        if Some(entity) != ui {
            commands.entity(entity).try_remove::<IsDefaultUiCamera>();
        }
    }
    if let Some(entity) = ui
        && default_ui.get(entity).is_err()
    {
        commands.entity(entity).try_insert(IsDefaultUiCamera);
    }
}
