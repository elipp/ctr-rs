//! The browser build. A page has no files, so the disc image is the one the player dropped on it
//! (`web/play.html`): the page holds it, and the program reads it from there as the image at
//! [`DISC`] (`loader::disc::keep_external`) for `disc.rs` to find. Once everything the game reads
//! of it is taken into memory (`disc::prepare`, `maps::make_builtins`), the page lets go of it
//! ([`release_disc`]): a wasm program's memory never shrinks, so the image is never copied in. The clipboard is
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

/// Reads the dropped image from the page from now on.
pub fn take_disc_image() {
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
