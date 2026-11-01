//! The window's own icon, `icon.png` (the executable's too, see `build.rs`): Windows draws it in
//! the title bar, and an X11 desktop in its taskbar. Wayland and macOS leave it out, as theirs
//! comes from the `.desktop` file and the app bundle.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::NonSendMarker;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::winit::WINIT_WINDOWS;
use winit::window::Icon;

pub struct WindowIconPlugin;

impl Plugin for WindowIconPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, set);
    }
}

/// Once winit has made the window, which may be after the first frame.
fn set(
    mut done: Local<bool>,
    window: Option<Single<Entity, With<PrimaryWindow>>>,
    _: NonSendMarker,
) {
    let Some(window) = window.filter(|_| !*done) else {
        return;
    };
    WINIT_WINDOWS.with_borrow(|winit_windows| {
        let Some(winit_window) = winit_windows.get_window(*window) else {
            return;
        };
        winit_window.set_window_icon(Some(icon()));
        *done = true;
    });
}

fn icon() -> Icon {
    let image = Image::from_buffer(
        include_bytes!("../icon.png"),
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .expect("the icon is a PNG")
    .try_into_dynamic()
    .expect("the icon is RGBA")
    .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).expect("the icon is RGBA")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes() {
        icon();
    }
}
