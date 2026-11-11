use std::fs::File;
use std::io::Write;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Size of a raw CD-ROM sector in a `.bin` image.
pub const RAW_SECTOR_SIZE: u64 = 2352;
/// User data bytes per Mode 2 Form 1 sector (and per ISO 9660 logical block).
pub const USER_DATA_SIZE: u64 = 2048;
/// 12 byte sync + 4 byte header + 8 byte Mode 2 subheader.
const USER_DATA_OFFSET: u64 = 24;

/// Presents a raw 2352-byte-sector PS1 `.bin` image as a flat 2048-byte-sector
/// ISO 9660 image, so it can be handed to ISO readers that expect `.iso` layout.
pub struct RawSectorReader<R> {
    inner: R,
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> RawSectorReader<R> {
    pub fn new(mut inner: R) -> io::Result<Self> {
        let raw_len = inner.seek(SeekFrom::End(0))?;
        Ok(Self {
            inner,
            pos: 0,
            len: raw_len / RAW_SECTOR_SIZE * USER_DATA_SIZE,
        })
    }
}

impl<R: Read + Seek> Read for RawSectorReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.len.saturating_sub(self.pos);
        let want = (buf.len() as u64).min(remaining) as usize;
        let mut done = 0;
        while done < want {
            let sector = self.pos / USER_DATA_SIZE;
            let within = self.pos % USER_DATA_SIZE;
            let n = ((USER_DATA_SIZE - within) as usize).min(want - done);
            self.inner.seek(SeekFrom::Start(
                sector * RAW_SECTOR_SIZE + USER_DATA_OFFSET + within,
            ))?;
            self.inner.read_exact(&mut buf[done..done + n])?;
            self.pos += n as u64;
            done += n;
        }
        Ok(done)
    }
}

impl<R: Read + Seek> Seek for RawSectorReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let new = match from {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        self.pos = new;
        Ok(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn strips_sector_framing() {
        let mut raw = Vec::new();
        for s in 0..3u8 {
            raw.extend([0xAA; USER_DATA_OFFSET as usize]);
            raw.extend([s; USER_DATA_SIZE as usize]);
            raw.extend([0xBB; (RAW_SECTOR_SIZE - USER_DATA_OFFSET - USER_DATA_SIZE) as usize]);
        }
        let mut r = RawSectorReader::new(Cursor::new(raw)).unwrap();
        assert_eq!(r.seek(SeekFrom::End(0)).unwrap(), 3 * USER_DATA_SIZE);

        // Read across the boundary between sector 0 and sector 1.
        r.seek(SeekFrom::Start(USER_DATA_SIZE - 2)).unwrap();
        let mut buf = [0u8; 4];
        r.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [0, 0, 1, 1]);
    }
}

/// A `.bin` (or CHD) disc image opened for reading `BIGFILE.BIG`.
pub type Disc = RawSectorReader<FileDisc>;

/// Reads `buf.len()` bytes of a disc image from byte `at` on: how [`keep_external`] reads it.
pub type ReadAt = fn(at: u64, buf: &mut [u8]) -> io::Result<()>;

/// A disc image that the program does not hold itself, which [`open_file`] opens in place of the
/// file at its path: its size, and how it is read. A browser has no files: the page holds the
/// image the player dropped on it, and the program reads the parts of it that it needs (a wasm
/// program's memory never shrinks, so the image is not copied into it).
static EXTERNAL: OnceLock<(PathBuf, u64, ReadAt)> = OnceLock::new();

/// Whether [`release_external`] has been called: the image is gone, and reading it fails.
static RELEASED: AtomicBool = AtomicBool::new(false);

/// Reads the disc image at `path`, of `len` bytes, by `read`, for as long as the program runs or
/// until [`release_external`]. Once only.
pub fn keep_external(path: PathBuf, len: u64, read: ReadAt) {
    if EXTERNAL.set((path, len, read)).is_err() {
        panic!("an external disc image is kept once");
    }
}

