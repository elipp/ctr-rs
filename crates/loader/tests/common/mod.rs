#![allow(dead_code)]

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use hadris_iso::read::IsoImage;
use loader::bigfile::BigFile;
use loader::disc::RawSectorReader;

pub type Disc = RawSectorReader<BufReader<File>>;

pub struct Big {
    pub disc: Disc,
    pub big: BigFile,
    /// Size of BIGFILE.BIG on the disc, in bytes.
    pub file_size: u64,
}

/// `CTR_BIN` overrides the default of `<repo>/assets/ctr-u.bin`.
pub fn disc_path() -> PathBuf {
    std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin"))
}

/// Opens BIGFILE.BIG on the disc image, or `None` (with a note) if there is no image.
pub fn open_bigfile() -> Option<Big> {
    let path = disc_path();
    if !path.exists() {
        eprintln!(
            "skipping: {} not found (set CTR_BIN to override)",
            path.display()
        );
        return None;
    }
    let open = || RawSectorReader::new(BufReader::new(File::open(&path).unwrap())).unwrap();

    let image = IsoImage::open(open()).expect("parse ISO 9660 filesystem");
    let root = image.root_dir();
    let (start, file_size) = root
        .iter(&image)
        .entries()
        .map(|e| e.expect("directory entry"))
        .find(|e| e.display_name().split(';').next().unwrap() == "BIGFILE.BIG")
        .map(|e| (e.header().extent.read() as u32, e.total_size()))
        .expect("BIGFILE.BIG not in root directory");

    let mut disc = open();
    let big = BigFile::open(&mut disc, start).expect("parse BIGFILE header");
    Some(Big {
        disc,
        big,
        file_size,
    })
}

/// The folder that `Source::prepare` made from the disc: `CTR_DISC_DIR`, or `<repo>/assets/disc`.
/// None (with a note) if there is no finished one.
pub fn prepared_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CTR_DISC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/disc"));
    if !dir.join("MANIFEST").exists() {
        eprintln!(
            "skipping: {} has no MANIFEST (start the client once to make it, or set CTR_DISC_DIR)",
            dir.display()
        );
        return None;
    }
    Some(dir)
}
