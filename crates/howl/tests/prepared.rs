//! The sounds of the folder that `Source::prepare` made, without the disc image.

use std::path::PathBuf;

use howl::Howl;
use loader::source::Source;

/// `CTR_DISC_DIR` overrides the default of `<repo>/assets/disc`.
#[test]
fn the_sounds_load_without_the_disc() {
    let dir = std::env::var_os("CTR_DISC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/disc"));
    if !dir.join("MANIFEST").exists() {
        eprintln!("skipping: {} has no MANIFEST (set CTR_DISC_DIR)", dir.display());
        return;
    }
    let source = Source::new(&dir, None);
    let howl = Howl::read_from(&source).expect("read KART.HWL");
    assert!(!howl.other_fx().is_empty());
    assert_eq!(source.xnf().expect("read ENG.XNF").files, [2, 6, 21]);
}