/// The path of the disc image that is [read externally](keep_external), if one is.
pub fn external() -> Option<PathBuf> {
    EXTERNAL.get().map(|(path, ..)| path.clone())
}

/// Says that the [external](keep_external) image is gone: a read of it fails from then on.
pub fn release_external() {
    RELEASED.store(true, Ordering::Relaxed);
}

/// A disc image opened: the file, or the image [read externally](keep_external) at its path. Either
/// may be a CHD, which is then read as the `.bin` it was made from.
pub enum FileDisc {
    File(BufReader<File>),
    External { pos: u64, len: u64, read: ReadAt },
    Chd(Box<ChdDisc<FileDisc>>),
}

/// The first bytes of a CHD.
const CHD_MAGIC: &[u8; 8] = b"MComprHD";

/// The bytes a CHD keeps of each CD sector: the raw sector and its 96 bytes of subcode.
const CHD_FRAME_SIZE: u64 = RAW_SECTOR_SIZE + 96;

/// A CHD of a CD, read as the `.bin` it was made from: the raw 2352-byte sectors, one after
/// another, without their subcode. The hunks are decompressed as they are read, the last one kept.
///
/// Only the first track is read correctly to its end: a CHD pads each track to a multiple of 4
/// sectors, so the sectors of any after it are not where they are in a `.bin` (the game's
/// files are all in the first).
pub struct ChdDisc<R: Read + Seek> {
    chd: chd::Chd<R>,
    hunk_size: u64,
    /// The hunk in `hunk`, if one is.
    current: Option<u32>,
    hunk: Vec<u8>,
    /// The compressed hunk, as it is read.
    compressed: Vec<u8>,
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> ChdDisc<R> {
    pub fn new(inner: R) -> io::Result<Self> {
        let chd = chd::Chd::open(inner, None).map_err(|e| invalid(format!("not a CHD: {e}")))?;
        let hunk_count = chd.header().hunk_count() as u64;
        let hunk_size = chd.header().hunk_size() as u64;
        if hunk_size == 0 || hunk_size % CHD_FRAME_SIZE != 0 {
            return Err(invalid(format!(
                "not a CHD of a CD: its hunks are {hunk_size} bytes, not whole sectors of {CHD_FRAME_SIZE}"
            )));
        }
        let hunk = chd.get_hunksized_buffer();
        Ok(Self {
            chd,
            hunk_size,
            current: None,
            hunk,
            compressed: Vec::new(),
            pos: 0,
            len: hunk_count * hunk_size / CHD_FRAME_SIZE * RAW_SECTOR_SIZE,
        })
    }
}

impl<R: Read + Seek> Read for ChdDisc<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut done = 0;
        while done < buf.len() && self.pos < self.len {
            let sector = self.pos / RAW_SECTOR_SIZE;
            let within = self.pos % RAW_SECTOR_SIZE;
            // A hunk holds whole sectors, so a sector is never split between two.
            let at = sector * CHD_FRAME_SIZE + within;
            let hunk = (at / self.hunk_size) as u32;
            let offset = (at % self.hunk_size) as usize;
            if self.current != Some(hunk) {
                self.current = None;
                self.chd
                    .hunk(hunk)
                    .and_then(|mut h| h.read_hunk_in(&mut self.compressed, &mut self.hunk))
                    .map_err(|e| invalid(format!("CHD hunk {hunk}: {e}")))?;
                self.current = Some(hunk);
            }
            let n = ((RAW_SECTOR_SIZE - within) as usize).min(buf.len() - done);
            buf[done..done + n].copy_from_slice(&self.hunk[offset..offset + n]);
            self.pos += n as u64;
            done += n;
        }
        Ok(done)
    }
}

impl<R: Read + Seek> Seek for ChdDisc<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let new = match from {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        self.pos = new;
        Ok(new)
    }
}

