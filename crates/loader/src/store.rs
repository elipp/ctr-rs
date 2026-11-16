//! The files the game makes for itself and reads back: what `source` carves from the disc and the
//! maps of the game's tracks (`map::builtin`). Natively a folder; in a browser, which has no
//! files, an in-memory map that the page fills from its IndexedDB before the game starts
//! ([`preload`]) and that tells the page of every change ([`on_change`]). Everything above this
//! is the same code on both, `Source::check` and the map versions included, so a browser makes
//! again exactly what a native client would.
//!
//! Paths are the keys as they are: the page keeps whatever `PathBuf` the client made.

use std::io;
use std::path::{Path, PathBuf};

pub use imp::*;

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    pub fn read(path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    /// The first `len` bytes (fewer if the file is shorter), without reading the rest.
    pub fn read_prefix(path: &Path, len: usize) -> io::Result<Vec<u8>> {
        use std::io::Read;
        let mut head = Vec::with_capacity(len);
        std::fs::File::open(path)?.take(len as u64).read_to_end(&mut head)?;
        Ok(head)
    }

    pub fn len(path: &Path) -> io::Result<u64> {
        let meta = std::fs::metadata(path)?;
        if meta.is_file() {
            Ok(meta.len())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "not a file"))
        }
    }

    /// Writes `path` and the directories above it, under another name until it is whole: a stop
    /// leaves the old file or none, never half of one.
    pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut part = path.as_os_str().to_owned();
        part.push(".part");
        let part = PathBuf::from(part);
        match std::fs::write(&part, bytes) {
            Ok(()) => std::fs::rename(&part, path),
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                Err(e)
            }
        }
    }

    /// Removes the file; NotFound if there is none.
    pub fn remove_file(path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    /// Removes everything under `dir`; NotFound if there is no such folder.
    pub fn remove_dir_all(dir: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(dir)
    }

    /// The files directly in `dir`; NotFound if there is no such folder.
    pub fn list(dir: &Path) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                files.push(entry.path());
            }
        }
        Ok(files)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    static FILES: Mutex<BTreeMap<PathBuf, Vec<u8>>> = Mutex::new(BTreeMap::new());

    /// What the page is told of a change: the bytes written, or None for a removal.
    pub type Hook = fn(&Path, Option<&[u8]>);

    static HOOK: OnceLock<Hook> = OnceLock::new();

    /// Takes in what the page kept, before anything reads. Not told to the hook: it is the page's.
    pub fn preload(files: impl IntoIterator<Item = (PathBuf, Vec<u8>)>) {
        FILES.lock().unwrap().extend(files);
    }

    /// Who is told of every write and removal from now on, to keep them. Once.
    pub fn on_change(hook: Hook) {
        if HOOK.set(hook).is_err() {
            panic!("the store's hook is set once");
        }
    }

    fn tell(path: &Path, bytes: Option<&[u8]>) {
        if let Some(hook) = HOOK.get() {
            hook(path, bytes);
        }
    }

    fn not_found(path: &Path) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, format!("{} is not kept", path.display()))
    }

    pub fn read(path: &Path) -> io::Result<Vec<u8>> {
        FILES.lock().unwrap().get(path).cloned().ok_or_else(|| not_found(path))
    }

    pub fn read_prefix(path: &Path, len: usize) -> io::Result<Vec<u8>> {
        let files = FILES.lock().unwrap();
        let bytes = files.get(path).ok_or_else(|| not_found(path))?;
        Ok(bytes[..len.min(bytes.len())].to_vec())
    }

    pub fn len(path: &Path) -> io::Result<u64> {
        let files = FILES.lock().unwrap();
        files.get(path).map(|b| b.len() as u64).ok_or_else(|| not_found(path))
    }

    pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        FILES.lock().unwrap().insert(path.to_path_buf(), bytes.to_vec());
        tell(path, Some(bytes));
        Ok(())
    }

    pub fn remove_file(path: &Path) -> io::Result<()> {
        FILES.lock().unwrap().remove(path).ok_or_else(|| not_found(path))?;
        tell(path, None);
        Ok(())
    }

    pub fn remove_dir_all(dir: &Path) -> io::Result<()> {
        let gone: Vec<PathBuf> = {
            let mut files = FILES.lock().unwrap();
            let under: Vec<PathBuf> =
                files.keys().filter(|p| p.starts_with(dir)).cloned().collect();
            for path in &under {
                files.remove(path);
            }
            under
        };
        if gone.is_empty() {
            return Err(not_found(dir));
        }
        for path in &gone {
            tell(path, None);
        }
        Ok(())
    }

    pub fn list(dir: &Path) -> io::Result<Vec<PathBuf>> {
        let files = FILES.lock().unwrap();
        let inside: Vec<PathBuf> = (files.keys())
            .filter(|p| p.parent() == Some(dir))
            .cloned()
            .collect();
        if inside.is_empty() && !files.keys().any(|p| p.starts_with(dir)) {
            return Err(not_found(dir));
        }
        Ok(inside)
    }
}
