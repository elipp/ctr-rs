//! The browser build. A page has no files, so the disc image is the one the player dropped on it
//! (`web/play.html`): the page holds it, and the program reads it from there as the image at
//! [`DISC`] (`loader::disc::keep_external`) for `disc.rs` to find. What the game makes of it
//! (`disc::prepare`, `maps::make_builtins`) goes to `loader::store`, which the page keeps in its
//! IndexedDB ([`take_kept`]): a later start needs no image, as a native one doesn't. Once that is
//! made, the page lets go of the image ([`release_disc`]): a wasm program's memory never shrinks,
//! so the image is never copied in. The clipboard is
//! the page's as well, and so are the settings (`config.rs`) and the maps other players sent
//! (`maps.rs`), which it keeps in its storage, and a panic is told to it (`crash.rs`). On a touch
//! screen its buttons and text field stand in for the keyboard (`command.rs`).

use std::path::PathBuf;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The path that the dropped image goes by.
const DISC: &str = "dropped.bin";

#[wasm_bindgen]
extern "C" {
    /// The size in bytes of the image the player dropped.
    #[wasm_bindgen(js_namespace = window, js_name = ctrDiscSize)]
    fn disc_size() -> f64;

    /// Copies `buf.len()` bytes of the image from byte `at` into `buf`.
    #[wasm_bindgen(js_namespace = window, js_name = ctrReadDisc)]
    fn read_disc(at: f64, buf: &mut [u8]);

    /// Lets the page let go of the image.
    #[wasm_bindgen(js_namespace = window, js_name = ctrReleaseDisc)]
    fn release_disc_bytes();

    /// Whether the page was given an image: it starts the game without one when it has kept
    /// what was made of one.
    #[wasm_bindgen(js_namespace = window, js_name = ctrHasDisc)]
    fn has_disc() -> bool;

    /// Asks the page for the image, which it does on a reload: the files it kept are not enough
    /// (another version of the game, or the browser evicted some). `why` is shown.
    #[wasm_bindgen(js_namespace = window, js_name = ctrNeedDisc)]
    pub fn need_disc(why: &str);

    /// What the page kept of `loader::store`, as `[path, Uint8Array]` pairs, once.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeKept)]
    fn take_kept_files() -> js_sys::Array;

    #[wasm_bindgen(js_namespace = window, js_name = ctrKeep)]
    fn keep_file(path: &str, bytes: &[u8]);

    #[wasm_bindgen(js_namespace = window, js_name = ctrUnkeep)]
    fn unkeep_file(path: &str);

    /// Puts `text` on the clipboard; a failure is only reported on the console.
    #[wasm_bindgen(js_namespace = window, js_name = ctrCopy)]
    pub fn copy(text: &str);

    /// Asks for the text on the clipboard, which the browser reads in the background:
    /// [`take_pasted`] has it once it is read.
    #[wasm_bindgen(js_namespace = window, js_name = ctrPaste)]
    pub fn request_paste();

    /// The text [`request_paste`] asked for, once.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakePasted)]
    pub fn take_pasted() -> Option<String>;

    /// What the page's text field holds, once each time it changes: the on-screen keyboard's way
    /// of typing in the command line.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeTyped)]
    pub fn take_typed() -> Option<String>;

    /// Tells the page the state of the command line and the menu, for its touch buttons and text
    /// field.
    #[wasm_bindgen(js_namespace = window, js_name = ctrShowLine)]
    pub fn show_line(open: bool, text: &str, menu_open: bool);

    /// How far up from the bottom of the page its on-screen keyboard and buttons reach.
    #[wasm_bindgen(js_namespace = window, js_name = ctrKeyboardInset)]
    pub fn keyboard_inset() -> f32;

    /// Where the console is on the page, for a tap on it to open the command line.
    #[wasm_bindgen(js_namespace = window, js_name = ctrConsoleArea)]
    pub fn console_area(left: f32, top: f32, right: f32, bottom: f32);

    /// How many levels on the page's button has asked for, once.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeLevelStep)]
    pub fn take_level_step() -> i32;

    /// How many racers on the page's button has asked for, once.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeRacerStep)]
    pub fn take_racer_step() -> i32;

    /// Draws at that part of the screen's resolution from now on.
    #[wasm_bindgen(js_namespace = window, js_name = ctrSetRenderScale)]
    pub fn set_render_scale(scale: f32);

    /// The settings the page keeps, none if it has none or cannot read them.
    #[wasm_bindgen(js_namespace = window, js_name = ctrLoadConfig)]
    pub fn load_config() -> Option<String>;

    /// Keeps the settings; what went wrong if the page could not.
    #[wasm_bindgen(js_namespace = window, js_name = ctrSaveConfig)]
    pub fn save_config(text: &str) -> Option<String>;

    /// `console.warn`, for what a native build prints (`tell!`).
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    pub fn console_warn(text: &str);

    /// Moves the settings to where they are no longer read; what went wrong if the page could not.
    #[wasm_bindgen(js_namespace = window, js_name = ctrSetConfigAside)]
    pub fn set_config_aside() -> Option<String>;

    /// The maps the page keeps, as the bytes of their files (`Uint8Array`s), once.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeMaps)]
    fn take_saved_maps() -> js_sys::Array;

    /// Keeps the file of a map that was sent to us, under `key`.
    #[wasm_bindgen(js_namespace = window, js_name = ctrSaveMap)]
    pub fn save_map(key: &str, bytes: &[u8]);

    /// Deletes the maps the page keeps.
    #[wasm_bindgen(js_namespace = window, js_name = ctrPurgeMaps)]
    pub fn purge_maps();

    /// The files dropped on the running game since the last call, as `[name, Uint8Array]` pairs.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeDropped)]
    fn take_dropped_files() -> js_sys::Array;

    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeTrackPictures)]
    fn take_saved_track_pictures() -> js_sys::Array;

    /// Keeps the picture of a map we took, under `key`.
    #[wasm_bindgen(js_namespace = window, js_name = ctrSaveTrackPicture)]
    pub fn save_track_picture(key: &str, bytes: &[u8]);

    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeRelicPbs)]
    fn take_saved_relic_pbs() -> js_sys::Array;

    /// Keeps a relic race's personal best (`relic_pb.rs`), under `key`.
    #[wasm_bindgen(js_namespace = window, js_name = ctrSaveRelicPb)]
    pub fn save_relic_pb(key: &str, bytes: &[u8]);

    /// Hands the page a warning or an error that was logged, for it to show if the game crashes.
    #[wasm_bindgen(js_namespace = window, js_name = ctrLog)]
    pub fn log_line(line: &str);

    /// Tells the page, once a second, how the game is doing: the size of its memory in bytes, the
    /// frames drawn so far, the frame rate and whether a race is being driven. If the browser
    /// ends the page without a word, the page's trail has the last of these.
    #[cfg(feature = "wasm-debug")]
    #[wasm_bindgen(js_namespace = window, js_name = ctrBeat)]
    fn beat_page(memory: f64, frames: f64, fps: f64, driving: bool);

    /// Tells the page what the game is about to do and how big its memory is, in bytes; the
    /// page writes it to its trail at once, as the browser may end the page in the work that
    /// follows.
    #[cfg(feature = "wasm-debug")]
    #[wasm_bindgen(js_namespace = window, js_name = ctrStage)]
    fn stage_page(what: &str, memory: f64);

    /// Tells the page that the game has crashed, with what the panic said.
    #[wasm_bindgen(js_namespace = window, js_name = ctrCrashed)]
    pub fn crashed(message: &str);
}

