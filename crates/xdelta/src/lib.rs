//! The image that an xdelta3 patch makes of another one, as a `Read + Seek` that decodes it when
//! it is read (VCDIFF, RFC 3284). A patch is a list of windows (xdelta3 makes them 8 MiB), each of
//! which makes one stretch of the output from instructions: add bytes, repeat a byte, or copy from
//! the source image or from earlier in the same window. The windows do not depend on each other, so
//! a read decodes the window it lands in, and the last few are kept.
//!
//! Not supported, and refused when the patch is opened: a custom code table, secondary compression
//! (`-S lzma` and the like), and windows that copy from the output of the ones before
//! (`VCD_TARGET`). xdelta3 makes none of them unless it is told to.

use std::io::{self, Read, Seek, SeekFrom};

/// How many decoded windows a reader keeps.
const CACHED_WINDOWS: usize = 3;

/// `VCD_SOURCE`, `VCD_TARGET` and `VCD_ADLER32` of a window's indicator.
const WIN_SOURCE: u8 = 0x1;
const WIN_TARGET: u8 = 0x2;
const WIN_ADLER32: u8 = 0x4;

/// `VCD_DECOMPRESS`, `VCD_CODETABLE` and `VCD_APPHEADER` of the header's indicator.
const HDR_SECONDARY: u8 = 0x1;
const HDR_CODE_TABLE: u8 = 0x2;
const HDR_APP_HEADER: u8 = 0x4;

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// A VCDIFF integer: seven bits at a time, the high bit set on all but the last byte.
fn varint(buf: &[u8], pos: &mut usize) -> io::Result<u64> {
    let mut value = 0u64;
    loop {
        let byte = *buf
            .get(*pos)
            .ok_or_else(|| invalid("patch ends inside a number"))?;
        *pos += 1;
        if value > u64::MAX >> 7 {
            return Err(invalid("number in the patch is too large"));
        }
        value = value << 7 | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
}

/// One instruction of the code table: what it does and the size it carries (0: it is in the
/// instruction stream), then for a copy the address mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Noop,
    Add(u32),
    Run(u32),
    Copy(u32, u8),
}

/// The default code table of RFC 3284 section 5.6: each entry holds up to two instructions.
fn code_table() -> [[Op; 2]; 256] {
    let mut t = [[Op::Noop; 2]; 256];
    let mut i = 0;
    let mut push = |a: Op, b: Op| {
        t[i] = [a, b];
        i += 1;
    };
    push(Op::Run(0), Op::Noop);
    for size in 0..=17 {
        push(Op::Add(size), Op::Noop);
    }
    for mode in 0..9 {
        push(Op::Copy(0, mode), Op::Noop);
        for size in 4..=18 {
            push(Op::Copy(size, mode), Op::Noop);
        }
    }
    for mode in 0..6 {
        for add in 1..=4 {
            for copy in 4..=6 {
                push(Op::Add(add), Op::Copy(copy, mode));
            }
        }
    }
    for mode in 6..9 {
        for add in 1..=4 {
            push(Op::Add(add), Op::Copy(4, mode));
        }
    }
    for mode in 0..9 {
        push(Op::Copy(4, mode), Op::Add(1));
    }
    debug_assert_eq!(i, 256);
    t
}

/// `adler32` of `data`, as the patch's checksums are.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// A window of the patch, found when it was opened.
#[derive(Clone, Debug)]
struct Window {
    /// Where its output is, and how long it is.
    start: u64,
    len: usize,
    /// The stretch of the source it copies from.
    source_len: u64,
    source_start: u64,
    adler32: bool,
    /// The delta encoding in the patch (after its length): the sizes of the three sections, then
    /// the sections.
    body: std::ops::Range<usize>,
}

/// The windows of a patch.
#[derive(Debug)]
struct Index {
    windows: Vec<Window>,
    len: u64,
}

