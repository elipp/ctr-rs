//! The pause menu's "About" (`pause.rs`): what the game is made of, a line each with its license,
//! which taking shows under it, as Chrome's `chrome://credits` does.
//!
//! First ctr-rs, under the GPL; then the code and fonts it has from others, a part of
//! `THIRD_PARTY_NOTICES.md` each; and Inconsolata, the font of the list.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::{LazyLock, OnceLock};

/// The most chars a line has: a longer one is broken at its last space before.
pub const COLUMNS: usize = 80;
/// The most chars of a part's license its line shows.
const LICENSE_SHOWN: usize = 30;

const LICENSE: &str = include_str!("../../../LICENSE");
const NOTICES: &str = include_str!("../../../THIRD_PARTY_NOTICES.md");
const INCONSOLATA: &str = include_str!("../fonts/OFL.txt");

/// Something the game is made of, and its license.
pub struct Part {
    name: String,
    license: String,
    text: Cow<'static, str>,
    /// The text in lines of at most [`COLUMNS`] chars, once it is first shown.
    lines: OnceLock<Vec<String>>,
}

impl Part {
    fn new(
        name: impl Into<String>,
        license: impl Into<String>,
        text: impl Into<Cow<'static, str>>,
    ) -> Part {
        Part {
            name: name.into(),
            license: license.into(),
            text: text.into(),
            lines: OnceLock::new(),
        }
    }

    fn lines(&self) -> &[String] {
        self.lines
            .get_or_init(|| self.text.lines().flat_map(wrap).collect())
    }
}

/// The parts, in the order the list has them.
pub static PARTS: LazyLock<Vec<Part>> = LazyLock::new(parts);

fn parts() -> Vec<Part> {
    let source = "Source: https://github.com/elipp/ctr-rs\n\n";
    let mut parts = vec![Part::new("ctr-rs", "GPL-3.0", format!("{source}{LICENSE}"))];
    // A part of the notices is a `## ` heading, and its license is on the line `License: `.
    for section in NOTICES.split("\n## ").skip(1) {
        let (name, text) = section.split_once('\n').unwrap_or((section, ""));
        let license = text
            .lines()
            .find_map(|line| line.strip_prefix("License: "))
            .and_then(|license| license.split(',').next())
            .unwrap_or("")
            .trim_end_matches('.');
        parts.push(Part::new(name, license, text.trim()));
    }
    parts.push(Part::new(
        "Inconsolata (this list's font)",
        "OFL-1.1",
        INCONSOLATA,
    ));
    parts
}

/// A line of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    /// A part's line.
    Name(usize),
    /// A line of an open part's text.
    Text(usize, usize),
}

impl Line {
    fn part(self) -> usize {
        match self {
            Line::Name(part) | Line::Text(part, _) => part,
        }
    }
}

/// A line of the list as shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    /// A part's line, not one of its text.
    pub name: bool,
    pub picked: bool,
}

/// The list as the menu shows it: the parts open, the line picked, and the first line shown.
#[derive(Default)]
pub struct List {
    open: BTreeSet<usize>,
    cursor: usize,
    scroll: usize,
    /// The part of a line the pick and the list have moved, which move by whole ones.
    moved: f32,
    scrolled: f32,
}

