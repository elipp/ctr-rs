//! The objects a driver collects or breaks: the wumpa fruit and the crates. They do nothing to a
//! driver on their own (no [`crate::Contact`]): a touch of their hitbox does it
//! ([`crate::Instance::touch`]).

pub mod crates;
pub mod fruit;
