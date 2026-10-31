//! The browser build. A page has no files, so the disc image is the one the player dropped on it
//! (`web/play.html`): the page holds it until `main` runs, which takes it into memory as the
//! image at [`DISC`] (`loader::disc::keep_in_memory`) for `disc.rs` to find. The clipboard is
//! the page's as well, and so are the settings (`config.rs`) and the maps other players sent
//! (`maps.rs`), which it keeps in its storage, and a panic is told to it (`crash.rs`).

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

/// Copies the dropped image into memory.
pub fn take_disc_image() {
    loader::disc::keep_in_memory(PathBuf::from(DISC), take_disc().to_vec());
}
