use std::io::{self, Read, Seek, SeekFrom};

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

/// Opens the disc image at `path` and parses the header of `BIGFILE.BIG`.
pub fn open_bigfile(path: &std::path::Path) -> io::Result<(Disc, crate::bigfile::BigFile)> {
    open_bigfile_with(|| std::fs::File::open(path).map(std::io::BufReader::new))
}

/// Opens a disc image that `open` makes (as many times as it is asked: once to find
/// `BIGFILE.BIG` and once for the reader that is returned), in the raw 2352-byte sectors of a
/// `.bin`, and parses the header of `BIGFILE.BIG`.
pub fn open_bigfile_with<R: Read + Seek>(
    open: impl Fn() -> io::Result<R>,
) -> io::Result<(RawSectorReader<R>, crate::bigfile::BigFile)> {
    use hadris_iso::read::IsoImage;

    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidData, msg);
    let open = || RawSectorReader::new(open()?);

    let image =
        IsoImage::open(open()?).map_err(|e| invalid(format!("not an ISO 9660 image: {e:?}")))?;
    let start = find(&image, "BIGFILE.BIG")
        .ok_or_else(|| invalid("BIGFILE.BIG not in the root directory".into()))?;

    let mut disc = open()?;
    let big = crate::bigfile::BigFile::open(&mut disc, start)?;
    Ok((disc, big))
}

/// Opens the disc image at `path` and reads the header of `SOUNDS/KART.HWL`.
pub fn open_howl(path: &std::path::Path) -> io::Result<(Disc, crate::howl::Howl)> {
    use hadris_iso::read::IsoImage;

    let open = || RawSectorReader::new(std::fs::File::open(path).map(std::io::BufReader::new)?);
    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidData, msg);
    let image =
        IsoImage::open(open()?).map_err(|e| invalid(format!("not an ISO 9660 image: {e:?}")))?;
    let start = find(&image, "SOUNDS/KART.HWL")
        .ok_or_else(|| invalid("SOUNDS/KART.HWL not on the disc".into()))?;

    let mut disc = open()?;
    let howl = crate::howl::Howl::open(&mut disc, start)?;
    Ok((disc, howl))
}

/// The XA audio of a disc image (`crate::xa`), its files read from their raw sectors.
pub type XaDisc = crate::xa::XaFiles<std::io::BufReader<std::fs::File>>;

/// Opens the disc image at `path` and reads the index of its XA audio, `XA/ENG.XNF`, and where
/// each of the files it names is (`CdSearchFile`, as `CDSYS_SetXAToLang` looks them up).
pub fn open_xa(path: &std::path::Path) -> io::Result<(crate::xa::Xnf, XaDisc)> {
    use crate::xa::{Category, XaFiles, Xnf};
    use hadris_iso::read::IsoImage;

    let file = || std::fs::File::open(path).map(std::io::BufReader::new);
    let invalid = |msg: String| io::Error::new(io::ErrorKind::InvalidData, msg);
    let image = IsoImage::open(RawSectorReader::new(file()?)?)
        .map_err(|e| invalid(format!("not an ISO 9660 image: {e:?}")))?;
    let (start, size) = find_sized(&image, "XA/ENG.XNF")
        .ok_or_else(|| invalid("XA/ENG.XNF not on the disc".into()))?;
    let mut disc = RawSectorReader::new(file()?)?;
    disc.seek(SeekFrom::Start(start as u64 * USER_DATA_SIZE))?;
    let mut bytes = vec![0; size as usize];
    disc.read_exact(&mut bytes)?;
    let xnf = Xnf::parse(&bytes)?;

    let starts = Category::ALL.map(|category| {
        (0..xnf.files[category as usize])
            .map_while(|n| find(&image, &format!("{}/S{n:02}.XA", category.dir())))
            .collect()
    });
    Ok((xnf, XaFiles::new(file()?, starts)))
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
