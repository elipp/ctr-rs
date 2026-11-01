//! A scrolling list whose entries open to show their text, as on the pause menu's "About".

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::sync::OnceLock;

use bevy::ecs::component::Mutable;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::picking::events::{PointerClick, PointerOver};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;

use crate::sound::{MENU_TAKE_SFX, Sounds};

/// The most chars a line of text has.
pub const COLUMNS: usize = 80;
const TAG_SHOWN: usize = 30;
/// Lines a second, while Up or Down is held.
const MOVE_PER_SECOND: f32 = 30.0;
const WHEEL_LINES: f32 = 3.0;
const NAME: Color = Color::srgb(1.0, 0.72, 0.3);
pub const TEXT: Color = Color::srgb(0.8, 0.8, 0.8);
const PICKED: Color = Color::WHITE;
const BAR: Color = Color::srgba(0.5, 0.85, 0.85, 0.3);
/// Inconsolata's width and line height, in its height.
const WIDTH: f32 = 0.5;
const LINE: f32 = 1.2;

/// An entry: its name, the tag at its right, and the text it opens to.
pub struct Entry {
    name: String,
    tag: String,
    text: String,
    lines: OnceLock<Vec<String>>,
}

impl Entry {
    pub fn new(name: impl Into<String>, tag: impl Into<String>, text: impl Into<String>) -> Entry {
        Entry {
            name: name.into(),
            tag: tag.into(),
            text: text.into(),
            lines: OnceLock::new(),
        }
    }

    /// The text, wrapped at [`COLUMNS`] when first shown.
    pub fn lines(&self) -> &[String] {
        self.lines
            .get_or_init(|| self.text.lines().flat_map(wrap).collect())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Name(usize),
    Text(usize, usize),
}

impl Line {
    fn entry(self) -> usize {
        match self {
            Line::Name(entry) | Line::Text(entry, _) => entry,
        }
    }
}

/// A line as shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    /// An entry's line, not one of its text.
    pub name: bool,
    pub picked: bool,
}

/// A frame's input to a list.
#[derive(Clone, Copy, Debug, Default)]
pub struct Steer {
    /// Lines to move the pick.
    pub lines: i32,
    /// The way Up or Down is held: -1, 0 or 1.
    pub held: i32,
    /// Lines to scroll.
    pub wheel: f32,
    pub take: bool,
}

pub struct Accordion {
    entries: Vec<Entry>,
    /// The lines on screen at once.
    shown: usize,
    open: BTreeSet<usize>,
    cursor: usize,
    scroll: usize,
    // The fractions of a line moved and scrolled so far.
    moved: f32,
    scrolled: f32,
}

impl Accordion {
    pub fn new(entries: Vec<Entry>, shown: usize) -> Accordion {
        Accordion {
            entries,
            shown: shown.max(1),
            open: BTreeSet::new(),
            cursor: 0,
            scroll: 0,
            moved: 0.0,
            scrolled: 0.0,
        }
    }

