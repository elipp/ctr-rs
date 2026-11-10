//! What the command line suggests while a command is typed: the commands when the line is a
//! lone `/`, then the subcommands and values of the one typed so far, those that start with the
//! word being typed. The tree here only lists the commands; [`Action::parse`] still reads them,
//! and a test keeps the two agreeing.
//!
//! [`Action::parse`]: crate::command::Action

use crate::CHARACTERS;

/// A public lobby's offered name is one of these and one of [`LOBBY_NOUNS`]. Neither list has a
/// place: a name like a track's would read as the track the lobby races on.
const LOBBY_ADJECTIVES: &[&str] = &[
    "Angry", "Blazing", "Cursed", "Crazy", "Drifting", "Evil", "Furious", "Golden", "Hyper",
    "Lucky", "Mad", "Mighty", "Reckless", "Sacred", "Sneaky", "Speedy", "Spinning", "Turbo",
    "Nitro", "Wacky", "Wild",
];

const LOBBY_NOUNS: &[&str] = &[
    "Wumpas",
    "Crates",
    "Bandicoots",
    "Beakers",
    "Bombs",
    "Missiles",
    "Masks",
    "Karts",
    "Crash",
    "Coco",
    "Cortex",
    "Aku Aku",
    "Uka Uka",
    "Papu",
    "Komodo Joe",
    "Tiny",
    "Dingodile",
    "N. Gin",
    "N. Tropy",
];

pub(crate) const LOBBY_NAME_COUNT: u64 = (LOBBY_ADJECTIVES.len() * LOBBY_NOUNS.len()) as u64;

/// Keeps the order of the names from reading like the lists'. It is prime so that it shares no
/// factor with their lengths, which would leave words out.
const LOBBY_NAME_STRIDE: usize = 101;

/// The name at `index`, below [`LOBBY_NAME_COUNT`]. Every pair comes up once only if the lengths
/// of the two lists share no factor either (a test checks).
pub(crate) fn lobby_name_at(index: u64) -> String {
    let position = index as usize * LOBBY_NAME_STRIDE;
    let adjective = LOBBY_ADJECTIVES[position % LOBBY_ADJECTIVES.len()];
    let noun = LOBBY_NOUNS[position % LOBBY_NOUNS.len()];
    format!("{adjective} {noun}")
}

/// What `line` takes to give a public lobby that name: none unless it asks for one and has no
/// name yet.
pub(crate) fn name_to_add(line: &str, name: &str) -> Option<String> {
    if !line.split_whitespace().eq(["/lobby", "create", "public"]) {
        return None;
    }
    Some(after_the_line(line, name))
}

/// `name` as what `line` takes to have it after its last word, with a space if it needs one.
fn after_the_line(line: &str, name: &str) -> String {
    if line.ends_with(|c: char| c.is_whitespace() || c == ',') {
        name.to_string()
    } else {
        format!(" {name}")
    }
}

/// What goes with a name the command line offers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OfferPicture {
    /// The portrait of a character (`CHARACTERS` order).
    Portrait(usize),
    /// The picture of a track, a `LevelId`.
    Track(u32),
    /// A cup whose first track is gone.
    Nothing,
}

/// A name the command line offers in grey.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Offer<'a> {
    pub name: &'a str,
    pub picture: OfferPicture,
}

/// The characters, as what `/character` offers.
pub(crate) fn characters() -> impl Iterator<Item = Offer<'static>> {
    CHARACTERS
        .iter()
        .enumerate()
        .map(|(character, name)| Offer {
            name,
            picture: OfferPicture::Portrait(character),
        })
}

/// What `line` has typed after `command`, if it is that command.
pub(crate) fn typed_after<'l>(line: &'l str, command: &str) -> Option<&'l str> {
    let rest = line.strip_prefix(command)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim_start())
}

/// What `line` has typed of the track it is on, if it is a `/cup create` past the cup's name.
pub(crate) fn cup_track_typed(line: &str) -> Option<&str> {
    let (_name, tracks) = typed_after(line, "/cup create")?.split_once(char::is_whitespace)?;
    tracks.rsplit(',').next().map(str::trim_start)
}

/// What `line` has typed of the name of a cup it starts or deletes.
pub(crate) fn cup_name_typed(line: &str) -> Option<&str> {
    typed_after(line, "/cup start").or_else(|| typed_after(line, "/cup delete"))
}

