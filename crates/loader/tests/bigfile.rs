mod common;

use loader::bigindex::BigIndex;
use loader::disc::USER_DATA_SIZE;

#[test]
fn bigfile_index() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let big = &b.big;
    println!(
        "BIGFILE.BIG @ sector {}, {} bytes, {} entries",
        big.start_sector,
        b.file_size,
        big.entries.len()
    );
    for (i, e) in big.entries.iter().take(10).enumerate() {
        println!("{i:>5}  +{:<8} {:>9}", e.sector_offset, e.size);
    }

    assert!(!big.entries.is_empty());
    // The scrapbook is the last group, so the index table must cover it.
    println!("SCRAPBOOK index = {}", BigIndex::SCRAPBOOK.0);
    assert!(BigIndex::SCRAPBOOK.get() < big.entries.len());
    for (i, e) in big.entries.iter().enumerate() {
        let end =
            (e.sector_offset as u64 * USER_DATA_SIZE + e.size as u64).div_ceil(USER_DATA_SIZE);
        assert!(
            end * USER_DATA_SIZE <= b.file_size,
            "entry {i} runs past end of BIGFILE.BIG"
        );
    }
    // Reading the last entry exercises seek + read at the far end of the file.
    let last = big.entries.len() - 1;
    assert_eq!(
        big.read(&mut b.disc, last).unwrap().len(),
        big.entries[last].size as usize
    );
}
