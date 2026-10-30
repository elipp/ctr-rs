//! The folder that `Source::prepare` made, read without the disc image: everything the game can
//! ask for (`Need::all`) is there and parses as the game parses it.

mod common;

use loader::dram::DramFile;
use loader::icons::{parse_groups, parse_icons};
use loader::model::Model;
use loader::source::{Check, Need, Source};
use loader::vram::parse_vram_file;
use loader::xa::decode;

#[test]
fn every_need_loads_without_the_disc() {
    let Some(dir) = common::prepared_dir() else {
        return;
    };
    // (No image: a file the folder lacks is an error, not a read of the disc.)
    let source = Source::new(&dir, None);
    assert_eq!(source.check(), Check::Ready);

    let xnf = source.xnf().expect("read ENG.XNF");
    let all = Need::all(&xnf);
    assert!(all.len() > 3, "{} needs", all.len());
    for need in all {
        let key = need.key();
        let bytes = source.get(need).unwrap_or_else(|e| panic!("{key}: {e}"));
        assert!(!bytes.is_empty(), "{key} is empty");
        match need {
            Need::Racer(_) => {
                let dram = DramFile::parse(&bytes).unwrap_or_else(|e| panic!("{key}: {e}"));
                Model::parse(&dram).unwrap_or_else(|e| panic!("{key}: {e}"));
            }
            Need::Pack(_) => {
                let dram = DramFile::parse(&bytes).unwrap_or_else(|e| panic!("{key}: {e}"));
                parse_icons(&dram).unwrap_or_else(|e| panic!("{key}: {e}"));
                parse_groups(&dram).unwrap_or_else(|e| panic!("{key}: {e}"));
            }
            Need::SharedVram => {
                let images = parse_vram_file(&bytes).unwrap_or_else(|e| panic!("{key}: {e}"));
                assert!(!images.is_empty(), "{key} has no images");
            }
            Need::Howl | Need::Xnf => {}
            Need::Xa(..) => {
                assert!(!decode(&bytes).frames.is_empty(), "{key} decodes to nothing");
            }
        }
    }
    source.howl().expect("read the header of KART.HWL");
}

#[test]
fn a_need_that_was_not_made_is_an_error_naming_it() {
    let Some(dir) = common::prepared_dir() else {
        return;
    };
    let e = Source::new(&dir, None)
        .get(Need::Racer(loader::source::CHARACTERS))
        .unwrap_err();
    assert!(e.to_string().contains("racer/16"), "{e}");
}