/// Whether `name` starts with `typed`, in any ASCII case (as `Lobby::player_named` reads names).
pub(crate) fn starts_with(name: &str, typed: &str) -> bool {
    name.as_bytes()
        .get(..typed.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(typed.as_bytes()))
}

/// The offers that what is typed can still come to, each name once.
pub(crate) fn matching<'a>(typed: &str, offers: impl Iterator<Item = Offer<'a>>) -> Vec<Offer<'a>> {
    each_name_once(offers.filter(|o| starts_with(o.name, typed)))
}

pub(crate) fn each_name_once<'a>(offers: impl Iterator<Item = Offer<'a>>) -> Vec<Offer<'a>> {
    let mut found: Vec<Offer> = Vec::new();
    for offer in offers {
        if found.iter().all(|o| o.name != offer.name) {
            found.push(offer);
        }
    }
    found
}

/// Of the offers [`matching`] what is typed: the one called `wanted`, or else the one it names
/// whole, so that a longer name does not take its place, or else the first.
pub(crate) fn pick<'a>(offers: &[Offer<'a>], wanted: &str, typed: &str) -> Option<Offer<'a>> {
    let find = |test: &dyn Fn(&Offer) -> bool| offers.iter().find(|o| test(o)).copied();
    find(&|o| o.name == wanted)
        .or_else(|| find(&|o| o.name.eq_ignore_ascii_case(typed)))
        .or_else(|| offers.first().copied())
}

/// What `line`, which has `typed` of it, takes to name `name`: none if it does.
pub(crate) fn rest_of_name(line: &str, typed: &str, name: &str) -> Option<String> {
    if typed.is_empty() {
        return Some(after_the_line(line, name));
    }
    name.get(typed.len()..)
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
}

/// A command, subcommand or value, and what can follow it.
pub(crate) struct Spec {
    pub name: &'static str,
    pub about: &'static str,
    /// The words that can come next.
    pub options: &'static [Spec],
    /// What can be typed next instead of an option, like `<address>`.
    pub arg: Option<&'static str>,
}

impl Spec {
    /// Nothing follows it: picking it sends the line.
    pub fn is_leaf(&self) -> bool {
        self.options.is_empty() && self.arg.is_none()
    }
}

const fn leaf(name: &'static str, about: &'static str) -> Spec {
    Spec {
        name,
        about,
        options: &[],
        arg: None,
    }
}

const fn node(name: &'static str, about: &'static str, options: &'static [Spec]) -> Spec {
    Spec {
        name,
        about,
        options,
        arg: None,
    }
}

const fn takes(name: &'static str, about: &'static str, arg: &'static str) -> Spec {
    Spec {
        name,
        about,
        options: &[],
        arg: Some(arg),
    }
}

