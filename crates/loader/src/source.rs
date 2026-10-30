//! Everything the game reads from the disc, and where it reads it from.
//!
//! The game asks for a [`Need`], and a [`Source`] answers with its bytes: from a folder of files
//! it made from the disc image earlier, or, if the folder lacks the file and the source has an
//! image, from the image (and the file is kept there from then on). A source without an image is
//! how the game runs once everything is prepared: a file the folder lacks is an error that says
//! which.
//!
//! [`Need::all`] is every need there can be, which [`Source::prepare`] makes the files of. All
//! reads of the disc go through a need, so that list is the whole of what the game can ask for.
//!
//! ```text
//! racer/03        the model of character 3 (BIGFILE.BIG, RACER_MODEL_HI)
//! pack/03         its pack: icons, fonts and the shared models (ARCADE_1P_PACK)
//! shared.vrm      the texture pages every track shares (SHARED_MPK_VRM)
//! KART.HWL        SOUNDS/KART.HWL, as on the disc
//! ENG.XNF         XA/ENG.XNF, as on the disc
//! xa/music/006    one XA track: the sectors of it, as `xa::read_track` makes them
//! ```
//!
//! `MANIFEST` is written last by [`Source::prepare`], and says what it made: the layout of the
//! folder ([`LAYOUT`]), the size of the image, and the size of each file. [`Source::check`] reads
//! it with the folder, and does not open the image. A folder with no manifest was never finished.
//!
//! The levels are not here: they are maps (`.ctrmap`), made from the disc by the `map` crate.

use std::io::{self, Cursor, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use hadris_iso::read::IsoImage;

use crate::bigfile::BigFile;
use crate::bigindex::BigIndex;
use crate::disc::{
    Disc, FileDisc, RawSectorReader, USER_DATA_SIZE, find, find_sized, invalid, mount, open_bigfile,
    open_file, write_atomically,
};
use crate::howl::Howl;
use crate::xa::{Category, Xnf, read_track};

/// The version of the layout of the folder: the names and contents of the files. A folder of
/// another is made again.
pub const LAYOUT: u32 = 1;

/// The manifest's name in the folder.
const MANIFEST: &str = "MANIFEST";

/// What [`Source::check`] finds of the folder.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    /// Every need has its file, as the manifest says.
    Ready,
    /// There is no manifest: nothing was made, or it did not finish.
    NotPrepared,
    /// The folder is of another layout or another image; it is made again from scratch.
    Stale(String),
    /// These needs have no file, or one of another size.
    Missing(Vec<Need>),
}

/// How many characters there are (`GAME_CHARACTER_IDS`): the models and packs have a file each.
pub const CHARACTERS: u32 = 16;

/// A file of the disc, or a part of one, that the game reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// The model of a character.
    Racer(u32),
    /// The pack of a character.
    Pack(u32),
    /// The texture pages that every track shares.
    SharedVram,
    /// `SOUNDS/KART.HWL`: the sound effects, engines and songs.
    Howl,
    /// `XA/ENG.XNF`: where the XA tracks are.
    Xnf,
    /// Track `id` of a category of XA audio.
    Xa(Category, u16),
}

impl Need {
    /// The name of its file in the folder of a source.
    pub fn key(self) -> String {
        match self {
            Need::Racer(c) => format!("racer/{c:02}"),
            Need::Pack(c) => format!("pack/{c:02}"),
            Need::SharedVram => "shared.vrm".into(),
            Need::Howl => "KART.HWL".into(),
            Need::Xnf => "ENG.XNF".into(),
            Need::Xa(category, id) => {
                let dir = match category {
                    Category::Music => "music",
                    Category::Extra => "extra",
                    Category::Game => "game",
                };
                format!("xa/{dir}/{id:03}")
            }
        }
    }

    /// Every need there is, given the index of the XA (which is itself [`Need::Xnf`]).
    pub fn all(xnf: &Xnf) -> Vec<Need> {
        let mut all = vec![Need::Xnf, Need::Howl, Need::SharedVram];
        all.extend((0..CHARACTERS).map(Need::Racer));
        all.extend((0..CHARACTERS).map(Need::Pack));
        for category in Category::ALL {
            let tracks = xnf.tracks[category as usize].len() as u16;
            all.extend(
                (0..tracks)
                    .filter(|&id| xnf.track(category, id).is_some())
                    .map(|id| Need::Xa(category, id)),
            );
        }
        all
    }
}

