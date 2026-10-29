//! Finding the disc image. In order: `CTR_BIN`, the `iso` of the config, and, in a debug build,
//! `assets/ctr-u.bin`. On Windows, if none of them is a disc, a file dialog asks for it and the
//! answer is kept in the config; elsewhere the client says how to give the path and exits.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use loader::disc::open_bigfile;

use crate::config::Config;

static DISC: OnceLock<PathBuf> = OnceLock::new();

/// The disc image; `locate` has to have run.
pub fn path() -> PathBuf {
    DISC.get().expect("the disc is located first").clone()
}

/// Finds the disc and keeps the answer for `path`, or exits the program if there is none.
pub fn locate(config: &mut Config) {
    let found = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .or_else(|| config.iso.clone().filter(|p| is_disc(p)))
        .or_else(dev_default)
        .or_else(|| {
            let picked = ask()?;
            config.iso = Some(picked.clone());
            if let Err(why) = config.save() {
                eprintln!("{why}");
            }
            Some(picked)
        });
    let Some(path) = found else {
        eprintln!(
            "No Crash Team Racing (USA) disc image. Set CTR_BIN to its path, or \"iso\" in {}",
            crate::paths::config_file().display()
        );
        std::process::exit(1);
    };
    DISC.set(path).expect("the disc is located once");
}

fn is_disc(path: &Path) -> bool {
    open_bigfile(path).is_ok()
}

#[cfg(debug_assertions)]
fn dev_default() -> Option<PathBuf> {
    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin"))
        .filter(|p| p.exists())
}

#[cfg(not(debug_assertions))]
fn dev_default() -> Option<PathBuf> {
    None
}

/// Asks until the file is a disc; none if the dialog is cancelled.
#[cfg(windows)]
fn ask() -> Option<PathBuf> {
    let mut why = None;
    loop {
        if let Some(why) = why.take() {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("ctr-rs")
                .set_description(why)
                .show();
        }
        let file = rfd::FileDialog::new()
            .set_title("Select your Crash Team Racing (USA) disc image")
            .add_filter("Disc image", &["bin", "iso"])
            .pick_file()?;
        match open_bigfile(&file) {
            Ok(_) => return Some(file),
            Err(e) => {
                why = Some(format!("{}: {e}\n\nThat is not a CTR (USA) disc image.", file.display()))
            }
        }
    }
}

#[cfg(not(windows))]
fn ask() -> Option<PathBuf> {
    None
}
