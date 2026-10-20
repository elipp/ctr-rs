mod common;

use loader::bigindex::BigIndex;
use loader::dram::DramFile;
use loader::icons::parse_groups;

#[test]
fn packs_have_the_wheel_icons() {
    let Some(mut b) = common::open_bigfile() else { return };
    for character in 0..16 {
        let file = b.big.read(&mut b.disc, BigIndex::ARCADE_1P_PACK.nth(character).get()).unwrap();
        let groups = parse_groups(&DramFile::parse(&file).unwrap()).unwrap();
        let wheels = groups.iter().find(|g| g.group_id == 0).expect("group 0");
        if character == 0 {
            for g in &groups {
                eprintln!("{:?} id {} icons {}", g.name, g.group_id, g.icons.len());
            }
            for i in &wheels.icons {
                eprintln!("{:?} {:?}", i.name, i.layout);
            }
        }
        assert_eq!(wheels.icons.len(), 17, "character {character}");
    }
}
