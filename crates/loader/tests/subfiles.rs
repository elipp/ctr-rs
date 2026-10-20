mod common;

use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::level::LevelId;
use loader::vram::{Vram, parse_vram_file};

#[test]
fn track_vram_files_upload() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let mut vram = Vram::default();
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        for lod in [Lod::OnePlayer, Lod::TwoPlayer, Lod::FourPlayer] {
            let idx = BigIndex::level(LevelId(id), lod, LevFile::Vram);
            let file = b.big.read(&mut b.disc, idx.get()).unwrap();
            let images = parse_vram_file(&file)
                .unwrap_or_else(|e| panic!("level {id} {lod:?} (entry {}): {e}", idx.0));
            assert!(!images.is_empty(), "level {id} {lod:?} has no images");
            for img in &images {
                vram.upload(img)
                    .unwrap_or_else(|e| panic!("level {id} {lod:?}: {e}"));
            }
        }
    }
    assert!(vram.pixels.iter().any(|&p| p != 0));
}

/// Finds which subfiles are laid out as DRAM files (pointer map at the end)
/// and checks that every one of them is internally consistent.
#[test]
fn dram_files_are_consistent() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let mut with_map = Vec::new();
    for i in 0..b.big.entries.len() {
        let file = b.big.read(&mut b.disc, i).unwrap();
        if file.len() < 8 {
            continue;
        }
        let off = i32::from_le_bytes(file[..4].try_into().unwrap());
        // A real pointer map ends exactly at the end of the file.
        let ends_at_eof = off >= 0
            && file
                .get(4 + off as usize..4 + off as usize + 4)
                .map(|n| 4 + off as usize + 4 + u32::from_le_bytes(n.try_into().unwrap()) as usize)
                == Some(file.len());
        if ends_at_eof {
            let dram = DramFile::parse(&file).unwrap_or_else(|e| panic!("entry {i}: {e}"));
            // Every pointer must land inside the payload.
            for &slot in &dram.pointer_slots {
                let target = dram.read_u32(slot) as usize;
                assert!(
                    target <= dram.data.len(),
                    "entry {i}: slot {slot:#x} -> {target:#x}"
                );
            }
            with_map.push((i, dram.pointer_slots.len()));
        }
    }
    println!("{} entries have a pointer map at EOF", with_map.len());
    for (i, n) in with_map.iter().take(20) {
        println!("  entry {i}: {n} pointers");
    }
}

#[test]
fn track_lev_files_parse() {
    use loader::lev::{BspKind, Level};

    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
        let file = b.big.read(&mut b.disc, idx.get()).unwrap();
        let dram = DramFile::parse(&file).unwrap();
        let lev = Level::parse(&dram).unwrap_or_else(|e| panic!("level {id}: {e}"));

        let nv = lev.vertices.len();
        for (i, q) in lev.quad_blocks.iter().enumerate() {
            for &v in &q.vertex_index {
                assert!(
                    (v as usize) < nv,
                    "level {id}: quad {i} uses vertex {v} of {nv}"
                );
            }
            let (min, max) = (q.bbox.min, q.bbox.max);
            assert!(
                min.x <= max.x && min.y <= max.y && min.z <= max.z,
                "level {id}: quad {i} bbox"
            );
        }
        if let Some(sky) = &lev.skybox {
            let faces: usize = sky.segments.iter().map(Vec::len).sum();
            println!("level {id:>2}: skybox {} verts {faces} faces", sky.vertices.len());
            assert!(faces > 0, "level {id}: empty skybox");
        }
        let mut leaves = 0;
        for n in &lev.bsp {
            if let BspKind::Leaf { quad_blocks, .. } = &n.kind {
                assert!(
                    quad_blocks.end <= lev.quad_blocks.len(),
                    "level {id}: leaf range"
                );
                leaves += 1;
            }
        }
        println!(
            "level {id:>2}: {:>5} quads {:>6} verts {:>5} bsp ({leaves} leaves) {:>3} restart pts, spawn0 {:?}",
            lev.quad_blocks.len(),
            nv,
            lev.bsp.len(),
            lev.restart_points.len(),
            lev.driver_spawns[0].pos,
        );
    }
}
