//! `BIGFILE.BIG`: a flat archive of every level, model and texture file of the game.
//!
//! Layout: an `i32` (the file's start sector, filled in at runtime by the game), the entry
//! count, then `(sector offset, size)` pairs. The archive is not compressed, and neither are its
//! entries: [`BigFile::read`] returns the bytes as stored, seeking straight to
//! `start + sector_offset` and reading `size` bytes. (The files inside have their own formats,
//! like the delta-compressed animation frames of a model, but nothing here inflates anything.)

use std::io::{self, Read, Seek, SeekFrom};

use crate::disc::USER_DATA_SIZE;

/// Size in bytes of the header region read from disc (`LOAD_BIGFILE_HEADER_SECTORS`).
const HEADER_SECTORS: u64 = 8;
const HEADER_SIZE: usize = (HEADER_SECTORS * USER_DATA_SIZE) as usize;
/// Two i32 words (cdpos placeholder, numEntry) precede the entry table.
const ENTRIES_OFFSET: usize = 8;
const ENTRY_SIZE: usize = 8;

/// One subfile in `BIGFILE.BIG`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigEntry {
    /// Offset from the start of BIGFILE.BIG, in 2048-byte sectors.
    pub sector_offset: u32,
    /// Size in bytes.
    pub size: u32,
}

/// Index of the subfiles inside `BIGFILE.BIG`, plus the file's own start sector
/// on the disc (the original game's `BigHeader::cdpos`).
#[derive(Debug, Clone)]
pub struct BigFile {
    pub start_sector: u32,
    pub entries: Vec<BigEntry>,
}

impl BigFile {
    /// Parses the header of `BIGFILE.BIG`, which begins at `start_sector` of
    /// the 2048-byte-sector `disc`.
    pub fn open<R: Read + Seek>(disc: &mut R, start_sector: u32) -> io::Result<Self> {
        let mut buf = vec![0u8; HEADER_SIZE];
        disc.seek(SeekFrom::Start(start_sector as u64 * USER_DATA_SIZE))?;
        disc.read_exact(&mut buf)?;
        Self::parse(&buf, start_sector)
    }

    fn parse(header: &[u8], start_sector: u32) -> io::Result<Self> {
        let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
        let count = word(4) as usize;
        if ENTRIES_OFFSET + count * ENTRY_SIZE > header.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("BIGFILE header claims {count} entries, which overflows the header"),
            ));
        }
        let entries = (0..count)
            .map(|i| {
                let at = ENTRIES_OFFSET + i * ENTRY_SIZE;
                BigEntry {
                    sector_offset: word(at),
                    size: word(at + 4),
                }
            })
            .collect();
        Ok(Self {
            start_sector,
            entries,
        })
    }

    /// Reads subfile `index` in full.
    pub fn read<R: Read + Seek>(&self, disc: &mut R, index: usize) -> io::Result<Vec<u8>> {
        let entry = self.entries.get(index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no BIGFILE entry {index}"))
        })?;
        let sector = self.start_sector as u64 + entry.sector_offset as u64;
        disc.seek(SeekFrom::Start(sector * USER_DATA_SIZE))?;
        let mut data = vec![0u8; entry.size as usize];
        disc.read_exact(&mut data)?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_and_reads_entries() {
        // BIGFILE at sector 2: header (8 sectors), then subfile 0 at +8, subfile 1 at +9.
        let mut disc = vec![0u8; 2 * USER_DATA_SIZE as usize];
        let mut header = vec![0u8; HEADER_SIZE];
        header[4..8].copy_from_slice(&2u32.to_le_bytes());
        for (i, (off, size)) in [(8u32, 3u32), (9, 2)].into_iter().enumerate() {
            header[8 + i * 8..12 + i * 8].copy_from_slice(&off.to_le_bytes());
            header[12 + i * 8..16 + i * 8].copy_from_slice(&size.to_le_bytes());
        }
        disc.extend(header);
        let mut s0 = vec![0u8; USER_DATA_SIZE as usize];
        s0[..3].copy_from_slice(b"abc");
        let mut s1 = vec![0u8; USER_DATA_SIZE as usize];
        s1[..2].copy_from_slice(b"de");
        disc.extend(s0);
        disc.extend(s1);

        let mut disc = Cursor::new(disc);
        let big = BigFile::open(&mut disc, 2).unwrap();
        assert_eq!(big.entries.len(), 2);
        assert_eq!(big.read(&mut disc, 0).unwrap(), b"abc");
        assert_eq!(big.read(&mut disc, 1).unwrap(), b"de");
        assert!(big.read(&mut disc, 2).is_err());
    }
}
