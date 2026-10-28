//! What the command line suggests while a command is typed: the commands when the line is a
//! lone `/`, then the subcommands and values of the one typed so far, those that start with the
//! word being typed. The tree here only lists the commands; [`Action::parse`] still reads them,
//! and a test keeps the two agreeing.
//!
//! [`Action::parse`]: crate::command::Action

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
            leaf("create", "Open a lobby for others to join"),
            takes("join", "Join a lobby", "<address>"),
            takes("spectate", "Watch the races of a lobby (no address: switch in this one)", "[address]"),
            leaf("close", "Leave the lobby, or close it if it is ours"),
        ],
    ),
    node(
        "race",
        "The lobby's race (the host only)",
        &[
            leaf("start", "Count down to a race on this track"),
            leaf("restart", "Count down to the race again"),
            leaf("stop", "End the race for everyone, with no winner"),
        ],
    ),
    node(
        "config",
        "Change a setting",
        &[node(
            "set",
            "Change a setting",
            &[takes(
                "nickname",
                "The name the others see, up to 12 characters",
                "<name>",
            )],
        )],
    ),
    node(
        "net",
        "The connection to the others",
        &[
            leaf("info", "How each peer is connected"),
            Spec {
                name: "smoothing",
                about: "How the others' karts ease after a rollback",
                options: &[
                    leaf("ping", "By the round trip to each player"),
                    leaf("replay", "By how far the rollbacks replay"),
                ],
                arg: Some("<duration, like 120ms>"),
            },
            node(
                "rollbacks",
                "A toast for each rollback",
                &[leaf("on", "Show them"), leaf("off", "Stop showing them")],
            ),
        ],
    ),
    node(
        "cache",
        "The maps other players sent",
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
                        "Move the crates the level puts in the ground",
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
                "Switch between the race and the fly-through viewer",
            ),
            node(
                "crate",
                "Crates",
                &[takes("bias", "Depth bias of the crates, live", "[number]")],
            ),
            takes("stall", "Freeze this client, online", "<duration, like 3s>"),
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

    #[test]
    fn a_slash_lists_the_commands() {
        assert_eq!(
            names("/"),
            ["lobby", "race", "config", "net", "cache", "debug"]
        );
        assert_eq!(names("/r"), ["race"]);
        assert_eq!(names("/race "), ["start", "restart", "stop"]);
        assert_eq!(names("/race st"), ["start", "stop"]);
        assert_eq!(names("/debug set weapon 1"), ["1", "10", "11"]);
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