/// The files of the maps the page keeps.
pub fn take_maps() -> Vec<Vec<u8>> {
    take_saved_maps()
        .iter()
        .map(|bytes| bytes.unchecked_into::<js_sys::Uint8Array>().to_vec())
        .collect()
}

/// The pictures of the maps the page keeps.
pub fn take_track_pictures() -> Vec<Vec<u8>> {
    take_saved_track_pictures()
        .iter()
        .map(|bytes| bytes.unchecked_into::<js_sys::Uint8Array>().to_vec())
        .collect()
}

/// The relic races' personal bests the page keeps, once.
pub fn take_relic_pbs() -> Vec<Vec<u8>> {
    take_saved_relic_pbs()
        .iter()
        .map(|bytes| bytes.unchecked_into::<js_sys::Uint8Array>().to_vec())
        .collect()
}

/// Fills `loader::store` with what the page kept, and has it keep every change from now on.
/// Before anything reads the store.
pub fn take_kept() {
    let kept = take_kept_files().iter().filter_map(|pair| {
        let pair: js_sys::Array = pair.dyn_into().ok()?;
        let path = pair.get(0).as_string()?;
        let bytes = pair.get(1).dyn_into::<js_sys::Uint8Array>().ok()?;
        Some((PathBuf::from(path), bytes.to_vec()))
    });
    loader::store::preload(kept);
    loader::store::on_change(|path, bytes| {
        let key = path.to_string_lossy();
        match bytes {
            Some(bytes) => keep_file(&key, bytes),
            None => unkeep_file(&key),
        }
    });
}