    fn lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            lines.push(Line::Name(i));
            if self.open.contains(&i) {
                lines.extend((0..entry.lines().len()).map(|at| Line::Text(i, at)));
            }
        }
        lines
    }

    /// Whether the pick was taken.
    pub fn steer(&mut self, steer: Steer, seconds: f32) -> bool {
        let held = steer.held.signum() as f32 * MOVE_PER_SECOND * seconds;
        self.move_by(steer.lines as f32 + held);
        self.scroll_by(steer.wheel);
        if steer.take {
            self.toggle();
        }
        steer.take
    }

    pub fn move_by(&mut self, lines: f32) {
        let whole = take_whole(&mut self.moved, lines);
        let last = self.lines().len().saturating_sub(1);
        self.cursor = (self.cursor as f32 + whole).clamp(0.0, last as f32) as usize;
        self.keep_picked();
    }

    /// Scrolls, and keeps the pick on screen.
    pub fn scroll_by(&mut self, lines: f32) {
        let whole = take_whole(&mut self.scrolled, lines);
        let last = self.lines().len().saturating_sub(self.shown);
        self.scroll = (self.scroll as f32 + whole).clamp(0.0, last as f32) as usize;
        self.cursor = self.cursor.clamp(self.scroll, self.scroll + self.shown - 1);
    }

    /// Picks the `slot`th line on screen.
    pub fn point(&mut self, slot: usize) {
        if self.scroll + slot < self.lines().len() {
            self.cursor = self.scroll + slot;
        }
    }

    /// Opens or closes the picked entry. Closing picks the entry's line.
    pub fn toggle(&mut self) {
        let Some(entry) = self.lines().get(self.cursor).map(|line| line.entry()) else {
            return;
        };
        if !self.open.remove(&entry) {
            self.open.insert(entry);
        }
        let lines = self.lines();
        self.cursor = lines
            .iter()
            .position(|line| *line == Line::Name(entry))
            .unwrap_or(0);
        self.scroll = self.scroll.min(lines.len().saturating_sub(self.shown));
        self.keep_picked();
    }

    /// The lines on screen.
    pub fn rows(&self) -> Vec<Row> {
        let lines = self.lines();
        let end = (self.scroll + self.shown).min(lines.len());
        (self.scroll..end)
            .map(|at| {
                let text = match lines[at] {
                    Line::Name(i) => {
                        let mark = if self.open.contains(&i) { '-' } else { '+' };
                        let entry = &self.entries[i];
                        let tag: String = entry.tag.chars().take(TAG_SHOWN).collect();
                        let room = COLUMNS - 2 - tag.chars().count();
                        let name: String = entry.name.chars().take(room).collect();
                        format!("{mark} {name:<room$}  {tag}")
                    }
                    Line::Text(i, at) => format!("  {}", self.entries[i].lines()[at]),
                };
                Row {
                    text,
                    name: matches!(lines[at], Line::Name(_)),
                    picked: at == self.cursor,
                }
            })
            .collect()
    }

    fn keep_picked(&mut self) {
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + self.shown {
            self.scroll = self.cursor + 1 - self.shown;
        }
    }
}

/// A resource that keeps a list, for the pointer to reach it from the list's lines.
pub trait HasAccordion: Resource + Component<Mutability = Mutable> {
    fn accordion(&mut self) -> Option<&mut Accordion>;
}

/// A line of `R`'s list on screen.
#[derive(Component)]
pub struct AccordionLine<R> {
    slot: usize,
    owner: PhantomData<fn() -> R>,
}

pub type Lines<'w, 's, R> = Query<
    'w,
    's,
    (
        &'static AccordionLine<R>,
        &'static mut Text,
        &'static mut TextColor,
        &'static mut BackgroundColor,
    ),
>;

