//! The address that a lobby is joined with.

use std::fmt;
use std::str::FromStr;

use iroh::{EndpointAddr, EndpointId, RelayUrl};

/// What a client needs to find a lobby: `<endpoint id>`, then `@<relay url>` when it has one.
/// With the id alone, iroh looks the lobby's address up (the `N0` preset's DNS service); the
/// relay makes the first contact quicker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub id: EndpointId,
    pub relay: Option<RelayUrl>,
}

impl Ticket {
    pub(crate) fn of(addr: &EndpointAddr) -> Ticket {
        Ticket {
            id: addr.id,
            relay: addr.relay_urls().next().cloned(),
        }
    }

    pub(crate) fn addr(&self) -> EndpointAddr {
        let addr = EndpointAddr::new(self.id);
        match &self.relay {
            Some(relay) => addr.with_relay_url(relay.clone()),
            None => addr,
        }
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.id)?;
        if let Some(relay) = &self.relay {
            write!(f, "@{relay}")?;
        }
        Ok(())
    }
}

impl FromStr for Ticket {
    type Err = String;

    fn from_str(s: &str) -> Result<Ticket, String> {
        let (id, relay) = match s.split_once('@') {
            Some((id, relay)) => (id, Some(relay)),
            None => (s, None),
        };
        Ok(Ticket {
            id: id
                .parse()
                .map_err(|e| format!("bad endpoint id '{id}': {e}"))?,
            relay: relay
                .map(|r| r.parse().map_err(|e| format!("bad relay url '{r}': {e}")))
                .transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_round_trips() {
        let id = iroh::SecretKey::generate().public();
        for relay in [None, Some("https://relay.example.com./".parse().unwrap())] {
            let ticket = Ticket { id, relay };
            assert_eq!(ticket.to_string().parse(), Ok(ticket));
        }
        assert!("nonsense".parse::<Ticket>().is_err());
    }
}