impl List {
    /// The lines of the parts, and the text of the open ones.
    fn lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();
        for (part, shown) in PARTS.iter().enumerate() {
            lines.push(Line::Name(part));
            if self.open.contains(&part) {
                lines.extend((0..shown.lines().len()).map(|i| Line::Text(part, i)));
            }
        }
        lines
    }

    /// Moves the pick `lines` on (back if less than 0), and the list with it, `shown` lines of it
    /// being shown.
    pub fn move_by(&mut self, lines: f32, shown: usize) {
        let whole = take_whole(&mut self.moved, lines);
        let last = self.lines().len().saturating_sub(1);
        self.cursor = (self.cursor as f32 + whole).clamp(0.0, last as f32) as usize;
        self.keep_picked(shown);
    }

    /// Scrolls the list `lines` on, and the pick with it as far as it has to go.
    pub fn scroll_by(&mut self, lines: f32, shown: usize) {
        let whole = take_whole(&mut self.scrolled, lines);
        let last = self.lines().len().saturating_sub(shown);
        self.scroll = (self.scroll as f32 + whole).clamp(0.0, last as f32) as usize;
        self.cursor = self.cursor.clamp(self.scroll, self.scroll + shown - 1);
    }

    /// Picks the line in the `slot`th place shown.
    pub fn point(&mut self, slot: usize) {
        if self.scroll + slot < self.lines().len() {
            self.cursor = self.scroll + slot;
        }
    }

    /// Opens the part picked, or closes it and picks its line.
    pub fn toggle(&mut self, shown: usize) {
        let Some(part) = self.lines().get(self.cursor).map(|line| line.part()) else {
            return;
        };
        if !self.open.remove(&part) {
            self.open.insert(part);
        }
        let lines = self.lines();
        self.cursor = lines
            .iter()
            .position(|line| *line == Line::Name(part))
            .unwrap_or(0);
        self.scroll = self.scroll.min(lines.len().saturating_sub(shown));
        self.keep_picked(shown);
    }

    /// The `shown` lines from the first shown on.
    pub fn rows(&self, shown: usize) -> Vec<Row> {
        let lines = self.lines();
        let end = (self.scroll + shown).min(lines.len());
        (self.scroll..end)
            .map(|at| {
                let text = match lines[at] {
                    Line::Name(part) => {
                        let mark = if self.open.contains(&part) { '-' } else { '+' };
                        let part = &PARTS[part];
                        let license: String = part.license.chars().take(LICENSE_SHOWN).collect();
                        let room = COLUMNS - 2 - license.chars().count();
                        let name: String = part.name.chars().take(room).collect();
                        format!("{mark} {name:<room$}  {license}")
                    }
                    Line::Text(part, i) => format!("  {}", PARTS[part].lines()[i]),
                };
                Row {
                    text,
                    name: matches!(lines[at], Line::Name(_)),
                    picked: at == self.cursor,
                }
            })
            .collect()
    }

    /// Scrolls as far as the pick is shown.
    fn keep_picked(&mut self, shown: usize) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + shown {
            self.scroll = self.cursor + 1 - shown;
        }
    }
}

/// Adds `lines` to the part of a line `carry` has, and takes the whole lines out of it.
fn take_whole(carry: &mut f32, lines: f32) -> f32 {
    *carry += lines;
    let whole = carry.trunc();
    *carry -= whole;
    whole
}

/// `line` broken into lines of at most [`COLUMNS`] chars, at spaces where it has them.
fn wrap(line: &str) -> Vec<String> {
    let line = line.replace('\t', "    ");
    let mut rest = line.trim_end();
    let mut lines = Vec::new();
    while let Some((end, _)) = rest.char_indices().nth(COLUMNS) {
        let cut = if rest[end..].starts_with(' ') {
            end
        } else {
            rest[..end].rfind(' ').filter(|&at| at > 0).unwrap_or(end)
        };
        lines.push(rest[..cut].to_string());
        rest = rest[cut..].trim_start();
    }
    lines.push(rest.to_string());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_spaces() {
        let long = format!("{} {}", "a".repeat(COLUMNS - 2), "b".repeat(5));
        assert_eq!(wrap(&long), ["a".repeat(COLUMNS - 2), "b".repeat(5)]);
        let word = "c".repeat(COLUMNS + 3);
        assert_eq!(wrap(&word), ["c".repeat(COLUMNS), "c".repeat(3)]);
        assert_eq!(wrap("short\t"), ["short"]);
    }

    #[test]
    fn has_every_part() {
        let names: Vec<&str> = PARTS.iter().map(|part| part.name.as_str()).collect();
        assert_eq!(names[0], "ctr-rs");
        assert!(names.iter().any(|name| name.starts_with("PsyCross")));
        assert!(names.iter().any(|name| name.starts_with("Inconsolata")));
        let psycross = PARTS
            .iter()
            .find(|part| part.name.starts_with("PsyCross"))
            .unwrap();
        assert_eq!(psycross.license, "MIT");
        assert!(
            PARTS[0]
                .lines()
                .iter()
                .any(|line| line.contains("GNU GENERAL PUBLIC LICENSE"))
        );
    }

    #[test]
    fn every_line_fits() {
        let mut list = List::default();
        list.open = (0..PARTS.len()).collect();
        let rows = list.rows(list.lines().len());
        assert!(
            rows.iter()
                .all(|row| row.text.chars().count() <= COLUMNS + 2)
        );
    }

    #[test]
    fn opens_and_closes() {
        let mut list = List::default();
        let closed = list.lines().len();
        assert_eq!(closed, PARTS.len());
        list.move_by(1.0, 10);
        list.toggle(10);
        assert_eq!(list.lines().len(), closed + PARTS[1].lines().len());
        let rows = list.rows(10);
        assert!(rows[1].name && rows[1].picked && rows[1].text.starts_with("- "));
        assert!(!rows[2].name);
        // Closing from a line of the text picks the part's line again.
        list.move_by(3.0, 10);
        list.toggle(10);
        assert_eq!(list.lines().len(), closed);
        assert!(list.rows(10)[1].picked);
    }
}
