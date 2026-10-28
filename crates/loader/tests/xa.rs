mod common;

use loader::disc::open_xa;
use loader::xa::{Category, RATE_FULL, RATE_HALF, decode};

#[test]
fn the_xa_of_the_disc() {
    let path = common::disc_path();
    if !path.exists() {
        eprintln!(
            "skipping: {} not found (set CTR_BIN to override)",
            path.display()
        );
        return;
    }
    let (xnf, mut disc) = open_xa(&path).expect("read XA/ENG.XNF");
    assert_eq!(xnf.files, [2, 6, 21]);
    assert_eq!(xnf.tracks.each_ref().map(Vec::len), [13, 87, 314]);

    // "FINAL LAP!" (`Audio_SetState(AUDIO_FINAL_LAP)`): channel 6 of the first music file.
    let final_lap = xnf.track(Category::Music, 6).unwrap();
    assert_eq!(
        (final_lap.channel, final_lap.file, final_lap.sectors),
        (6, 0, 252)
    );
    let sectors = disc.read(Category::Music, final_lap).unwrap().unwrap();
    let pcm = decode(&sectors);
    let seconds = pcm.frames.len() as f32 / pcm.rate as f32;
    assert!((1.5..4.0).contains(&seconds), "{seconds} s at {}", pcm.rate);
    assert!(
        pcm.frames
            .iter()
            .flatten()
            .any(|s| s.unsigned_abs() > 8_000)
    );

    // Every track of every category is on the disc, and is heard.
    for category in Category::ALL {
        for id in [0, xnf.tracks[category as usize].len() as u16 - 1] {
            let track = xnf.track(category, id).unwrap();
            let sectors = disc.read(category, track).unwrap().expect("its file");
            let pcm = decode(&sectors);
            assert!([RATE_FULL, RATE_HALF].contains(&pcm.rate));
            assert!(
                pcm.frames.len() > pcm.rate as usize / 4,
                "{category:?} {id}"
            );
            assert!(
                pcm.frames
                    .iter()
                    .flatten()
                    .any(|s| s.unsigned_abs() > 2_000)
            );
        }
    }
}
