//! The browser build. A page has no files, so the disc image is the one the player dropped on it
//! (`web/play.html`): the page holds it until `main` runs, which takes it into memory as the
//! image at [`DISC`] (`loader::disc::keep_in_memory`) for `disc.rs` to find. The clipboard is
//! the page's as well.

use std::path::PathBuf;

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
}

/// Copies the dropped image into memory.
pub fn take_disc_image() {
    loader::disc::keep_in_memory(PathBuf::from(DISC), take_disc().to_vec());
}