fn index(patch: &[u8]) -> io::Result<Index> {
    if patch.get(..4) != Some(&[0xd6, 0xc3, 0xc4, 0x00]) {
        return Err(invalid("not a VCDIFF patch (xdelta3's)"));
    }
    let indicator = *patch
        .get(4)
        .ok_or_else(|| invalid("patch ends in its header"))?;
    let mut pos = 5;
    if indicator & HDR_SECONDARY != 0 {
        return Err(invalid(
            "the patch uses secondary compression (xdelta3 -S): make it with -S none",
        ));
    }
    if indicator & HDR_CODE_TABLE != 0 {
        return Err(invalid("the patch has a code table of its own"));
    }
    if indicator & HDR_APP_HEADER != 0 {
        let len = varint(patch, &mut pos)? as usize;
        pos = pos
            .checked_add(len)
            .filter(|&p| p <= patch.len())
            .ok_or_else(|| invalid("patch ends in its header"))?;
    }
    let mut windows = Vec::new();
    let mut start = 0;
    while pos < patch.len() {
        let indicator = patch[pos];
        pos += 1;
        if indicator & WIN_TARGET != 0 {
            return Err(invalid("the patch copies from its own output (VCD_TARGET)"));
        }
        if indicator & !(WIN_SOURCE | WIN_ADLER32) != 0 {
            return Err(invalid(format!(
                "window indicator {indicator:#x} is not known"
            )));
        }
        let (mut source_len, mut source_start) = (0, 0);
        if indicator & WIN_SOURCE != 0 {
            source_len = varint(patch, &mut pos)?;
            source_start = varint(patch, &mut pos)?;
        }
        let delta_len = varint(patch, &mut pos)? as usize;
        let end = pos
            .checked_add(delta_len)
            .filter(|&e| e <= patch.len())
            .ok_or_else(|| invalid("patch ends inside a window"))?;
        let mut at = pos;
        let len = varint(patch, &mut at)? as usize;
        windows.push(Window {
            start,
            len,
            source_len,
            source_start,
            adler32: indicator & WIN_ADLER32 != 0,
            body: pos..end,
        });
        start += len as u64;
        pos = end;
    }
    Ok(Index {
        windows,
        len: start,
    })
}

/// The output of a patch applied to `source`, readable and seekable. `source` is read where the
/// patch copies from it.
pub struct Patched<R> {
    source: R,
    patch: Vec<u8>,
    index: Index,
    pos: u64,
    /// The last windows decoded, the least recent first: their number and their bytes.
    cache: Vec<(usize, Vec<u8>)>,
    table: [[Op; 2]; 256],
}

impl<R: Read + Seek> Patched<R> {
    /// The image that `patch` (the bytes of an `.xdelta` file) makes of `source`.
    pub fn new(source: R, patch: Vec<u8>) -> io::Result<Self> {
        let index = index(&patch)?;
        Ok(Patched {
            source,
            patch,
            index,
            pos: 0,
            cache: Vec::new(),
            table: code_table(),
        })
    }

    /// The size of the image.
    pub fn len(&self) -> u64 {
        self.index.len
    }

    pub fn is_empty(&self) -> bool {
        self.index.len == 0
    }

    /// The window that has the byte `pos`.
    fn window_of(&self, pos: u64) -> Option<usize> {
        let at = self
            .index
            .windows
            .partition_point(|w| w.start + w.len as u64 <= pos);
        (at < self.index.windows.len()).then_some(at)
    }

