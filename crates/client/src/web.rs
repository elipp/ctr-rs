//! The browser build. A page has no files, so the disc image is the one the player dropped on it
//! (`web/play.html`): the page holds it until `main` runs, which takes it into memory as the
//! image at [`DISC`] (`loader::disc::keep_in_memory`) for `disc.rs` to find. The clipboard is
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
    /// The image the player dropped, which the page lets go of once it is taken.
    #[wasm_bindgen(js_namespace = window, js_name = ctrTakeDisc)]
    fn take_disc() -> js_sys::Uint8Array;

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

/// Copies the dropped image into memory.
pub fn take_disc_image() {
    loader::disc::keep_in_memory(PathBuf::from(DISC), take_disc().to_vec());
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
