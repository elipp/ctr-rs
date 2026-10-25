use std::collections::HashSet;
use std::io;

/// A subfile loaded with `LT_DRAM`: a 4-byte header giving the offset of a
/// pointer map, the payload, and the pointer map itself at the end.
///
/// ```text
/// i32 ptr_map_offset            (< 0: no pointer map, payload is the rest of the file)
/// payload ...                   (offsets below are relative to its first byte)
/// u32 num_bytes                 (at payload[ptr_map_offset]; slots = num_bytes / 4)
/// i32 slot_offsets[num_bytes/4] (low 2 bits ignored)
/// ```
///
/// Each slot names a 32-bit word in the payload that holds an offset into the
/// payload. The original game adds the payload's address to each of those
/// words (`LOAD_RunPtrMap`). Here the words stay payload-relative, which is
/// the same thing with a base of 0; use [`DramFile::relocate`] to reproduce
/// the original patching for a given base.
#[derive(Debug, Clone)]
pub struct DramFile {
    /// Payload without the header word and without the pointer map.
    pub data: Vec<u8>,
    /// Payload offsets (4-aligned) of the words that are pointers.
    pub pointer_slots: Vec<u32>,
    /// Whether the file had a pointer map at all.
    pub has_pointer_map: bool,
    slot_set: HashSet<u32>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl DramFile {
    pub fn parse(file: &[u8]) -> io::Result<Self> {
        let word = |at: usize| -> io::Result<[u8; 4]> {
            file.get(at..at + 4)
                .map(|b| b.try_into().unwrap())
                .ok_or_else(|| invalid("DRAM file truncated"))
        };

        let map_offset = i32::from_le_bytes(word(0)?);
        let payload = &file[4..];
        if map_offset < 0 {
            return Ok(Self {
                data: payload.to_vec(),
                pointer_slots: Vec::new(),
                has_pointer_map: false,
                slot_set: HashSet::new(),
            });
        }

        let map_offset = map_offset as usize;
        let map = payload
            .get(map_offset..)
            .ok_or_else(|| invalid("DRAM pointer map offset is past end of file"))?;
        let num_bytes = u32::from_le_bytes(
            map.get(..4)
                .ok_or_else(|| invalid("DRAM pointer map header truncated"))?
                .try_into()
                .unwrap(),
        ) as usize;
        let slots = map
            .get(4..4 + num_bytes / 4 * 4)
            .ok_or_else(|| invalid("DRAM pointer map runs past end of file"))?;

        let mut pointer_slots = Vec::with_capacity(slots.len() / 4);
        for raw in slots.as_chunks::<4>().0 {
            let slot = i32::from_le_bytes(*raw) as u32 & !3;
            if slot as usize + 4 > map_offset {
                return Err(invalid(format!(
                    "pointer slot {slot:#x} is outside the payload"
                )));
            }
            pointer_slots.push(slot);
        }

        Ok(Self {
            data: payload[..map_offset].to_vec(),
            slot_set: pointer_slots.iter().copied().collect(),
            pointer_slots,
            has_pointer_map: true,
        })
    }

    /// Whether the word at payload offset `at` is a pointer (listed in the pointer map).
    pub fn is_pointer(&self, at: u32) -> bool {
        self.slot_set.contains(&at)
    }

    /// Reads the payload-relative pointer stored at `slot`.
    pub fn read_u32(&self, at: u32) -> u32 {
        let at = at as usize;
        u32::from_le_bytes(self.data[at..at + 4].try_into().unwrap())
    }

    /// Applies `LOAD_RunPtrMap` for a payload that lives at address `base`:
    /// every pointer slot gets `base` added (wrapping, like the 32-bit original).
    pub fn relocate(&mut self, base: u32) {
        for &slot in &self.pointer_slots {
            let at = slot as usize;
            let v = u32::from_le_bytes(self.data[at..at + 4].try_into().unwrap());
            self.data[at..at + 4].copy_from_slice(&v.wrapping_add(base).to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(payload: &[u8], slots: &[u32]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend((payload.len() as i32).to_le_bytes());
        f.extend(payload);
        f.extend((slots.len() as u32 * 4).to_le_bytes());
        for s in slots {
            f.extend(s.to_le_bytes());
        }
        f
    }

    #[test]
    fn splits_payload_and_pointer_map() {
        let mut payload = vec![0u8; 16];
        payload[4..8].copy_from_slice(&8u32.to_le_bytes()); // points at offset 8
        payload[12..16].copy_from_slice(&0u32.to_le_bytes());
        // Low bits of a slot are ignored (the game masks them off).
        let mut dram = DramFile::parse(&file(&payload, &[4, 12 | 3])).unwrap();
        assert_eq!(dram.data.len(), 16);
        assert_eq!(dram.pointer_slots, [4, 12]);

        dram.relocate(0x8001_0000);
        assert_eq!(dram.read_u32(4), 0x8001_0008);
        assert_eq!(dram.read_u32(12), 0x8001_0000);
    }

    #[test]
    fn negative_offset_means_no_pointer_map() {
        let mut f = (-1i32).to_le_bytes().to_vec();
        f.extend([1, 2, 3, 4]);
        let dram = DramFile::parse(&f).unwrap();
        assert!(!dram.has_pointer_map);
        assert_eq!(dram.data, [1, 2, 3, 4]);
    }

    #[test]
    fn rejects_corrupt_maps() {
        // Map claims more slots than the file holds.
        let mut f = file(&[0; 8], &[0]);
        f[4 + 8..4 + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        assert!(DramFile::parse(&f).is_err());
        // Slot points outside the payload.
        assert!(DramFile::parse(&file(&[0; 8], &[8])).is_err());
    }
}
