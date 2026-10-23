//! `META_PHYS` and `TERRAINS` against the initialisers in the reference source.

#[path = "../../coll/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;

use vehicle::stats::META_PHYS;
use vehicle::terrain::TERRAINS;

fn source() -> Option<String> {
    let path = support::reference_dir().join("game/zGlobal_DATA.c");
    let text = std::fs::read_to_string(&path).ok();
    if text.is_none() {
        eprintln!("skipping: {} not found", path.display());
    }
    text
}

fn number(s: &str) -> i64 {
    let s = s.trim();
    match s.strip_prefix("0x").or(s.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).unwrap(),
        None => s.parse().unwrap_or_else(|_| panic!("not a number: `{s}`")),
    }
}

/// Splits at the commas outside braces and parentheses.
fn split_top_level(s: &str) -> Vec<&str> {
    let (mut depth, mut start, mut out) = (0, 0, Vec::new());
    for (i, c) in s.char_indices() {
        match c {
            '{' | '(' => depth += 1,
            '}' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

#[test]
fn meta_phys_matches_reference_source() {
    let Some(text) = source() else { return };
    let table = text
        .split(".metaPhys =")
        .nth(1)
        .and_then(|t| t.split(".emSet_Exhaust_Water").next())
        .expect("metaPhys in zGlobal_DATA.c");
    let rows: Vec<Vec<i64>> = table
        .split("PHYSWRAP(")
        .skip(1)
        .map(|r| {
            split_top_level(r.split(')').next().unwrap())
                .iter()
                .skip(2)
                .map(|v| number(v))
                .collect()
        })
        .collect();
    assert_eq!(rows.len(), META_PHYS.len());
    for (i, (row, ours)) in rows.iter().zip(&META_PHYS).enumerate() {
        assert_eq!(row[..], ours.map(i64::from), "metaPhys row {i}");
    }
}

#[test]
fn terrain_table_matches_reference_source() {
    let Some(text) = source() else { return };
    let table = text
        .split(".MetaDataTerrain =")
        .nth(1)
        .and_then(|t| t.split("#define PHYSWRAP").next())
        .expect("MetaDataTerrain in zGlobal_DATA.c");
    let entries: Vec<Vec<&str>> = table
        .split("[TERRAIN_")
        .skip(1)
        .map(|e| {
            let body = e.split_once('{').unwrap().1;
            // The entry ends at the matching brace.
            let mut depth = 1;
            let end = body
                .char_indices()
                .find(|&(_, c)| {
                    depth += (c == '{') as i32 - (c == '}') as i32;
                    depth == 0
                })
                .unwrap()
                .0;
            split_top_level(&body[..end])
        })
        .collect();
    assert_eq!(entries.len(), TERRAINS.len());
    for (i, (e, t)) in entries.iter().zip(&TERRAINS).enumerate() {
        let ours = [
            t.flags as i64,
            t.speed_multiplier as i64,
            t.slow_until_speed as i64,
            t.counter_steer_ratio as i64,
            t.turn_lean_scale as i64,
            t.ground_friction_scale as i64,
            t.turn_angle_scale as i64,
            t.turn_response_scale as i64,
            t.sound as i64,
        ];
        let theirs = [
            number(e[1]),
            number(e[2]),
            number(e[3]),
            number(e[4]),
            number(e[5]),
            number(e[8]),
            number(e[9]),
            number(e[10]),
            number(e[13]),
        ];
        assert_eq!(theirs, ours, "terrain {i} ({})", t.name);
    }
}