/// Every command, as `Action::parse` reads them.
pub(crate) const COMMANDS: &[Spec] = &[
    node(
        "lobby",
        "Play online",
        &[
            node(
                "create",
                "Open a lobby for others to join",
                &[
                    leaf(
                        "private",
                        "Join by address only (the default)",
                    ),
                    takes(
                        "public",
                        "Listed for anyone to find",
                        "[name, Up/Down]",
                    ),
                ],
            ),
            takes("join", "Join a lobby", "<address>"),
            takes(
                "spectate",
                "Watch a lobby's races",
                "[address]",
            ),
            leaf("close", "Leave the lobby, or close it if ours"),
        ],
    ),
    node(
        "race",
        "The lobby's race (host only)",
        &[
            leaf("start", "Count down to a race here"),
            leaf("restart", "Count down to the race again"),
            leaf("stop", "End the race for everyone"),
        ],
    ),
    Spec {
        name: "bots",
        about: "Bots in the lobby's races (host only)",
        options: &[
            leaf("off", "No bots"),
            leaf("easy", "Easy bots"),
            leaf("medium", "Medium bots (the default)"),
            leaf("hard", "Hard bots"),
            leaf("brain", "Bots the learned policy drives (the default)"),
            leaf("retail", "Retail's bots, on the bot paths"),
        ],
        arg: Some("[0 to 7]"),
    },
    node(
        "cup",
        "A series of races with points",
        &[
            takes(
                "create",
                "Make and keep a cup",
                "<name> <tracks> <laps>",
            ),
            takes("start", "Count down to a cup's first race", "<name>"),
            takes("delete", "Forget a cup", "<name>"),
        ],
    ),
    takes(
        "promote",
        "Make a player the host (host only)",
        "<player>",
    ),
    Spec {
        name: "track",
        about: "Go to a track",
        options: &[leaf("list", "Print the tracks")],
        arg: Some("[name, Up/Down]"),
    },
    takes(
        "character",
        "Who to drive",
        "[name, Up/Down]",
    ),
    node(
        "config",
        "Change a setting",
        &[node(
            "set",
            "Change a setting",
            &[
                takes(
                    "nickname",
                    "The name others see (12 at most)",
                    "<name>",
                ),
                // (A browser has none of these three: see `config.rs`.)
                #[cfg(not(target_arch = "wasm32"))]
                node(
                    "vsync",
                    "Wait for the display before drawing",
                    &[
                        leaf("on", "Wait for the display"),
                        leaf("off", "Draw as fast as allowed"),
                    ],
                ),
                #[cfg(not(target_arch = "wasm32"))]
                takes(
                    "fps-max",
                    "Frame limit, 0 for none",
                    "<frames a second>",
                ),
                #[cfg(not(target_arch = "wasm32"))]
                takes(
                    "pixel-scale",
                    "Draw at 1/N of the window's resolution",
                    "<1 to 8>",
                ),
                #[cfg(target_arch = "wasm32")]
                takes(
                    "render-scale",
                    "How much of the resolution to draw",
                    "<0.25 to 1>",
                ),
                // (A browser has no msaa either, see `config.rs`.)
                #[cfg(not(target_arch = "wasm32"))]
                node(
                    "msaa",
                    "Anti-aliasing samples of the cameras",
                    &[
                        leaf("off", "None (fastest)"),
                        leaf("2", "2 samples"),
                        leaf("4", "4 samples"),
                    ],
                ),
                node(
                    "shader-effects",
                    "Heat shimmer and the clock's trail",
                    &[
                        leaf("on", "Draw them"),
                        leaf("off", "Skip them (faster)"),
                    ],
                ),
                node(
                    "global-voices",
                    "Others' voice lines heard from anywhere",
                    &[
                        leaf("on", "Full volume, wherever the kart is"),
                        leaf("off", "Heard from where the kart is"),
                    ],
                ),
                node(
                    "volume",
                    "Music, effects and voices",
                    &[
                        takes(
                            "music",
                            "Music volume, 100 is the game's",
                            "<0-100>",
                        ),
                        takes(
                            "sfx",
                            "Effects volume, 100 is the game's",
                            "<0-100>",
                        ),
                        takes(
                            "voice",
                            "Voice volume, 100 is the game's",
                            "<0-100>",
                        ),
                    ],
                ),
            ],
        )],
    ),
    leaf("version", "Show the version of this build"),
    leaf("clear", "Empty the console"),
    node(
        "net",
        "The connection to the others",
        &[
            leaf("info", "How each peer is connected"),
            Spec {
                name: "smoothing",
                about: "How karts ease after a rollback",
                options: &[
                    leaf("ping", "By the round trip to each player"),
                    leaf("replay", "By how far the rollbacks replay"),
                ],
                arg: Some("<like 120ms>"),
            },
            node(
                "rollbacks",
                "A toast for each rollback",
                &[leaf("on", "Show them"), leaf("off", "Stop showing them")],
            ),
            node(
                "relay",
                "Relays besides the public ones",
                &[
                    takes(
                        "add",
                        "Use this relay too, from the next lobby",
                        "<url>",
                    ),
                    leaf("clear", "Drop added relays, from the next lobby"),
                    #[cfg(not(target_arch = "wasm32"))]
                    node(
                        "only",
                        "Hide your address behind the relay",
                        &[
                            leaf("on", "Never connect straight (slower, private)"),
                            leaf("off", "Connect straight when possible"),
                        ],
                    ),
                ],
            ),
        ],
    ),
    node(
        "cache",
        "The custom tracks other players sent",
        &[leaf("purge", "Delete them")],
    ),
    node(
        "debug",
        "For testing",
        &[
            node(
                "set",
                "Set what our kart has",
                &[
                    takes("wumpa", "The wumpa count", "<0 to 10>"),
                    node(
                        "weapon",
                        "The weapon in hand",
                        &[
                            leaf("0", "Turbo"),
                            leaf("1", "Bomb"),
                            leaf("2", "Missile"),
                            leaf("3", "TNT"),
                            leaf("4", "Potion"),
                            leaf("6", "Shield"),
                            leaf("7", "Mask"),
                            leaf("8", "Clock"),
                            leaf("9", "Warpball"),
                            leaf("10", "Three bombs"),
                            leaf("11", "Three missiles"),
                        ],
                    ),
                ],
            ),
            node(
                "level",
                "How the level is built",
                &[
                    node(
                        "hidden",
                        "Untextured blocks and red-cornered faces",
                        &[leaf("on", "Show them"), leaf("off", "Hide them")],
                    ),
                    node(
                        "cull",
                        "Back faces",
                        &[
                            leaf("on", "Cull them, as the game does"),
                            leaf("off", "Draw them"),
                        ],
                    ),
                    node(
                        "crate-fixup",
                        "Move crates out of the ground",
                        &[
                            leaf("on", "Move them"),
                            leaf("off", "Draw them where the level has them"),
                        ],
                    ),
                    node(
                        "colors",
                        "Vertex colors",
                        &[leaf("hi", "color_hi"), leaf("lo", "color_lo")],
                    ),
                ],
            ),
            leaf(
                "viewer",
                "Race or fly-through viewer",
            ),
            node(
                "hud",
                "The race's dev text (alone: toggle)",
                &[leaf("on", "Show it"), leaf("off", "Hide it")],
            ),
            takes("toast", "Show a text as a toast", "<text>"),
            node(
                "crate",
                "Crates",
                &[takes("bias", "Depth bias of the crates, live", "[number]")],
            ),
            leaf("ranking", "Random ranking (host)"),
            leaf("navpaths", "Draw the bot paths in the race too (toggle)"),
            takes("stall", "Freeze this client, online", "<duration, like 3s>"),
            takes(
                "loading-screen",
                "Show the loading screen",
                "<duration, like 3s>",
            ),
            Spec {
                name: "lag",
                about: "Hold back the others' pads here",
                options: &[leaf("off", "No lag")],
                arg: Some("<delay> [jitter]"),
            },
            node(
                "fx",
                "Try a visual effect (alone: the panel)",
                &[
                    node(
                        "confetti",
                        "The confetti of a win",
                        &[
                            leaf("auto", "When we finish first"),
                            leaf("on", "Always"),
                            leaf("off", "Never"),
                        ],
                    ),
                    leaf("flash", "The white flash of a blast on our kart"),
                    leaf("finish", "Our kart finishes first now, offline"),
                    Spec {
                        name: "speed",
                        about: "Slow motion, offline",
                        options: &[
                            leaf("1", "The game's speed"),
                            leaf("0.5", "Half"),
                            leaf("0.25", "A quarter"),
                            leaf("0.1", "A tenth"),
                        ],
                        arg: Some("<factor, 0.05 to 4>"),
                    },
                    node(
                        "freeze",
                        "Stop the race, offline",
                        &[leaf("on", "Stop it"), leaf("off", "Let it go on")],
                    ),
                    leaf("step", "One tick of a frozen race"),
                    node(
                        "weather",
                        "The level's rain or snow",
                        &[leaf("on", "Draw it"), leaf("off", "Hide it")],
                    ),
                    node(
                        "rain",
                        "The red beaker's rain",
                        &[leaf("on", "Draw it"), leaf("off", "Hide it")],
                    ),
                ],
            ),
        ],
    ),
];