/// Where the needs are answered from: a folder, and the disc image behind it if there is one.
#[derive(Clone, Debug)]
pub struct Source {
    dir: PathBuf,
    image: Option<PathBuf>,
}

impl Source {
    pub fn new(dir: impl Into<PathBuf>, image: Option<PathBuf>) -> Source {
        Source {
            dir: dir.into(),
            image,
        }
    }

    fn path(&self, need: Need) -> PathBuf {
        self.dir.join(need.key())
    }

    /// Whether the folder has the file of `need`.
    pub fn has(&self, need: Need) -> bool {
        self.path(need).is_file()
    }

    /// The bytes of `need`. A file that the folder lacks is taken from the image and kept (if it
    /// cannot be kept, it is still returned: the next read takes it from the image again).
    pub fn get(&self, need: Need) -> io::Result<Vec<u8>> {
        match std::fs::read(self.path(need)) {
            Ok(bytes) => return Ok(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let xnf = if matches!(need, Need::Xa(..)) {
            self.xnf()?
        } else {
            Xnf::default()
        };
        let bytes = Image::open(self.image_for(need)?)?.extract(need, &xnf)?;
        let _ = self.store(need, &bytes);
        Ok(bytes)
    }

    fn image_for(&self, need: Need) -> io::Result<&Path> {
        self.image.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "{} is not in {}, and there is no disc image to make it from",
                    need.key(),
                    self.dir.display()
                ),
            )
        })
    }

    fn store(&self, need: Need, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;
        write_atomically(&self.path(need), |out| out.write_all(bytes))
    }

    /// Whether the folder has everything: reads the manifest and looks at the files, not at the
    /// image's contents.
    pub fn check(&self) -> Check {
        let Ok(text) = std::fs::read_to_string(self.dir.join(MANIFEST)) else {
            return Check::NotPrepared;
        };
        let Some(manifest) = Manifest::parse(&text) else {
            return Check::Stale("the manifest is of another layout".into());
        };
        if let Some(image) = &self.image
            && let Ok(meta) = std::fs::metadata(image)
            && manifest.image_size != 0
            && meta.len() != manifest.image_size
        {
            return Check::Stale(format!(
                "the files were made from an image of {} bytes, this one has {}",
                manifest.image_size,
                meta.len()
            ));
        }
        let Some(xnf) = std::fs::read(self.path(Need::Xnf))
            .ok()
            .and_then(|bytes| Xnf::parse(&bytes).ok())
        else {
            return Check::Stale("ENG.XNF cannot be read".into());
        };
        let missing: Vec<Need> = Need::all(&xnf)
            .into_iter()
            .filter(|&need| {
                let on_disk = std::fs::metadata(self.path(need)).map(|m| m.len());
                manifest.size_of(&need.key()) != on_disk.ok()
            })
            .collect();
        if missing.is_empty() {
            Check::Ready
        } else {
            Check::Missing(missing)
        }
    }

    /// Deletes what a source made, the manifest first (the maps made from the disc are not ours
    /// to delete here).
    fn wipe(&self) -> io::Result<()> {
        let gone = |r: io::Result<()>| match r {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
        gone(std::fs::remove_file(self.dir.join(MANIFEST)))?;
        for file in ["shared.vrm", "KART.HWL", "ENG.XNF"] {
            gone(std::fs::remove_file(self.dir.join(file)))?;
        }
        for dir in ["racer", "pack", "xa"] {
            gone(std::fs::remove_dir_all(self.dir.join(dir)))?;
        }
        Ok(())
    }

    /// `XA/ENG.XNF`, parsed.
    pub fn xnf(&self) -> io::Result<Xnf> {
        Xnf::parse(&self.get(Need::Xnf)?)
    }

    /// `SOUNDS/KART.HWL`, as a file that begins at sector 0, and its header.
    pub fn howl(&self) -> io::Result<(Cursor<Vec<u8>>, Howl)> {
        let mut bytes = self.get(Need::Howl)?;
        // The file's size is in bytes, but it is read in sectors: the last song pack ends in a
        // sector that the file only part fills.
        bytes.resize(bytes.len().next_multiple_of(USER_DATA_SIZE as usize), 0);
        let mut file = Cursor::new(bytes);
        let howl = Howl::open(&mut file, 0)?;
        Ok((file, howl))
    }

    /// Makes the file of every need that the folder lacks, from the image, and the manifest last.
    /// It tells how far it is (`done` of `of`). A folder that [`Source::check`] calls stale is
    /// deleted first. Returns how many files it made.
    pub fn prepare(&self, mut progress: impl FnMut(usize, usize)) -> io::Result<usize> {
        match self.check() {
            Check::Stale(_) => self.wipe()?,
            // A file of the wrong size is made again.
            Check::Missing(needs) => {
                for need in needs {
                    let _ = std::fs::remove_file(self.path(need));
                }
            }
            Check::Ready | Check::NotPrepared => {}
        }
        let xnf = self.xnf()?;
        let all = Need::all(&xnf);
        let missing: Vec<Need> = all.iter().copied().filter(|&n| !self.has(n)).collect();
        let mut fingerprint = None;
        if let Some(&first) = missing.first() {
            let mut image = Image::open(self.image_for(first)?)?;
            for (done, &need) in missing.iter().enumerate() {
                progress(done, missing.len());
                self.store(need, &image.extract(need, &xnf)?)?;
            }
            progress(missing.len(), missing.len());
            fingerprint = Some(image.fingerprint()?);
        }
        let mut manifest = Manifest::default();
        if let Some(image) = &self.image {
            manifest.image_size = std::fs::metadata(image)?.len();
        }
        manifest.bigfile = fingerprint.unwrap_or_default();
        for &need in &all {
            let size = std::fs::metadata(self.path(need))?.len();
            manifest.files.push((need.key(), size));
        }
        write_atomically(&self.dir.join(MANIFEST), |out| {
            use std::io::Write;
            out.write_all(manifest.render().as_bytes())
        })?;
        Ok(missing.len())
    }
}

