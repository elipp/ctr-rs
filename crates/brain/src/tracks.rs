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
    /// The jumps on them, as (path, the frame it launches from): the last of each run of frames
    /// that launch retail's bots (`race::bot::SPECIAL_RAMP`).
    pub jumps: Vec<(usize, usize)>,
    /// How often the episodes are on this track, against the others' (1 for each to begin with).
    pub weight: f32,
}

/// The path of `track` that is nearest the others on the whole: with three lines side by side,
/// the middle one. By each path's frames' mean distance (on the ground plane) to the nearest frame
/// of each other path. None with fewer than three paths.
pub fn middle_path(track: &Track) -> Option<usize> {
    if track.paths.len() < 3 {
        return None;
    }
    let pos = |p: usize| -> Vec<(i64, i64)> {
        track.level.nav_paths[p]
            .frames
            .iter()
            .map(|f| (f.pos.x as i64, f.pos.z as i64))
            .collect()
    };
    let points: Vec<(usize, Vec<(i64, i64)>)> = track.paths.iter().map(|&p| (p, pos(p))).collect();
    let mean_to = |from: &[(i64, i64)], to: &[(i64, i64)]| -> f64 {
        from.iter()
            .map(|&(x, z)| {
                to.iter()
                    .map(|&(u, v)| (x - u).pow(2) + (z - v).pow(2))
                    .min()
                    .map_or(0.0, |d2| (d2 as f64).sqrt())
            })
            .sum::<f64>()
            / from.len().max(1) as f64
    };
    points
        .iter()
        .map(|(p, from)| {
            let off: f64 = points
                .iter()
                .filter(|(q, _)| q != p)
                .map(|(_, to)| mean_to(from, to))
                .sum();
            (*p, off)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p)
}

/// Keeps only some of the tracks' paths, by `list`: `id:path` or `id:middle` ([`middle_path`]),
/// comma-separated (several for one track keep each). The jumps on the others go too.
pub fn keep_paths(tracks: &mut [Track], list: &str) -> Result<(), String> {
    let mut keep: Vec<(u32, usize)> = Vec::new();
    for entry in list.split(',') {
        let (id, path) = entry
            .trim()
            .split_once(':')
            .ok_or_else(|| format!("paths: expected id:path or id:middle, got {entry:?}"))?;
        let id: u32 = id.parse().map_err(|e| format!("paths: {id:?}: {e}"))?;
        let track = tracks
            .iter()
            .find(|t| t.id.0 == id)
            .ok_or_else(|| format!("paths: track {id} is not among the ones loaded"))?;
        let path = match path {
            "middle" => middle_path(track)
                .ok_or_else(|| format!("paths: track {id} has no middle of three paths"))?,
            p => p.parse().map_err(|e| format!("paths: {p:?}: {e}"))?,
        };
        if !track.paths.contains(&path) {
            return Err(format!("paths: track {id} has no path {path}"));
        }
        keep.push((id, path));
    }
    for track in tracks.iter_mut() {
        let kept: Vec<usize> = keep
            .iter()
            .filter(|(id, _)| *id == track.id.0)
            .map(|&(_, p)| p)
            .collect();
        if !kept.is_empty() {
            track.paths.retain(|p| kept.contains(p));
            track.jumps.retain(|(p, _)| kept.contains(p));
        }
    }
    Ok(())
}

/// The launches of `path` ([`Track::jumps`]).
fn launches(path: &[loader::lev::NavFrame]) -> Vec<usize> {
    let n = path.len();
    let ramp = |i: usize| path[i % n].special_bits & race::bot::SPECIAL_RAMP != 0;
    if n < 2 || (0..n).all(ramp) {
        return Vec::new();
    }
    (0..n).filter(|&i| ramp(i) && !ramp(i + 1)).collect()
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
            let paths: Vec<usize> = (0..level.nav_paths.len())
                .filter(|&i| level.nav_paths[i].frames.len() >= 2)
                .collect();
            let jumps = paths
                .iter()
                .flat_map(|&p| launches(&level.nav_paths[p].frames).into_iter().map(move |f| (p, f)))
                .collect();
            Ok(Track {
                id,
                level,
                objects,
                lengths,
                paths,
                jumps,
                weight: 1.0,
            })
        })
        .collect()
}
