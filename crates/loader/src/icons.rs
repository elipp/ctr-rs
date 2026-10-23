//! Named icons (`struct Icon`, `struct IconGroup`, `struct LevTexLookup`):
//! single texture layouts that sprites are cut from. The racer packs
//! (`BigIndex::ARCADE_1P_PACK` and friends) start with a `LevTexLookup`, which
//! the game hands to `DecalGlobal_Store`; group 0 holds the 17 wheel frames.

use std::io;

use crate::dram::DramFile;
use crate::texture::{TEXTURE_LAYOUT_SIZE, TextureLayout};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn word(dram: &DramFile, at: u32) -> io::Result<u32> {
    dram.data
        .get(at as usize..at as usize + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid(format!("icon word {at:#x} is outside the payload")))
}

fn name(dram: &DramFile, at: u32) -> io::Result<String> {
    let bytes = dram
        .data
        .get(at as usize..at as usize + 16)
        .ok_or_else(|| invalid("icon name is outside the payload"))?;
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(16);
    Ok(String::from_utf8_lossy(&bytes[..len]).into_owned())
}

/// `struct Icon`.
#[derive(Debug, Clone)]
pub struct Icon {
    pub name: String,
    /// `global_IconArray_Index`
    pub global_index: i32,
    pub layout: TextureLayout,
}

/// `struct IconGroup`: icons the game indexes by number.
#[derive(Debug, Clone)]
pub struct IconGroup {
    pub name: String,
    pub group_id: i16,
    pub icons: Vec<Icon>,
}

const ICON_GROUP_SIZE: u32 = 0x14;

fn icon_at(dram: &DramFile, at: u32) -> io::Result<Icon> {
    let layout = dram
        .data
        .get(at as usize + 0x14..at as usize + 0x14 + TEXTURE_LAYOUT_SIZE)
        .ok_or_else(|| invalid(format!("icon {at:#x} is outside the payload")))?;
    Ok(Icon {
        name: name(dram, at)?,
        global_index: word(dram, at + 0x10)? as i32,
        layout: TextureLayout::parse(layout)?,
    })
}

/// Parses the flat icon array of the `LevTexLookup` a pack starts with
/// (`DecalGlobal_Store` files them by `global_index`).
pub fn parse_icons(dram: &DramFile) -> io::Result<Vec<Icon>> {
    let lookup = word(dram, 0)?;
    if lookup == 0 {
        return Ok(Vec::new());
    }
    let count = word(dram, lookup)?;
    let first = word(dram, lookup + 4)?;
    if count > 0x1000 {
        return Err(invalid(format!("implausible icon count {count}")));
    }
    (0..count)
        .map(|i| icon_at(dram, first + i * 0x20))
        .collect()
}

/// Parses the `LevTexLookup` a pack starts with into its icon groups.
pub fn parse_groups(dram: &DramFile) -> io::Result<Vec<IconGroup>> {
    groups_at(dram, word(dram, 0)?)
}

/// `struct Level::levTexLookup`
const LEVEL_TEX_LOOKUP: u32 = 0x3c;

/// The icon groups of a level's own `LevTexLookup` (the fire of the fireballs, for one).
pub fn parse_level_groups(lev: &DramFile) -> io::Result<Vec<IconGroup>> {
    groups_at(lev, word(lev, LEVEL_TEX_LOOKUP)?)
}

fn groups_at(dram: &DramFile, lookup: u32) -> io::Result<Vec<IconGroup>> {
    if lookup == 0 {
        return Ok(Vec::new());
    }
    let num_groups = word(dram, lookup + 8)?;
    let groups = word(dram, lookup + 12)?;
    if num_groups > 0x100 {
        return Err(invalid(format!(
            "implausible icon group count {num_groups}"
        )));
    }
    (0..num_groups)
        .map(|i| {
            let at = word(dram, groups + i * 4)?;
            let count = dram
                .data
                .get(at as usize + 0x10..at as usize + 0x14)
                .map(|b| {
                    (
                        i16::from_le_bytes([b[0], b[1]]),
                        i16::from_le_bytes([b[2], b[3]]),
                    )
                })
                .ok_or_else(|| invalid(format!("icon group {at:#x} is outside the payload")))?;
            let (group_id, num_icons) = count;
            if !(0..=0x100).contains(&num_icons) {
                return Err(invalid(format!("implausible icon count {num_icons}")));
            }
            Ok(IconGroup {
                name: name(dram, at)?,
                group_id,
                icons: (0..num_icons as u32)
                    .map(|j| icon_at(dram, word(dram, at + ICON_GROUP_SIZE + j * 4)?))
                    .collect::<io::Result<_>>()?,
            })
        })
        .collect()
}
