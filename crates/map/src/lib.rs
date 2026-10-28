//! A map as a file of its own (`.ctrmap`): a track of the game that a community patch changed,
//! kept as the viewer's parsed `Level`, its `DramFile` (the rest of the level's data: models and
//! textures) and its VRAM images, so that nothing of the disc or of the patch is needed to load it.
//! `ctr-map import` makes one from an `.xdelta` file.
//!
//! A map takes the place of nothing: it is a track of its own, which the viewer lists after the
//! game's. What tells it from the others is [`Map::id`], made from its contents, so that two
//! players who have the same map agree on which it is (and a player who has not got it can ask
//! for it by that id).
//!
//! The file is the magic, the [`VERSION`] and the [`Map`] in `bitcode`, compressed with zstd.

pub mod import;

use std::io;
use std::path::Path;

use loader::dram::DramFile;
use loader::lev::Level;
use loader::vram::VramImage;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"CTRMAP\0\0";

/// The version of the format: changes with the structs that are saved in it.
pub const VERSION: u32 = 3;

/// zstd's level for a map file: it is made once and sent many times.
const COMPRESSION: i32 = 19;

/// The top bit of the `u32` that names a track: the game's tracks are small numbers, a map's id
/// has this set (and the rest of the id from its contents).
const MAP_ID_BIT: u32 = 0x8000_0000;

/// Whether a track number (a `LevelId`) is a map's.
pub fn is_map_id(track: u32) -> bool {
    track & MAP_ID_BIT != 0
}

/// A track, as the viewer loads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Map {
    /// What it is called: the name of the patch it was made from.
    pub name: String,
    pub level: Level,
    pub dram: DramFile,
    /// The level's own texture pages (`LevFile::Vram`); the pages shared by every track come from
    /// the disc.
    pub vram: Vec<VramImage>,
}

/// FNV-1a over `bytes`, 64 bits.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3)
    })
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl Map {
    fn encode(&self) -> io::Result<Vec<u8>> {
        bitcode::serialize(self).map_err(|e| invalid(format!("saving the map: {e}")))
    }

    /// The number that names the map as a track (a `LevelId`): [`is_map_id`] is true of it, and
    /// the rest is FNV-1a of the map's contents, so the same map has the same id everywhere. (Two
    /// different maps share one in about one in two billion; whoever sends a map also sends
    /// [`Map::hash`], which tells them apart.)
    pub fn id(&self) -> io::Result<u32> {
        let hash = fnv1a(&self.encode()?);
        Ok(MAP_ID_BIT | ((hash ^ hash >> 32) as u32 & !MAP_ID_BIT))
    }

    /// The hash of the map's contents (FNV-1a, 64 bits): not a defence against someone who makes
    /// a map to collide, but enough to know that a map that was received is the one asked for.
    pub fn hash(&self) -> io::Result<u64> {
        Ok(fnv1a(&self.encode()?))
    }

    /// The map as a file: this is also what is sent to a player who has not got it.
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut out = MAGIC.to_vec();
        out.extend(VERSION.to_le_bytes());
        out.extend(zstd::stream::encode_all(
            self.encode()?.as_slice(),
            COMPRESSION,
        )?);
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> io::Result<Map> {
        let rest = bytes
            .strip_prefix(MAGIC)
            .ok_or_else(|| invalid("not a .ctrmap file"))?;
        let (version, rest) = rest
            .split_first_chunk::<4>()
            .ok_or_else(|| invalid("the .ctrmap file ends in its header"))?;
        let version = u32::from_le_bytes(*version);
        if version != VERSION {
            return Err(invalid(format!(
                "the map is version {version}, this build reads version {VERSION}"
            )));
        }
        let unpacked = zstd::stream::decode_all(rest)
            .map_err(|e| invalid(format!("unpacking the map: {e}")))?;
        bitcode::deserialize(&unpacked).map_err(|e| invalid(format!("reading the map: {e}")))
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.to_bytes()?)
    }

    pub fn read(path: &Path) -> io::Result<Map> {
        Map::from_bytes(&std::fs::read(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Map {
        let file: Vec<u8> = (-1i32)
            .to_le_bytes()
            .into_iter()
            .chain([1, 2, 3, 4])
            .collect();
        let dram = DramFile::parse(&file).unwrap();
        let spawn = loader::lev::SpawnPoint {
            pos: Default::default(),
            rot: Default::default(),
        };
        Map {
            name: "tiny".into(),
            level: Level {
                instances: Vec::new(),
                models: Vec::new(),
                spawn_delays: vec![1, 2],
                fly_in: None,
                position_paths: vec![vec![Default::default()]],
                paths: Vec::new(),
                skybox: None,
                clear_colors: [None; 3],
                quad_blocks: Vec::new(),
                vertices: Vec::new(),
                bsp: Vec::new(),
                driver_spawns: [spawn; 8],
                restart_points: Vec::new(),
                water: Default::default(),
                scenery: Default::default(),
                config_flags: 7,
                jump_vertical_speed_cap: 3,
                rain_buffer: Default::default(),
            },
            dram,
            vram: vec![VramImage {
                x: 1,
                y: 2,
                w: 1,
                h: 1,
                pixels: vec![9, 8],
            }],
        }
    }

    #[test]
    fn a_map_comes_back_as_it_was_saved() {
        let map = tiny();
        let bytes = map.to_bytes().unwrap();
        assert_eq!(Map::from_bytes(&bytes).unwrap(), map);
    }

    #[test]
    fn the_id_is_the_contents_and_marks_a_map() {
        let map = tiny();
        let id = map.id().unwrap();
        assert!(is_map_id(id) && !is_map_id(24));
        assert_eq!(id, tiny().id().unwrap());
        let other = Map {
            name: "other".into(),
            ..tiny()
        };
        assert_ne!(other.id().unwrap(), id);
        assert_ne!(other.hash().unwrap(), map.hash().unwrap());
    }

    #[test]
    fn the_file_is_compressed() {
        let mut map = tiny();
        map.vram[0].pixels = vec![7; 100_000];
        assert!(map.to_bytes().unwrap().len() < 1_000);
    }

    #[test]
    fn other_files_and_versions_are_refused() {
        assert!(Map::from_bytes(b"nope").is_err());
        let mut bytes = tiny().to_bytes().unwrap();
        bytes[8] = 99;
        let error = Map::from_bytes(&bytes).unwrap_err();
        assert!(error.to_string().contains("version 99"), "{error}");
    }
}
