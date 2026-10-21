//! The objects that hurt (and one that throws): the armadillos of Dingo Canyon, the fireballs of
//! Mystery Caves, the minecarts of Dragon Mines, and the snowball and barrels that roll along
//! recorded paths. Each tick says where the object looks for a driver ([`crate::Contact`]).
//!
//! The turtles of Mystery Caves are here too, though they do not hurt: like the others they are
//! in the way of the kart, and what a touch does is thrown into a jump.

pub mod armadillo;
pub mod fireball;
pub mod minecart;
pub mod roller;
pub mod turtle;