    fn decode(&mut self, number: usize) -> io::Result<Vec<u8>> {
        let window = &self.index.windows[number];
        let body = &self.patch[window.body.clone()];
        let mut at = 0;
        let len = varint(body, &mut at)? as usize;
        let delta = *body
            .get(at)
            .ok_or_else(|| invalid("patch ends in a window"))?;
        at += 1;
        if delta != 0 {
            return Err(invalid("the patch uses secondary compression"));
        }
        let data_len = varint(body, &mut at)? as usize;
        let inst_len = varint(body, &mut at)? as usize;
        let addr_len = varint(body, &mut at)? as usize;
        let checksum = if window.adler32 {
            let bytes = body
                .get(at..at + 4)
                .ok_or_else(|| invalid("patch ends in a window"))?;
            at += 4;
            Some(u32::from_be_bytes(bytes.try_into().unwrap()))
        } else {
            None
        };
        let section = |from: usize, len: usize| {
            body.get(from..from + len)
                .ok_or_else(|| invalid("a window's sections are longer than it"))
        };
        let data = section(at, data_len)?;
        let inst = section(at + data_len, inst_len)?;
        let addr = section(at + data_len + inst_len, addr_len)?;

        let mut out: Vec<u8> = Vec::with_capacity(len);
        let (mut data_at, mut inst_at, mut addr_at) = (0, 0, 0);
        // The address caches of section 5.3, which start empty in every window.
        let mut near = [0u64; 4];
        let mut next_near = 0;
        let mut same = [0u64; 768];
        while inst_at < inst.len() {
            let code = inst[inst_at];
            inst_at += 1;
            for op in self.table[code as usize] {
                let sized = |size: u32, inst_at: &mut usize| -> io::Result<usize> {
                    Ok(if size == 0 {
                        varint(inst, inst_at)? as usize
                    } else {
                        size as usize
                    })
                };
                match op {
                    Op::Noop => {}
                    Op::Add(size) => {
                        let size = sized(size, &mut inst_at)?;
                        let bytes = data
                            .get(data_at..data_at + size)
                            .ok_or_else(|| invalid("an add runs past the data"))?;
                        out.extend_from_slice(bytes);
                        data_at += size;
                    }
                    Op::Run(size) => {
                        let size = sized(size, &mut inst_at)?;
                        let byte = *data
                            .get(data_at)
                            .ok_or_else(|| invalid("a run has no byte"))?;
                        data_at += 1;
                        out.resize(out.len() + size, byte);
                    }
                    Op::Copy(size, mode) => {
                        let size = sized(size, &mut inst_at)?;
                        let here = window.source_len + out.len() as u64;
                        let address = match mode {
                            0 => varint(addr, &mut addr_at)?,
                            1 => here
                                .checked_sub(varint(addr, &mut addr_at)?)
                                .ok_or_else(|| invalid("a copy reaches before the window"))?,
                            2..=5 => near[mode as usize - 2] + varint(addr, &mut addr_at)?,
                            _ => {
                                let byte = *addr
                                    .get(addr_at)
                                    .ok_or_else(|| invalid("patch ends in the addresses"))?;
                                addr_at += 1;
                                same[(mode as usize - 6) * 256 + byte as usize]
                            }
                        };
                        near[next_near] = address;
                        next_near = (next_near + 1) % 4;
                        same[(address % 768) as usize] = address;

                        if address < window.source_len {
                            let end = address + size as u64;
                            if end > window.source_len {
                                return Err(invalid("a copy runs past the source segment"));
                            }
                            let from = out.len();
                            out.resize(from + size, 0);
                            self.source
                                .seek(SeekFrom::Start(window.source_start + address))?;
                            self.source.read_exact(&mut out[from..])?;
                        } else {
                            // From earlier in the window, where the copy may run into its own
                            // output: one byte at a time.
                            let mut from = (address - window.source_len) as usize;
                            if from >= out.len() {
                                return Err(invalid("a copy reaches past the window"));
                            }
                            for _ in 0..size {
                                out.push(out[from]);
                                from += 1;
                            }
                        }
                    }
                }
            }
        }
        if out.len() != len {
            return Err(invalid(format!(
                "a window made {} bytes, not the {len} it says",
                out.len()
            )));
        }
        if let Some(expected) = checksum
            && adler32(&out) != expected
        {
            return Err(invalid(format!(
                "window {number} does not match its checksum"
            )));
        }
        Ok(out)
    }

    /// The bytes of window `number`, from the cache or decoded.
    fn window(&mut self, number: usize) -> io::Result<&[u8]> {
        if let Some(at) = self.cache.iter().position(|(n, _)| *n == number) {
            let entry = self.cache.remove(at);
            self.cache.push(entry);
        } else {
            let bytes = self.decode(number)?;
            if self.cache.len() == CACHED_WINDOWS {
                self.cache.remove(0);
            }
            self.cache.push((number, bytes));
        }
        Ok(&self.cache.last().unwrap().1)
    }
}