/// Draws `R`'s `list` in `parent`. Pointing at a line picks it, and a click takes it.
pub fn spawn<R: HasAccordion>(
    commands: &mut Commands,
    parent: Entity,
    list: &Accordion,
    font_size: f32,
) -> Entity {
    let font = TextFont {
        font_size: font_size.into(),
        ..default()
    };
    let line = font_size * LINE;
    let lines = commands
        .spawn((
            ChildOf(parent),
            Node {
                flex_direction: FlexDirection::Column,
                // The text's columns, its indent, and one for the rounding.
                width: Val::Px((COLUMNS + 3) as f32 * font_size * WIDTH),
                height: Val::Px(list.shown as f32 * line),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .id();
    let rows = list.rows();
    for slot in 0..list.shown {
        let row = rows.get(slot);
        let (ink, bar) = colors(row);
        commands
            .spawn((
                ChildOf(lines),
                AccordionLine::<R> {
                    slot,
                    owner: PhantomData,
                },
                Text::new(row.map_or("", |row| row.text.as_str())),
                TextLayout::linebreak(LineBreak::NoWrap),
                font.clone(),
                TextColor(ink),
                BackgroundColor(bar),
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(line),
                    flex_shrink: 0.0,
                    ..default()
                },
            ))
            .observe(move |_: On<PointerOver>, mut owner: ResMut<R>| {
                if let Some(list) = owner.accordion() {
                    list.point(slot);
                }
            })
            .observe(
                move |click: On<PointerClick>,
                      mut owner: ResMut<R>,
                      sounds: Option<Res<Sounds>>| {
                    if click.button != PointerButton::Primary {
                        return;
                    }
                    if let Some(list) = owner.accordion() {
                        list.point(slot);
                        list.toggle();
                        if let Some(sounds) = sounds {
                            sounds.menu(MENU_TAKE_SFX);
                        }
                    }
                },
            );
    }
    lines
}

pub fn update<R: HasAccordion>(list: &Accordion, lines: &mut Lines<R>) {
    if lines.is_empty() {
        return;
    }
    let rows = list.rows();
    for (line, mut text, mut color, mut bar) in lines {
        let row = rows.get(line.slot);
        let (ink, back) = colors(row);
        let shown = row.map_or("", |row| row.text.as_str());
        if text.0 != shown {
            text.0 = shown.to_string();
        }
        if color.0 != ink {
            color.0 = ink;
        }
        if bar.0 != back {
            bar.0 = back;
        }
    }
}

/// A line's colour, and its bar's.
fn colors(row: Option<&Row>) -> (Color, Color) {
    match row {
        Some(row) if row.picked => (PICKED, BAR),
        Some(row) if row.name => (NAME, Color::NONE),
        _ => (TEXT, Color::NONE),
    }
}

/// The lines `key` moves the pick: Up and Down, W and S, Page Up and Down, Home and End.
pub fn key_lines(key: KeyCode, page: usize) -> i32 {
    let page = page as i32;
    match key {
        KeyCode::ArrowUp | KeyCode::KeyW => -1,
        KeyCode::ArrowDown | KeyCode::KeyS => 1,
        KeyCode::PageUp => -page,
        KeyCode::PageDown => page,
        KeyCode::Home => i32::MIN,
        KeyCode::End => i32::MAX,
        _ => 0,
    }
}

/// The lines the wheel scrolls; up is back.
pub fn wheel_lines(event: &MouseWheel, font_size: f32) -> f32 {
    match event.unit {
        MouseScrollUnit::Line => -event.y * WHEEL_LINES,
        MouseScrollUnit::Pixel => -event.y / (font_size * LINE),
    }
}

fn take_whole(carry: &mut f32, lines: f32) -> f32 {
    *carry += lines;
    let whole = carry.trunc();
    *carry -= whole;
    whole
}

/// `line` broken at spaces into lines of at most [`COLUMNS`] chars.
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

    /// A short entry, one of three lines, and one with a name and a tag too long.
    fn list(shown: usize) -> Accordion {
        let entries = vec![
            Entry::new("one", "A", "a line"),
            Entry::new("two", "B", "first\nsecond\nthird"),
            Entry::new(
                "three ".repeat(20),
                "C".repeat(TAG_SHOWN * 2),
                "x".repeat(COLUMNS * 2),
            ),
        ];
        Accordion::new(entries, shown)
    }

    #[test]
    fn wraps_at_spaces() {
        let long = format!("{} {}", "a".repeat(COLUMNS - 2), "b".repeat(5));
        assert_eq!(wrap(&long), ["a".repeat(COLUMNS - 2), "b".repeat(5)]);
        let word = "c".repeat(COLUMNS + 3);
        assert_eq!(wrap(&word), ["c".repeat(COLUMNS), "c".repeat(3)]);
        assert_eq!(wrap("short\t"), ["short"]);
    }

    #[test]
    fn every_line_fits() {
        let mut list = list(100);
        list.open = (0..list.entries.len()).collect();
        let rows = list.rows();
        assert_eq!(rows.len(), 3 + 1 + 3 + 2);
        assert!(
            rows.iter()
                .all(|row| row.text.chars().count() <= COLUMNS + 2)
        );
    }

    #[test]
    fn opens_and_closes() {
        let mut list = list(10);
        list.move_by(1.0);
        list.toggle();
        let rows = list.rows();
        assert_eq!(rows.len(), 3 + 3);
        assert!(rows[1].name && rows[1].picked && rows[1].text.starts_with("- two"));
        assert_eq!(rows[2].text, "  first");
        list.move_by(2.0);
        list.toggle();
        assert_eq!(list.rows().len(), 3);
        assert!(list.rows()[1].picked);
    }

    #[test]
    fn keeps_the_pick_shown() {
        let mut list = list(2);
        list.open = (0..list.entries.len()).collect();
        list.move_by(4.0);
        let rows = list.rows();
        assert_eq!(rows.len(), 2);
        assert!(rows[1].picked && rows[1].text == "  second");
        list.scroll_by(10.0);
        assert!(list.rows().iter().any(|row| row.picked));
        list.move_by(i32::MIN as f32);
        assert!(list.rows()[0].picked && list.rows()[0].text.starts_with("- one"));
    }

    #[test]
    fn steers() {
        let mut list = list(10);
        let take = Steer {
            lines: 1,
            take: true,
            ..default()
        };
        assert!(list.steer(take, 0.0));
        assert!(list.rows()[1].text.starts_with("- two"));
        let held = Steer {
            held: 1,
            ..default()
        };
        list.steer(held, 1.0);
        assert!(list.rows().last().unwrap().picked);
    }
}