/// The manifest: what a folder was made from, and the size of each file.
#[derive(Debug, Default, PartialEq, Eq)]
struct Manifest {
    /// The size of the image, 0 if the files were not made from one here.
    image_size: u64,
    /// FNV-1a of the header of `BIGFILE.BIG` (0 if unknown): a record of which disc it was.
    bigfile: u64,
    files: Vec<(String, u64)>,
}

impl Manifest {
    fn render(&self) -> String {
        let mut text = format!(
            "ctr-rs prepared {LAYOUT}\nimage-size {}\nbigfile {:016x}\n",
            self.image_size, self.bigfile
        );
        for (key, size) in &self.files {
            text.push_str(&format!("{key} {size}\n"));
        }
        text
    }

    /// None if it is not one of this [`LAYOUT`].
    fn parse(text: &str) -> Option<Manifest> {
        let mut lines = text.lines();
        let layout = lines.next()?.strip_prefix("ctr-rs prepared ")?;
        if layout.parse::<u32>().ok()? != LAYOUT {
            return None;
        }
        let image_size = lines.next()?.strip_prefix("image-size ")?.parse().ok()?;
        let bigfile = u64::from_str_radix(lines.next()?.strip_prefix("bigfile ")?, 16).ok()?;
        let files = lines
            .map(|line| {
                let (key, size) = line.rsplit_once(' ')?;
                Some((key.to_string(), size.parse().ok()?))
            })
            .collect::<Option<_>>()?;
        Some(Manifest {
            image_size,
            bigfile,
            files,
        })
    }

    fn size_of(&self, key: &str) -> Option<u64> {
        self.files.iter().find(|f| f.0 == key).map(|f| f.1)
    }
}

/// The disc image, opened to take files from.
struct Image {
    big: BigFile,
    /// The image as 2048-byte sectors, at `BIGFILE.BIG` and the other files.
    disc: Disc,
    iso: IsoImage<RawSectorReader<FileDisc>>,
    /// The image as it is, in raw sectors, for the XA.
    raw: FileDisc,
}

impl Image {
    fn open(path: &Path) -> io::Result<Image> {
        let (disc, big) = open_bigfile(path)?;
        Ok(Image {
            big,
            disc,
            iso: mount(&|| open_file(path))?,
            raw: open_file(path)?,
        })
    }

