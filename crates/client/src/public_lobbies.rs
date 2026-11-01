//! The public lobbies of the realm, for the pause menu to list (`pause.rs`): fetched on a thread
//! of `net`'s when the menu asks, and polled here until they are in.

use bevy::prelude::*;

/// The most lobbies the menu shows. (A screen has room for a few; the realm may have more.)
pub const SHOWN: usize = 8;

/// Where the fetch stands.
#[derive(Default)]
pub enum Status {
    /// Nothing was asked for.
    #[default]
    Idle,
    Loading,
    /// The lobbies, the ones we can join first, then the fullest.
    Loaded(Vec<net::PublicLobby>),
    Failed,
}

#[derive(Resource, Default)]
pub struct PublicLobbies {
    fetch: Option<net::Browse>,
    pub status: Status,
}

impl PublicLobbies {
    /// Asks the realm for the list again. (One that is on its way is let go of.)
    pub fn refresh(&mut self) {
        self.fetch = Some(net::browse(net::default_realm(), race::SIM_VERSION));
        self.status = Status::Loading;
    }

    /// The lobbies the menu shows, and how many more there are.
    pub fn shown(&self) -> (&[net::PublicLobby], usize) {
        match &self.status {
            Status::Loaded(lobbies) => {
                let n = lobbies.len().min(SHOWN);
                (&lobbies[..n], lobbies.len() - n)
            }
            _ => (&[], 0),
        }
    }
}

/// The order of the list: lobbies we can join first, then the fullest, then by name.
fn sort(lobbies: &mut [net::PublicLobby]) {
    lobbies.sort_by(|a, b| {
        (!a.compatible, std::cmp::Reverse(a.players), &a.name).cmp(&(
            !b.compatible,
            std::cmp::Reverse(b.players),
            &b.name,
        ))
    });
}

/// Takes in the list when it has come.
pub fn poll(mut lobbies: ResMut<PublicLobbies>) {
    let Some(result) = lobbies.fetch.as_ref().and_then(net::Browse::poll) else {
        return;
    };
    lobbies.fetch = None;
    lobbies.status = match result {
        Ok(mut found) => {
            debug!("the realm lists {} lobbies", found.len());
            sort(&mut found);
            Status::Loaded(found)
        }
        Err(why) => {
            warn!("could not fetch the public lobbies: {why}");
            Status::Failed
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lobby(name: &str, players: u8, compatible: bool) -> net::PublicLobby {
        // (An id has to be a point of the curve: this is the encoding of its base point.)
        let mut key = [0x66; 32];
        key[0] = 0x58;
        net::PublicLobby {
            id: net::EndpointId::from_bytes(&key).unwrap(),
            name: name.into(),
            players,
            max_players: 8,
            racing: false,
            track: String::new(),
            compatible,
        }
    }

    /// What a lobby name may have is what the fonts can draw: `net` cannot see `render`, so this
    /// is where the two are held together.
    #[test]
    fn lobby_names_are_made_of_what_the_font_has() {
        for c in (0..=0x7f_u8).map(char::from).chain("äöå€".chars()) {
            let drawn = c == ' ' || render::font::glyph(c).is_some();
            assert_eq!(net::is_lobby_name_char(c), drawn, "{c:?}");
        }
    }

    #[test]
    fn the_joinable_and_the_fullest_come_first() {
        let mut lobbies = vec![
            lobby("old", 7, false),
            lobby("b", 2, true),
            lobby("a", 2, true),
            lobby("full", 5, true),
        ];
        sort(&mut lobbies);
        let names: Vec<_> = lobbies.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["full", "a", "b", "old"]);
    }
}
