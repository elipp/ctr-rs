use std::io::{self, Read, Seek, SeekFrom};
use std::io::Write;
use std::path::{Path, PathBuf};

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

/// A `.bin` disc image opened for reading `BIGFILE.BIG`.
pub type Disc = RawSectorReader<std::io::BufReader<std::fs::File>>;

type FileDisc = std::io::BufReader<std::fs::File>;

fn open_file(path: &std::path::Path) -> io::Result<FileDisc> {
    std::fs::File::open(path).map(std::io::BufReader::new)
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// The ISO 9660 filesystem of a disc image that `open` makes (as many times as it is asked), in
/// the raw 2352-byte sectors of a `.bin`.
fn mount<R: Read + Seek>(
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

/// Where the game's files come from: a directory of extracted copies, laid out as on the disc,
/// and the disc image behind it. A file that the directory lacks is extracted from the image the
/// first time it is read; one that it has is read without the image being opened.
///
/// ```text
/// BIGFILE.HDR           the first sector of BIGFILE.BIG (u32), then its header
/// BIGFILE/0123.bin      entry 123 of BIGFILE.BIG
/// SOUNDS/KART.HWL       as on the disc
/// XA/ENG.XNF            as on the disc
/// XA/MUSIC/S00.XA       as on the disc, in the raw 2352-byte sectors of the image
/// ```
///
/// A file is written whole under another name first, so a run that ends half-way leaves none.
/// The extracted files are not checked against the image: delete the directory for another disc.
#[derive(Clone, Debug)]
pub struct Source {
    image: PathBuf,
    dir: PathBuf,
}

impl Source {
    pub fn new(image: impl Into<PathBuf>, dir: impl Into<PathBuf>) -> Self {
        Self {
            image: image.into(),
            dir: dir.into(),
        }
    }

    /// The path of the extracted `name` on the disc; None if the disc has no such file. With
    /// `raw`, the copy is of the image's raw sectors, not of the files's 2048-byte ones.
    fn file(&self, name: &str, raw: bool) -> io::Result<Option<PathBuf>> {
        let path = self.dir.join(name);
        if path.exists() {
            return Ok(Some(path));
        }
        let image = || open_file(&self.image);
        let iso = hadris_iso::read::IsoImage::open(RawSectorReader::new(image()?)?)
            .map_err(|e| invalid(format!("not an ISO 9660 image: {e:?}")))?;
        let Some((start, size)) = find_sized(&iso, name) else {
            return Ok(None);
        };
        let (mut from, len): (Box<dyn Read>, _) = if raw {
            let sectors = (size as u64).div_ceil(USER_DATA_SIZE);
            let mut file = image()?;
            file.seek(SeekFrom::Start(start as u64 * RAW_SECTOR_SIZE))?;
            (Box::new(file), sectors * RAW_SECTOR_SIZE)
        } else {
            let mut disc = RawSectorReader::new(image()?)?;
            disc.seek(SeekFrom::Start(start as u64 * USER_DATA_SIZE))?;
            (Box::new(disc), size as u64)
        };
        write_atomically(&path, |out| {
            let copied = io::copy(&mut from.as_mut().take(len), out)?;
            if copied == len {
                Ok(())
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            }
        })?;
        Ok(Some(path))
    }

    /// `BIGFILE.BIG`: its entries, from the directory or the image.
    pub fn bigfile(&self) -> io::Result<Archive> {
        let header = self.dir.join("BIGFILE.HDR");
        let big = match std::fs::read(&header) {
            Ok(bytes) if bytes.len() > 4 => {
                let start = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                crate::bigfile::BigFile::parse(&bytes[4..], start)?
            }
            _ => {
                let (mut disc, big) = open_bigfile(&self.image)?;
                let mut bytes = big.start_sector.to_le_bytes().to_vec();
                bytes.extend(crate::bigfile::BigFile::read_header(
                    &mut disc,
                    big.start_sector,
                )?);
                write_atomically(&header, |out| out.write_all(&bytes))?;
                big
            }
        };
        Ok(Archive {
            source: self.clone(),
            big,
        })
    }

    /// `SOUNDS/KART.HWL`, opened as a file of its own (it begins at sector 0), and its header.
    pub fn howl(&self) -> io::Result<(std::io::BufReader<std::fs::File>, crate::howl::Howl)> {
        let path = self
            .file("SOUNDS/KART.HWL", false)?
            .ok_or_else(|| invalid("SOUNDS/KART.HWL not on the disc".into()))?;
        let mut file = open_file(&path)?;
        let howl = crate::howl::Howl::open(&mut file, 0)?;
        Ok((file, howl))
    }

    /// The index of the XA audio, `XA/ENG.XNF`, and the XA files it names.
    pub fn xa(&self) -> io::Result<(crate::xa::Xnf, XaFiles)> {
        let path = self
            .file("XA/ENG.XNF", false)?
            .ok_or_else(|| invalid("XA/ENG.XNF not on the disc".into()))?;
        let xnf = crate::xa::Xnf::parse(&std::fs::read(path)?)?;
        let files = XaFiles {
            source: self.clone(),
            counts: xnf.files,
        };
        Ok((xnf, files))
    }
}

/// `BIGFILE.BIG` of a [`Source`].
#[derive(Debug, Clone)]
pub struct Archive {
    source: Source,
    big: crate::bigfile::BigFile,
}

impl Archive {
    pub fn entries(&self) -> &[crate::bigfile::BigEntry] {
        &self.big.entries
    }

    /// Subfile `index` in full, extracted from the image if the directory lacks it.
    pub fn read(&self, index: usize) -> io::Result<Vec<u8>> {
        let path = self.source.dir.join(format!("BIGFILE/{index:04}.bin"));
        if let Ok(bytes) = std::fs::read(&path) {
            return Ok(bytes);
        }
        let mut disc = RawSectorReader::new(open_file(&self.source.image)?)?;
        let bytes = self.big.read(&mut disc, index)?;
        write_atomically(&path, |out| out.write_all(&bytes))?;
        Ok(bytes)
    }
}

/// The XA files of a [`Source`], whose tracks [`XaFiles::read`] reads.
#[derive(Debug, Clone)]
pub struct XaFiles {
    source: Source,
    /// How many files each category has (`Xnf::files`).
    counts: [u32; 3],
}

impl XaFiles {
    /// As `xa::XaFiles::read`: the sectors of `track`; None if its file is not on the disc.
    pub fn read(
        &self,
        category: crate::xa::Category,
        track: crate::xa::Track,
    ) -> io::Result<Option<Vec<u8>>> {
        if track.file >= self.counts[category as usize] as _ {
            return Ok(None);
        }
        let name = format!("{}/S{:02}.XA", category.dir(), track.file);
        let Some(path) = self.source.file(&name, true)? else {
            return Ok(None);
        };
        crate::xa::read_track(&mut open_file(&path)?, 0, track).map(Some)
    }
}

/// Writes `path` (and the directories above it) by `write`, under another name until it is done.
fn write_atomically(
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
fn find<R: Read + Seek>(image: &hadris_iso::read::IsoImage<R>, path: &str) -> Option<u32> {
    find_sized(image, path).map(|(start, _)| start)
}

/// The first sector and the size in bytes of the file at `path`.
fn find_sized<R: Read + Seek>(
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