impl<R: Read + Seek> Read for Patched<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(number) = self.window_of(self.pos) else {
            return Ok(0);
        };
        let start = self.index.windows[number].start;
        let within = (self.pos - start) as usize;
        let bytes = self.window(number)?;
        let n = buf.len().min(bytes.len() - within);
        buf[..n].copy_from_slice(&bytes[within..within + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Patched<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let new = match from {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.index.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        }
        .ok_or_else(|| invalid("seek before the start"))?;
        self.pos = new;
        Ok(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn number(mut n: u64) -> Vec<u8> {
        let mut out = vec![(n & 0x7f) as u8];
        n >>= 7;
        while n > 0 {
            out.push((n & 0x7f) as u8 | 0x80);
            n >>= 7;
        }
        out.reverse();
        out
    }

    /// One instruction, spelled out with the general codes of the table (the sizes in the stream).
    enum Step<'a> {
        Add(&'a [u8]),
        Run(u8, u64),
        /// Mode 0: the address as it is.
        Copy(u64, u64),
        /// Mode 1: back from the end of what is made so far.
        CopyHere(u64, u64),
    }

    /// A window with `source` as its segment (`len`, `start`) and these steps.
    fn window(source: Option<(u64, u64)>, steps: &[Step], checksum: Option<u32>) -> Vec<u8> {
        let (mut data, mut inst, mut addr) = (Vec::new(), Vec::new(), Vec::new());
        let mut made = 0u64;
        for step in steps {
            match step {
                Step::Add(bytes) => {
                    inst.push(1);
                    inst.extend(number(bytes.len() as u64));
                    data.extend_from_slice(bytes);
                    made += bytes.len() as u64;
                }
                Step::Run(byte, size) => {
                    inst.push(0);
                    inst.extend(number(*size));
                    data.push(*byte);
                    made += size;
                }
                Step::Copy(at, size) => {
                    inst.push(19);
                    inst.extend(number(*size));
                    addr.extend(number(*at));
                    made += size;
                }
                Step::CopyHere(back, size) => {
                    inst.push(19 + 16);
                    inst.extend(number(*size));
                    addr.extend(number(*back));
                    made += size;
                }
            }
        }
        let mut body = number(made);
        body.push(0);
        body.extend(number(data.len() as u64));
        body.extend(number(inst.len() as u64));
        body.extend(number(addr.len() as u64));
        if let Some(sum) = checksum {
            body.extend(sum.to_be_bytes());
        }
        body.extend(data);
        body.extend(inst);
        body.extend(addr);
        let mut out =
            vec![source.is_some() as u8 | if checksum.is_some() { WIN_ADLER32 } else { 0 }];
        if let Some((len, start)) = source {
            out.extend(number(len));
            out.extend(number(start));
        }
        out.extend(number(body.len() as u64));
        out.extend(body);
        out
    }

    fn patch(windows: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0xd6, 0xc3, 0xc4, 0x00, 0x00];
        for w in windows {
            out.extend_from_slice(w);
        }
        out
    }

    fn read_all(r: &mut impl Read) -> Vec<u8> {
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn the_table_has_the_instructions_of_the_rfc() {
        let t = code_table();
        assert_eq!(t[0], [Op::Run(0), Op::Noop]);
        assert_eq!(t[1], [Op::Add(0), Op::Noop]);
        assert_eq!(t[18], [Op::Add(17), Op::Noop]);
        assert_eq!(t[19], [Op::Copy(0, 0), Op::Noop]);
        assert_eq!(t[162], [Op::Copy(18, 8), Op::Noop]);
        assert_eq!(t[163], [Op::Add(1), Op::Copy(4, 0)]);
        assert_eq!(t[234], [Op::Add(4), Op::Copy(6, 5)]);
        assert_eq!(t[247], [Op::Copy(4, 0), Op::Add(1)]);
        assert_eq!(t[255], [Op::Copy(4, 8), Op::Add(1)]);
    }

    #[test]
    fn a_window_adds_runs_and_copies() {
        let source = b"hello world".to_vec();
        let sum = adler32(b"hello, hello!!!!!!ello");
        let steps = [
            Step::Copy(0, 5),
            Step::Add(b", "),
            Step::Copy(0, 5),
            Step::Add(b"!"),
            Step::Run(b'!', 5),
            // From the output: "ello" of the second hello, 10 back from the end of it.
            Step::CopyHere(10, 4),
        ];
        let w = window(Some((11, 0)), &steps, Some(sum));
        let mut patched = Patched::new(Cursor::new(source), patch(&[w])).unwrap();
        assert_eq!(patched.len(), 22);
        assert_eq!(read_all(&mut patched), b"hello, hello!!!!!!ello");
    }

    #[test]
    fn a_copy_may_run_into_its_own_output() {
        let w = window(None, &[Step::Add(b"ab"), Step::CopyHere(2, 7)], None);
        let mut patched = Patched::new(Cursor::new(Vec::new()), patch(&[w])).unwrap();
        assert_eq!(read_all(&mut patched), b"ababababa");
    }

    #[test]
    fn a_read_lands_in_the_window_it_seeks_to() {
        let source: Vec<u8> = (0..100).collect();
        let a = window(Some((100, 0)), &[Step::Copy(10, 20)], None);
        let b = window(Some((50, 50)), &[Step::Add(b"xyz"), Step::Copy(0, 5)], None);
        let c = window(None, &[Step::Run(7, 4)], None);
        let mut patched = Patched::new(Cursor::new(source), patch(&[a, b, c])).unwrap();
        assert_eq!(patched.len(), 20 + 8 + 4);
        patched.seek(SeekFrom::Start(22)).unwrap();
        let mut buf = [0; 4];
        patched.read_exact(&mut buf).unwrap();
        // "xyz" then the source segment's first byte, which is the image's byte 50.
        assert_eq!(buf, [b'z', 50, 51, 52]);
        // Across the end of a window and the start of the next.
        patched.seek(SeekFrom::Start(26)).unwrap();
        let mut buf = [0; 6];
        patched.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [53, 54, 7, 7, 7, 7]);
        // And back to the first.
        patched.seek(SeekFrom::Start(0)).unwrap();
        patched.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [10, 11, 12, 13, 14, 15]);
        // The end.
        patched.seek(SeekFrom::End(0)).unwrap();
        assert_eq!(patched.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn a_wrong_checksum_is_caught() {
        let w = window(None, &[Step::Add(b"abc")], Some(adler32(b"abd")));
        let mut patched = Patched::new(Cursor::new(Vec::new()), patch(&[w])).unwrap();
        let error = patched.read(&mut [0; 3]).unwrap_err();
        assert!(error.to_string().contains("checksum"), "{error}");
    }

    #[test]
    fn what_is_not_supported_is_refused_when_the_patch_opens() {
        let open = |bytes: Vec<u8>| Patched::new(Cursor::new(Vec::new()), bytes).err();
        assert!(open(b"not a patch".to_vec()).is_some());
        // Secondary compression, a code table, a copy from the output.
        assert!(open(vec![0xd6, 0xc3, 0xc4, 0x00, 0x01, 0x00]).is_some());
        assert!(open(vec![0xd6, 0xc3, 0xc4, 0x00, 0x02, 0x00]).is_some());
        assert!(open(vec![0xd6, 0xc3, 0xc4, 0x00, 0x00, 0x02, 0x00, 0x00]).is_some());
        // A header with an application header and no windows is an empty image.
        let mut with_header = vec![0xd6, 0xc3, 0xc4, 0x00, 0x04, 0x03];
        with_header.extend(b"app");
        let patched = Patched::new(Cursor::new(Vec::new()), with_header).unwrap();
        assert!(patched.is_empty());
    }

    #[test]
    fn adler32_is_the_rfc_1950_one() {
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        assert_eq!(adler32(b""), 1);
    }
}