impl Read for FileDisc {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            FileDisc::File(file) => file.read(buf),
            FileDisc::Chd(chd) => chd.read(buf),
            FileDisc::External { pos, len, read } => {
                if RELEASED.load(Ordering::Relaxed) {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "the disc image was let go of once everything was taken from it",
                    ));
                }
                let n = (buf.len() as u64).min(len.saturating_sub(*pos)) as usize;
                read(*pos, &mut buf[..n])?;
                *pos += n as u64;
                Ok(n)
            }
        }
    }
}

impl Seek for FileDisc {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        match self {
            FileDisc::File(file) => file.seek(from),
            FileDisc::Chd(chd) => chd.seek(from),
            FileDisc::External { pos, len, .. } => {
                let new = match from {
                    SeekFrom::Start(p) => Some(p),
                    SeekFrom::End(d) => len.checked_add_signed(d),
                    SeekFrom::Current(d) => pos.checked_add_signed(d),
                }
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
                *pos = new;
                Ok(new)
            }
        }
    }
}

/// Opens the disc image at `path` (a `.bin`, or a CHD, told by its first bytes) as the bytes of a
/// `.bin`.
pub fn open_file(path: &Path) -> io::Result<FileDisc> {
    let mut disc = match EXTERNAL.get() {
        Some(&(ref at, len, read)) if at == path => FileDisc::External { pos: 0, len, read },
        _ => FileDisc::File(BufReader::new(File::open(path)?)),
    };
    let mut magic = [0; CHD_MAGIC.len()];
    let is_chd = match disc.read_exact(&mut magic) {
        Ok(()) => &magic == CHD_MAGIC,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => false,
        Err(e) => return Err(e),
    };
    disc.seek(SeekFrom::Start(0))?;
    if is_chd {
        Ok(FileDisc::Chd(Box::new(ChdDisc::new(disc)?)))
    } else {
        Ok(disc)
    }
}

pub(crate) fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// The ISO 9660 filesystem of a disc image that `open` makes (as many times as it is asked), in
/// the raw 2352-byte sectors of a `.bin`.
pub(crate) fn mount<R: Read + Seek>(
    open: &impl Fn() -> io::Result<R>,
) -> io::Result<hadris_iso::read::IsoImage<RawSectorReader<R>>> {
    hadris_iso::read::IsoImage::open(RawSectorReader::new(open()?)?)
        .map_err(|e| invalid(format!("not an ISO 9660 image: {e:?}")))
}

/// Opens the disc image at `path` and parses the header of `BIGFILE.BIG`.
pub fn open_bigfile(path: &std::path::Path) -> io::Result<(Disc, crate::bigfile::BigFile)> {
    open_bigfile_with(|| open_file(path))
}

/// Opens a disc image that `open` makes (as many times as it is asked: once to find
/// `BIGFILE.BIG` and once for the reader that is returned), in the raw 2352-byte sectors of a
/// `.bin`, and parses the header of `BIGFILE.BIG`.
pub fn open_bigfile_with<R: Read + Seek>(
    open: impl Fn() -> io::Result<R>,
) -> io::Result<(RawSectorReader<R>, crate::bigfile::BigFile)> {
    let start = find(&mount(&open)?, "BIGFILE.BIG")
        .ok_or_else(|| invalid("BIGFILE.BIG not in the root directory".into()))?;

    let mut disc = RawSectorReader::new(open()?)?;
    let big = crate::bigfile::BigFile::open(&mut disc, start)?;
    Ok((disc, big))
}

