//! Nicknames: how they are cleaned, and what the players are called when two are alike.

use std::time::Duration;

use iroh::EndpointId;

/// The most characters of a nickname.
pub const NICK_MAX: usize = 12;

/// How long a player waits between changes of its nickname (the game's to keep: see
/// [`Session::set_nickname`]).
pub const NICK_COOLDOWN: Duration = Duration::from_secs(60);

/// A nickname as it is kept and sent: trimmed, without control characters, and at most
/// [`NICK_MAX`] characters. Empty means none.
pub fn clean_nick(nick: &str) -> String {
    nick.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(NICK_MAX)
        .collect()
}

/// What each player is called, given the id and the nickname of each (empty for none): the
/// nickname, or the short form of the key without one. Players with the same nickname (whatever
/// the case) all get the first 3 characters of their key added, so that they can be told apart.
pub fn display_names(players: &[(EndpointId, &str)]) -> Vec<String> {
    players
        .iter()
        .map(|&(id, nick)| {
            let short = id.fmt_short().to_string();
            if nick.is_empty() {
                return short;
            }
            let twins = players
                .iter()
                .filter(|(_, other)| other.to_lowercase() == nick.to_lowercase())
                .count();
            if twins > 1 {
                format!("{nick}#{}", &short[..3])
            } else {
                nick.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nicknames_are_trimmed_and_cut() {
        assert_eq!(clean_nick("  Penta \n"), "Penta");
        assert_eq!(clean_nick("Dingodile the Great"), "Dingodile th");
        assert_eq!(clean_nick("a\u{7}b"), "ab");
        assert_eq!(clean_nick("ääääääääääääää").chars().count(), NICK_MAX);
        assert_eq!(clean_nick("   "), "");
    }

    #[test]
    fn equal_nicknames_get_the_key_added() {
        let [a, b, c] = std::array::from_fn(|_| iroh::SecretKey::generate().public());
        let short = |id: EndpointId| id.fmt_short().to_string()[..3].to_string();
        let names = display_names(&[(a, "Penta"), (b, "penta"), (c, "")]);
        assert_eq!(names[0], format!("Penta#{}", short(a)));
        assert_eq!(names[1], format!("penta#{}", short(b)));
        assert_eq!(names[2], c.fmt_short().to_string());
        assert_eq!(display_names(&[(a, "Penta"), (b, "Tiny")]), ["Penta", "Tiny"]);
    }
}