/// Reads the dropped image from the page from now on, if it has one.
pub fn take_disc_image() {
    if !has_disc() {
        return;
    }
    loader::disc::keep_external(PathBuf::from(DISC), disc_size() as u64, |at, buf| {
        read_disc(at as f64, buf);
        Ok(())
    });
}

/// Lets go of the dropped image, once everything the game reads of it has been taken: reading it
/// fails from then on.
pub fn release_disc() {
    loader::disc::release_external();
    release_disc_bytes();
}

/// The files dropped on the page while the game runs: their names and contents.
pub fn take_dropped() -> Vec<(String, Vec<u8>)> {
    take_dropped_files()
        .iter()
        .filter_map(|pair| {
            let pair: js_sys::Array = pair.dyn_into().ok()?;
            let name = pair.get(0).as_string()?;
            let bytes = pair.get(1).dyn_into::<js_sys::Uint8Array>().ok()?;
            Some((name, bytes.to_vec()))
        })
        .collect()
}

/// Once a second, tells the page how the game is doing (`ctrBeat`). A page that a phone ends for
/// using too much memory leaves nothing else behind.
#[cfg(feature = "wasm-debug")]
pub fn beat(
    time: bevy::prelude::Res<bevy::prelude::Time<bevy::prelude::Real>>,
    diagnostics: bevy::prelude::Res<bevy::diagnostic::DiagnosticsStore>,
    drive: Option<bevy::prelude::Res<crate::drive::Drive>>,
    images: bevy::prelude::Res<bevy::prelude::Assets<bevy::prelude::Image>>,
    meshes: bevy::prelude::Res<bevy::prelude::Assets<bevy::prelude::Mesh>>,
    mut frames: bevy::prelude::Local<u64>,
    mut last: bevy::prelude::Local<f32>,
    mut shown: bevy::prelude::Local<(usize, usize, u64)>,
) {
    *frames += 1;
    let now = time.elapsed_secs();
    if now - *last < 1.0 {
        return;
    }
    *last = now;
    let fps = diagnostics
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let memory = core::arch::wasm32::memory_size(0) as f64 * 65536.0;
    beat_page(memory, *frames as f64, fps, drive.is_some());

    // What the assets add up to, logged when it changes (the page keeps the last lines): the
    // images' texels as the GPU holds them, and the meshes' vertex data.
    let texels: u64 = images
        .iter()
        .map(|(_, i)| {
            let size = i.texture_descriptor.size;
            let bytes = i.texture_descriptor.format.block_copy_size(None).unwrap_or(4);
            size.width as u64 * size.height as u64 * size.depth_or_array_layers as u64 * bytes as u64
        })
        .sum();
    let counts = (images.len(), meshes.len(), texels >> 20);
    if counts != *shown {
        *shown = counts;
        log_line(&format!(
            "INFO assets: {} images, {} MB of texels, {} meshes",
            counts.0, counts.2, counts.1
        ));
    }
}

/// Tells the page what the game is about to do, with the size of its memory: for a load that the
/// browser may end the page in, which leaves nothing else behind.
#[cfg(feature = "wasm-debug")]
pub fn stage(what: &str) {
    stage_page(what, core::arch::wasm32::memory_size(0) as f64 * 65536.0);
}