/// The bytes of the file at `path` (directories split by `/`) of a disc image that `open` makes,
/// as `open_bigfile_with`; `None` if the disc has no such file.
pub fn read_file_with<R: Read + Seek>(
    open: impl Fn() -> io::Result<R>,
    path: &str,
) -> io::Result<Option<Vec<u8>>> {
    let Some((start, size)) = find_sized(&mount(&open)?, path) else {
        return Ok(None);
    };
    let mut disc = RawSectorReader::new(open()?)?;
    disc.seek(SeekFrom::Start(start as u64 * USER_DATA_SIZE))?;
    let mut bytes = vec![0; size as usize];
    disc.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

/// Opens the disc image at `path` and reads the header of `SOUNDS/KART.HWL`.
pub fn open_howl(path: &std::path::Path) -> io::Result<(Disc, crate::howl::Howl)> {
    open_howl_with(|| open_file(path))
}

/// `open_howl` for a disc image that `open` makes, as `open_bigfile_with`.
pub fn open_howl_with<R: Read + Seek>(
    open: impl Fn() -> io::Result<R>,
) -> io::Result<(RawSectorReader<R>, crate::howl::Howl)> {
    let start = find(&mount(&open)?, "SOUNDS/KART.HWL")
        .ok_or_else(|| invalid("SOUNDS/KART.HWL not on the disc".into()))?;

    let mut disc = RawSectorReader::new(open()?)?;
    let howl = crate::howl::Howl::open(&mut disc, start)?;
    Ok((disc, howl))
}

/// The XA audio of a disc image (`crate::xa`), its files read from their raw sectors.
pub type XaDisc = crate::xa::XaFiles<FileDisc>;

/// Opens the disc image at `path` and reads the index of its XA audio, `XA/ENG.XNF`, and where
/// each of the files it names is (`CdSearchFile`, as `CDSYS_SetXAToLang` looks them up).
pub fn open_xa(path: &std::path::Path) -> io::Result<(crate::xa::Xnf, XaDisc)> {
    open_xa_with(|| open_file(path))
}

/// `open_xa` for a disc image that `open` makes, as `open_bigfile_with`.
pub fn open_xa_with<R: Read + Seek>(
    open: impl Fn() -> io::Result<R>,
) -> io::Result<(crate::xa::Xnf, crate::xa::XaFiles<R>)> {
    use crate::xa::{Category, XaFiles, Xnf};

    let image = mount(&open)?;
    let (start, size) = find_sized(&image, "XA/ENG.XNF")
        .ok_or_else(|| invalid("XA/ENG.XNF not on the disc".into()))?;
    let mut disc = RawSectorReader::new(open()?)?;
    disc.seek(SeekFrom::Start(start as u64 * USER_DATA_SIZE))?;
    let mut bytes = vec![0; size as usize];
    disc.read_exact(&mut bytes)?;
    let xnf = Xnf::parse(&bytes)?;

    let starts = Category::ALL.map(|category| {
        (0..xnf.files[category as usize])
            .map_while(|n| find(&image, &format!("{}/S{n:02}.XA", category.dir())))
            .collect()
    });
    Ok((xnf, XaFiles::new(open()?, starts)))
}

/// Writes `path` (and the directories above it) by `write`, under another name until it is done.
pub(crate) fn write_atomically(
    path: &Path,
    write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&part)?);
    let done = write(&mut out).and_then(|()| out.flush());
    drop(out);
    match done {
        Ok(()) => std::fs::rename(&part, path),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// The first sector of the file at `path` (directories split by `/`, no version suffix).
pub(crate) fn find<R: Read + Seek>(
    image: &hadris_iso::read::IsoImage<R>,
    path: &str,
) -> Option<u32> {
    find_sized(image, path).map(|(start, _)| start)
}

/// The first sector and the size in bytes of the file at `path`.
pub(crate) fn find_sized<R: Read + Seek>(
    image: &hadris_iso::read::IsoImage<R>,
    path: &str,
) -> Option<(u32, u32)> {
    let mut dir = image.root_dir().iter(image);
    let mut names = path.split('/').peekable();
    while let Some(name) = names.next() {
        let entry = dir
            .entries()
            .filter_map(Result::ok)
            .find(|e| e.display_name().split(';').next() == Some(name))?;
        if names.peek().is_none() {
            let header = entry.header();
            return Some((header.extent.read(), header.data_len.read()));
        }
        dir = image.open_dir(entry.as_dir_ref(image).ok()?);
    }
    None
}
