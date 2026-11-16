//! The language files (`BigIndex::LANGUAGE_FILE + lang`, `LOAD_LangFile`): an `i32` count, an
//! `i32` offset to a table of that many `u32` offsets (from the file's start), each to a
//! NUL-terminated string in the game's font encoding (ASCII for the European languages; the
//! Japanese one is not).

use std::io;

use crate::level::LevelId;

/// The language the USA disc loads (0 ja, 1 en, 2 en2, 3 fr, 4 de, 5 it, 6 es, 7 nl).
pub const ENGLISH: u32 = 1;

pub fn parse(file: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("language file: {what}"));
    let word = |at: usize| {
        file.get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
            .ok_or_else(|| bad("ends early"))
    };
    let (count, table) = (word(0)?, word(4)?);
    if count > file.len() / 4 {
        return Err(bad("claims more strings than it has room for"));
    }
    (0..count)
        .map(|i| {
            let at = word(table + 4 * i)?;
            let rest = file.get(at..).ok_or_else(|| bad("a string is past its end"))?;
            let end = rest.iter().position(|&b| b == 0).ok_or_else(|| bad("a string is not terminated"))?;
            Ok(rest[..end].to_vec())
        })
        .collect()
}

/// The string that names the track `level` (`data.metaDataLEV[level].name_LNG` of the retail
/// exe; a patch could change that table, which we don't read).
pub fn track_name(level: LevelId) -> Option<usize> {
    match level.0 {
        id @ 0..=17 => Some(110 + id as usize),
        id @ 18..=24 => Some(98 + id as usize - 18),
        _ => None,
    }
}
