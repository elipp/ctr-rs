use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::PathBuf;

use hadris_iso::read::{IsoDir, IsoImage};
use loader::disc::RawSectorReader;

/// `CTR_BIN` overrides the default of `<repo>/assets/ctr-u.bin`.
fn disc_path() -> PathBuf {
    std::env::var_os("CTR_BIN").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/ctr-u.bin")
    })
}

struct Found {
    path: String,
    size: u64,
    sector: usize,
}

fn walk<R: Read + Seek>(
    image: &IsoImage<R>,
    dir: &IsoDir<'_, R>,
    prefix: &str,
    out: &mut Vec<Found>,
) {
    for entry in dir.entries() {
        let entry = entry.expect("failed to read directory entry");
        if entry.is_special() {
            continue;
        }
        let name = entry.display_name();
        let name = name.split(';').next().unwrap();
        let path = format!("{prefix}/{name}");

        if entry.is_directory() {
            let dir_ref = entry.as_dir_ref(image).expect("bad directory reference");
            walk(image, &image.open_dir(dir_ref), &path, out);
        } else {
            out.push(Found {
                path,
                size: entry.total_size(),
                sector: entry.header().extent.read() as usize,
            });
        }
    }
}

#[test]
fn dump_all_files() {
    let path = disc_path();
    if !path.exists() {
        eprintln!("skipping: {} not found (set CTR_BIN to override)", path.display());
        return;
    }

    let file = File::open(&path).expect("open disc image");
    let raw = RawSectorReader::new(BufReader::new(file)).expect("wrap raw sectors");
    let image = IsoImage::open(raw).expect("parse ISO 9660 filesystem");

    let mut files = Vec::new();
    let root = image.root_dir();
    walk(&image, &root.iter(&image), "", &mut files);

    println!("{:>8}  {:>10}  path", "sector", "bytes");
    for f in &files {
        println!("{:>8}  {:>10}  {}", f.sector, f.size, f.path);
    }
    println!("{} files", files.len());

    assert!(!files.is_empty());
    assert!(
        files.iter().any(|f| f.path.eq_ignore_ascii_case("/SYSTEM.CNF")),
        "no SYSTEM.CNF in image"
    );
}