/// What to show under a line being typed.
pub(crate) struct Menu {
    /// Where the word being typed starts in the line: picking an option replaces the rest.
    pub start: usize,
    /// The options that start with that word.
    pub options: Vec<&'static Spec>,
    /// What else can be typed there, and what it is for.
    pub arg: Option<(&'static str, &'static str)>,
}

impl Menu {
    /// The menu for `line`, or none if it is not a command or nothing is known to follow.
    pub fn of(line: &str) -> Option<Menu> {
        let command = line.strip_prefix('/')?;
        // The words typed in full, and the one being typed (empty after a space).
        let start = command
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map_or(0, |(at, c)| at + c.len_utf8());
        let typing = &command[start..];
        let mut options = COMMANDS;
        let mut about = "";
        let mut arg = None;
        for word in command[..start].split_whitespace() {
            let spec = options.iter().find(|s| s.name == word)?;
            (options, arg, about) = (spec.options, spec.arg, spec.about);
        }
        let matching: Vec<_> = options
            .iter()
            .filter(|s| s.name.starts_with(typing))
            .collect();
        let arg = arg.map(|a| (a, about));
        (!matching.is_empty() || arg.is_some()).then_some(Menu {
            start: start + 1,
            options: matching,
            arg,
        })
    }

    /// `line` with its word being typed replaced by option `index`, a space after it if more
    /// can follow; and whether nothing can, so that the line can be sent.
    pub fn pick(&self, line: &str, index: usize) -> Option<(String, bool)> {
        let spec = self.options.get(index)?;
        let space = if spec.is_leaf() { "" } else { " " };
        Some((
            format!("{}{}{space}", &line[..self.start], spec.name),
            spec.is_leaf(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Action;

    fn names(line: &str) -> Vec<&'static str> {
        Menu::of(line).map_or(Vec::new(), |m| m.options.iter().map(|s| s.name).collect())
    }

    /// Every name comes up once, can be listed, and fits a row of the pause menu's list uncut.
    #[test]
    fn the_offered_lobby_names_are_valid() {
        let names: std::collections::HashSet<_> =
            (0..LOBBY_NAME_COUNT).map(lobby_name_at).collect();
        assert_eq!(names.len() as u64, LOBBY_NAME_COUNT);
        for name in names {
            assert_eq!(net::lobby_name(&name).as_deref(), Ok(name.as_str()));
            assert!(name.chars().count() <= crate::pause::NAME_SHOWN, "{name}");
        }
    }

    #[test]
    fn a_slash_lists_the_commands() {
        assert_eq!(
            names("/"),
            [
                "lobby",
                "race",
                "bots",
                "cup",
                "promote",
                "track",
                "character",
                "config",
                "version",
                "clear",
                "net",
                "cache",
                "debug"
            ]
        );
        assert_eq!(names("/r"), ["race"]);
        assert_eq!(names("/race "), ["start", "restart", "stop"]);
        assert_eq!(names("/race st"), ["start", "stop"]);
        assert_eq!(names("/debug set weapon 1"), ["1", "10", "11"]);
        assert_eq!(
            names("/debug fx "),
            [
                "confetti", "flash", "finish", "speed", "freeze", "step", "weather", "rain"
            ]
        );
        assert_eq!(names("/debug fx f"), ["flash", "finish", "freeze"]);
        // Not a command, an unknown one, or past what is known.
        assert!(Menu::of("hello").is_none());
        assert!(Menu::of("/teapot ").is_none());
        assert!(Menu::of("/race start ").is_none());
        assert!(Menu::of("/config set nickname Big Bob").is_none());
    }

    #[test]
    fn a_value_to_type_is_shown() {
        let menu = Menu::of("/lobby join ").unwrap();
        assert!(menu.options.is_empty());
        assert_eq!(menu.arg.unwrap().0, "<address>");
        // Both the options and what else can go there.
        let menu = Menu::of("/net smoothing ").unwrap();
        assert_eq!(menu.options.len(), 2);
        assert!(menu.arg.is_some());
        assert!(Menu::of("/net smoothing 1").unwrap().options.is_empty());
    }

    #[test]
    fn picking_completes_the_word() {
        let line = "/ra";
        let menu = Menu::of(line).unwrap();
        assert_eq!(menu.pick(line, 0), Some(("/race ".into(), false)));
        let line = "/race res";
        let menu = Menu::of(line).unwrap();
        assert_eq!(menu.pick(line, 0), Some(("/race restart".into(), true)));
        let line = "/lobby jo";
        let menu = Menu::of(line).unwrap();
        assert_eq!(menu.pick(line, 0), Some(("/lobby join ".into(), false)));
        assert_eq!(menu.pick(line, 1), None);
    }

    #[test]
    fn offers_narrow_down_to_what_is_typed() {
        let named = |typed| -> Vec<&str> {
            matching(typed, characters()).iter().map(|o| o.name).collect()
        };
        assert_eq!(named("").len(), CHARACTERS.len());
        assert_eq!(named("n. t"), ["N. Tropy"]);
        assert_eq!(named("PE"), ["Penta"]);
        assert_eq!(named("fake "), ["Fake Crash"]);
        assert!(named("nobody").is_empty());
        assert_eq!(typed_after("/character", "/character"), Some(""));
        assert_eq!(typed_after("/character  Pe", "/character"), Some("Pe"));
        assert_eq!(typed_after("/characters", "/character"), None);

        assert_eq!(rest_of_name("/character", "", "Penta").unwrap(), " Penta");
        assert_eq!(rest_of_name("/character ", "", "Penta").unwrap(), "Penta");
        assert_eq!(rest_of_name("/character pe", "pe", "Penta").unwrap(), "nta");
        assert_eq!(rest_of_name("/character penta", "penta", "Penta"), None);
        assert_eq!(rest_of_name("/cup create a b,", "", "Penta").unwrap(), "Penta");

        assert_eq!(cup_track_typed("/cup create"), None);
        assert_eq!(cup_track_typed("/cup create fast"), None);
        assert_eq!(cup_track_typed("/cup create fast "), Some(""));
        assert_eq!(cup_track_typed("/cup create fast Crash Cove"), Some("Crash Cove"));
        assert_eq!(cup_track_typed("/cup create fast Crash Cove,"), Some(""));
        assert_eq!(cup_track_typed("/cup create fast Crash Cove, Pa"), Some("Pa"));
        assert_eq!(cup_track_typed("/cup start fast "), None);
    }

    #[test]
    fn a_players_name_is_any_text() {
        let offer = |name| Offer {
            name,
            picture: OfferPicture::Portrait(0),
        };
        let players = [offer("Äijä"), offer("Bobby"), offer("Bob"), offer("Bob")];
        let found = |typed| matching(typed, players.iter().copied());
        // (Part of a character of more than a byte is no prefix.)
        assert_eq!(found("Äi"), [players[0]]);
        assert_eq!(rest_of_name("/promote Äi", "Äi", "Äijä").unwrap(), "jä");
        assert!(found("ä").is_empty());
        // Each name once, and a name typed whole goes before a longer one.
        assert_eq!(found("bob").len(), 2);
        assert_eq!(pick(&found("bob"), "", "bob").unwrap().name, "Bob");
        assert_eq!(pick(&found("bob"), "Bobby", "bob").unwrap().name, "Bobby");
        assert_eq!(pick(&found("b"), "", "b").unwrap().name, "Bobby");
    }

    /// How many lines `text` takes in a column of `width` characters.
    fn lines(text: &str, width: usize) -> usize {
        let (mut lines, mut used) = (1, 0);
        for word in text.split(' ') {
            let length = word.chars().count();
            if used > 0 && used + 1 + length > width {
                lines += 1;
                used = 0;
            }
            if used > 0 {
                used += 1;
            }
            used += length;
            lines += used.saturating_sub(1) / width.max(1);
            used = (used.saturating_sub(1) % width.max(1)) + 1;
        }
        lines
    }

    /// What a command does fits two lines of the suggestions, in the narrowest console.
    #[test]
    fn the_descriptions_are_short() {
        /// `console::spawn`'s least width, less the padding, in the line's characters of 12 pixels;
        /// and the gap between the columns.
        const COLUMNS: usize = 37;
        const GAP: usize = 2;
        fn walk(about: &str, specs: &[Spec], arg: Option<&str>) {
            let names = specs.iter().map(|s| s.name).chain(arg);
            let name = names.map(|n| n.chars().count()).max().unwrap_or(0);
            let width = COLUMNS - name - GAP;
            let rows = specs.iter().map(|s| s.about).chain(arg.map(|_| about));
            for row in rows {
                assert!(lines(row, width) <= 2, "{row} in {width} columns");
            }
            for spec in specs {
                walk(spec.about, spec.options, spec.arg);
            }
        }
        assert_eq!(lines("one two", 7), 1);
        assert_eq!(lines("one two", 6), 2);
        walk("", COMMANDS, None);
    }

    /// Every word of the tree is a command `Action::parse` knows: a leaf parses, and one that
    /// takes more says how it is used rather than that it is unknown.
    #[test]
    fn the_tree_is_what_the_parser_reads() {
        fn walk(prefix: &str, specs: &[Spec]) {
            for spec in specs {
                let line = format!("{prefix}{}", spec.name);
                let parsed = Action::parse(&line);
                if spec.is_leaf() {
                    assert!(parsed.is_ok(), "/{line}: {:?}", parsed.err());
                } else if let Err(why) = parsed {
                    assert!(!why.starts_with("Unknown"), "/{line}: {why}");
                }
                walk(&format!("{line} "), spec.options);
            }
        }
        walk("", COMMANDS);
    }
}
