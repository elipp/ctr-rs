//! The race tracks, read off the disc image.

use std::path::{Path, PathBuf};

use instance::Objects;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use race::bot::PathLength;

/// The race tracks (the ones a lap is driven on), by `LevelId`.
pub const RACE_TRACKS: std::ops::RangeInclusive<u32> = 0..=17;

/// A track, and what the trainer keeps of its bot paths.
pub struct Track {
    pub id: LevelId,
    pub level: Level,
    /// As the level places them: each race starts from a clone.
    pub objects: Objects,
    /// By `Level::nav_paths`.
    pub lengths: Vec<PathLength>,
    /// The paths a kart can be dropped on (two frames or more).
    pub paths: Vec<usize>,
}

/// `CTR_BIN`, or `<repo>/assets/ctr-u.bin`.
pub fn disc_path() -> PathBuf {
    std::env::var_os("CTR_BIN").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
    })
}

/// The tracks `ids` off the disc image at `disc`.
pub fn load(disc: &Path, ids: impl IntoIterator<Item = u32>) -> std::io::Result<Vec<Track>> {
    let (mut reader, big) = loader::disc::open_bigfile(disc)?;
    ids.into_iter()
        .map(|id| {
            let id = LevelId(id);
            let index = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
            let dram = DramFile::parse(&big.read(&mut reader, index.get())?)?;
            let level = Level::parse(&dram)?;
            let objects = Objects::new(&level, &dram);
            let lengths = race::nn::path_lengths(&level);
            let paths = (0..level.nav_paths.len())
                .filter(|&i| level.nav_paths[i].frames.len() >= 2)
                .collect();
            Ok(Track {
                id,
                level,
                objects,
                lengths,
                paths,
            })
        })
        .collect()
}
