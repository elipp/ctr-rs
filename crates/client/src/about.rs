//! The licenses on the pause menu's "About": ctr-rs's, `THIRD_PARTY_NOTICES.md`'s, and
//! Inconsolata's. The notices' web page font is not in the game, so it is left out.

use crate::accordion::{Accordion, Entry};

const LICENSE: &str = include_str!("../../../LICENSE");
const NOTICES: &str = include_str!("../../../THIRD_PARTY_NOTICES.md");
const INCONSOLATA: &str = include_str!("../fonts/OFL.txt");

pub fn list(shown: usize) -> Accordion {
    Accordion::new(entries(), shown)
}

fn entries() -> Vec<Entry> {
    // (As the README says it.)
    let source = "ctr-rs is licensed under the GNU General Public License, version 3, which is the \
        license of ctr-native and CTR-ModSDK, the projects it is ported from.\n\n\
        Source: https://github.com/elipp/ctr-rs\n\n";
    let mut entries = vec![Entry::new(
        "ctr-rs",
        "GPL-3.0",
        format!("{source}{LICENSE}"),
    )];
    // Each notice is a `## ` heading, with its license on a `License: ` line.
    for section in NOTICES.split("\n## ").skip(1) {
        let (name, text) = section.split_once('\n').unwrap_or((section, ""));
        if name.contains("(web page") {
            continue;
        }
        let license = text
            .lines()
            .find_map(|line| line.strip_prefix("License: "))
            .and_then(|license| license.split(',').next())
            .unwrap_or("")
            .trim_end_matches('.');
        entries.push(Entry::new(name, license, text.trim()));
    }
    entries.push(Entry::new(
        "Inconsolata (this list's font)",
        "OFL-1.1",
        INCONSOLATA,
    ));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_every_part() {
        let rows: Vec<String> = list(10).rows().into_iter().map(|row| row.text).collect();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].starts_with("+ ctr-rs") && rows[0].ends_with("GPL-3.0"));
        assert!(rows[1].starts_with("+ PsyCross") && rows[1].ends_with("MIT"));
        assert!(rows[2].starts_with("+ Inconsolata") && rows[2].ends_with("OFL-1.1"));
        let ctr_rs = entries()[0].lines().to_vec();
        assert!(ctr_rs[0].starts_with("ctr-rs is licensed under the GNU"));
        assert!(
            ctr_rs
                .iter()
                .any(|line| line.contains("GNU GENERAL PUBLIC LICENSE"))
        );
    }
}
