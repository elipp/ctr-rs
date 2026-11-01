//! Finding the disc image. In order: the image in memory (in a browser, see `web.rs`), `CTR_BIN`,
//! the `iso` of the config, and, in a debug build, `assets/ctr-u.bin`. If there is none, and
//! everything the client reads from the disc has been made already (`prepare`,
//! `maps::make_builtins`), it runs without one. Otherwise a file dialog asks for it and the
//! answer is kept in the config; if that is cancelled, the client says how to give the path and
//! exits.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use loader::disc::open_bigfile;

use crate::config::Config;
use crate::paths;

static DISC: OnceLock<Option<PathBuf>> = OnceLock::new();

/// The disc image, if the client has one; `locate` has to have run.
pub fn image() -> Option<PathBuf> {
    DISC.get().expect("the disc is located first").clone()
}

/// Where the client reads the files of the disc from: the folder it makes from the image
/// (`paths::disc`), and the image behind it, if there is one.
pub fn source() -> loader::source::Source {
    loader::source::Source::new(paths::disc(), image())
}

/// Whether everything the client reads from the disc has been made, so that it needs no image.
fn made() -> bool {
    use loader::source::{Check, Source};
    Source::new(paths::disc(), None).check() == Check::Ready
        && map::builtin::missing_count(&paths::builtin_maps()) == 0
}

/// Makes the files of the disc that the client reads, if the folder lacks any: the first start
/// does (it takes a while), and so does the first after an update that reads more. What went
/// wrong is printed; the files that are missing are then made as they are asked for.
pub fn prepare() {
    use loader::source::Check;
    // A browser has nowhere to keep them: there everything is read from the image.
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let source = source();
    let why = match source.check() {
        Check::Ready => return,
        Check::NotPrepared => "making the files of the disc (this is done once)".to_string(),
        Check::Stale(why) => format!("making the files of the disc again: {why}"),
        Check::Missing(needs) => format!("making {} more files of the disc", needs.len()),
    };
    eprintln!("{}...", why);
    let mut told = 0;
    let made = source.prepare(|done, of| {
        let tenth = if of == 0 { 10 } else { done * 10 / of };
        if tenth > told {
            told = tenth;
            eprintln!("  {}%", tenth * 10);
        }
    });
    match made {
        Ok(made) => eprintln!("Made {made} files in {}", paths::disc().display()),
        Err(e) => eprintln!("Could not make the files of the disc: {e}"),
    }
}

/// Finds the disc and keeps the answer for `image`, or exits the program if it is needed and
/// there is none.
pub fn locate(config: &mut Config) {
    let found = loader::disc::in_memory()
        .or_else(|| std::env::var_os("CTR_BIN").map(PathBuf::from))
        .or_else(|| config.iso.clone().filter(|p| is_disc(p)))
        .or_else(dev_default);
    if found.is_none() && made() {
        DISC.set(None).expect("the disc is located once");
        return;
    }
    let found = found.or_else(|| {
        let picked = ask()?;
        config.iso = Some(picked.clone());
        if let Err(why) = config.save() {
            eprintln!("{why}");
        }
        Some(picked)
    });
    let Some(path) = found else {
        eprintln!(
            "No Crash Team Racing (USA) disc image. Set CTR_BIN to its path, or \"iso\" in {}\n\
             (it is read once, to make the files of the game in {})",
            crate::paths::config_file().display(),
            paths::disc().display()
        );
        std::process::exit(1);
    };
    DISC.set(Some(path)).expect("the disc is located once");
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
#[cfg(not(target_arch = "wasm32"))]
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
                why = Some(format!(
                    "{}: {e}\n\nThat is not a CTR (USA) disc image.",
                    file.display()
                ))
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn ask() -> Option<PathBuf> {
    None
}