    /// FNV-1a of the header of `BIGFILE.BIG`, which tells discs apart.
    fn fingerprint(&mut self) -> io::Result<u64> {
        let header = BigFile::read_header(&mut self.disc, self.big.start_sector)?;
        Ok(header.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, &byte| {
            (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3)
        }))
    }

    fn extract(&mut self, need: Need, xnf: &Xnf) -> io::Result<Vec<u8>> {
        let entry = |group: BigIndex, n: u32| group.nth(n).get();
        match need {
            Need::Racer(c) => self
                .big
                .read(&mut self.disc, entry(BigIndex::RACER_MODEL_HI, c)),
            Need::Pack(c) => self
                .big
                .read(&mut self.disc, entry(BigIndex::ARCADE_1P_PACK, c)),
            Need::SharedVram => self.big.read(&mut self.disc, BigIndex::SHARED_MPK_VRM.get()),
            Need::Howl => self.file("SOUNDS/KART.HWL"),
            Need::Xnf => self.file("XA/ENG.XNF"),
            Need::Xa(category, id) => {
                let track = xnf
                    .track(category, id)
                    .ok_or_else(|| invalid(format!("no XA track {id} in {category:?}")))?;
                let name = format!("{}/S{:02}.XA", category.dir(), track.file);
                let start = find(&self.iso, &name)
                    .ok_or_else(|| invalid(format!("{name} not on the disc")))?;
                read_track(&mut self.raw, start, track)
            }
        }
    }

    /// A file of the disc, whole.
    fn file(&mut self, name: &str) -> io::Result<Vec<u8>> {
        use std::io::Read;
        let (start, size) = find_sized(&self.iso, name)
            .ok_or_else(|| invalid(format!("{name} not on the disc")))?;
        self.disc
            .seek(SeekFrom::Start(start as u64 * USER_DATA_SIZE))?;
        let mut bytes = vec![0; size as usize];
        self.disc.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xa::Track;

    fn xnf() -> Xnf {
        let track = |file| Track {
            channel: 0,
            file,
            sectors: 10,
        };
        Xnf {
            files: [1, 1, 1],
            tracks: [
                vec![track(0), Track { sectors: 0, ..track(0) }],
                vec![track(0)],
                Vec::new(),
            ],
        }
    }

    #[test]
    fn every_need_has_its_own_key() {
        let all = Need::all(&xnf());
        // The track that spans no sector is not one.
        assert_eq!(all.len(), 3 + 2 * CHARACTERS as usize + 2);
        let mut keys: Vec<String> = all.iter().map(|n| n.key()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), all.len());
        assert!(all.contains(&Need::Xa(Category::Extra, 0)));
        assert!(!all.contains(&Need::Xa(Category::Music, 1)));
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ctr-source-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_manifest_comes_back_as_it_was_written() {
        let manifest = Manifest {
            image_size: 737_280_000,
            bigfile: 0xdead_beef,
            files: vec![("racer/00".into(), 12), ("xa/music/006".into(), 3)],
        };
        assert_eq!(Manifest::parse(&manifest.render()), Some(manifest));
        assert_eq!(Manifest::parse("ctr-rs prepared 99\n"), None);
        assert_eq!(Manifest::parse("not a manifest"), None);
    }

    #[test]
    fn check_tells_what_is_wrong_with_a_folder() {
        let dir = temp("check");
        let source = Source::new(&dir, None);
        assert_eq!(source.check(), Check::NotPrepared);

        std::fs::write(dir.join(MANIFEST), "ctr-rs prepared 0\n").unwrap();
        assert!(matches!(source.check(), Check::Stale(_)));

        // Made from an image of another size.
        let image = dir.join("image.bin");
        std::fs::write(&image, [0u8; 10]).unwrap();
        let manifest = Manifest {
            image_size: 11,
            ..Manifest::default()
        };
        std::fs::write(dir.join(MANIFEST), manifest.render()).unwrap();
        let source = Source::new(&dir, Some(image));
        assert!(matches!(source.check(), Check::Stale(why) if why.contains("11")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_an_image_a_missing_file_is_told() {
        let source = Source::new(std::env::temp_dir().join("ctr-source-empty"), None);
        let e = source.get(Need::SharedVram).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(e.to_string().contains("shared.vrm"), "{e}");
    }
}
