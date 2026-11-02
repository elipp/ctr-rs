//! Relay URLs without the trailing dot of iroh's public relay hostnames: Safari rejects it, as the
//! certificates have no dot.
//!
//! Realms and peers on older builds still publish dotted URLs, so lookups and tickets are cleaned
//! too. All clients do this because iroh takes the two spellings for two relays, and a relay
//! deactivates the older of two connections with the same key.

use iroh::address_lookup::{
    AddressLookup, AddressLookupBuilder, AddressLookupBuilderError, EndpointData, EndpointInfo,
    Error, Item,
};
use iroh::{Endpoint, EndpointId, RelayUrl, TransportAddr};
use n0_future::StreamExt;
use n0_future::boxed::BoxStream;

pub(crate) fn without_dot(url: &RelayUrl) -> RelayUrl {
    let Some(host) = url.host_str().filter(|host| host.ends_with('.')) else {
        return url.clone();
    };
    url.as_str()
        .replacen(host, host.trim_end_matches('.'), 1)
        .parse()
        .expect("a relay URL without the dot is still one")
}

fn item_without_dots(item: Item) -> Item {
    let info = item.endpoint_info();
    let addrs = info
        .addrs()
        .map(|addr| match addr {
            TransportAddr::Relay(url) => TransportAddr::Relay(without_dot(url)),
            other => other.clone(),
        })
        .collect();
    let mut data = EndpointData::new(addrs);
    data.set_user_data(info.user_data().cloned());
    Item::new(
        EndpointInfo::from_parts(info.endpoint_id, data),
        item.provenance(),
        item.last_updated(),
    )
}

#[derive(Debug)]
pub(crate) struct Dotless<B>(pub(crate) B);

impl<B: AddressLookupBuilder> AddressLookupBuilder for Dotless<B> {
    fn into_address_lookup(
        self,
        endpoint: &Endpoint,
    ) -> Result<impl AddressLookup, AddressLookupBuilderError> {
        Ok(DotlessLookup(self.0.into_address_lookup(endpoint)?))
    }
}

#[derive(Debug)]
struct DotlessLookup<T>(T);

impl<T: AddressLookup> AddressLookup for DotlessLookup<T> {
    fn resolve(&self, endpoint_id: EndpointId) -> Option<BoxStream<Result<Item, Error>>> {
        let items = self.0.resolve(endpoint_id)?;
        Some(Box::pin(items.map(|item| item.map(item_without_dots))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOTTED: &str = "https://euc1-1.relay.n0.iroh.link./";
    const DOTLESS: &str = "https://euc1-1.relay.n0.iroh.link/";

    #[test]
    fn only_a_trailing_dot_is_removed() {
        let url: RelayUrl = DOTTED.parse().unwrap();
        assert_eq!(without_dot(&url).as_str(), DOTLESS);
        let url: RelayUrl = "https://relay.example:8443/".parse().unwrap();
        assert_eq!(without_dot(&url), url);
    }

    #[test]
    fn looked_up_relays_lose_the_dot() {
        let id = iroh::SecretKey::from_bytes(&[7; 32]).public();
        let info = EndpointInfo::new(id).with_relay_url(DOTTED.parse().unwrap());
        let item = item_without_dots(Item::new(info, "test", Some(1)));
        let urls: Vec<_> = item.endpoint_info().relay_urls().map(|url| url.as_str()).collect();
        assert_eq!(urls, [DOTLESS]);
        assert_eq!(item.provenance(), "test");
        assert_eq!(item.last_updated(), Some(1));
    }
}
