//! The objects that hurt (and one that throws): the armadillos of Dingo Canyon, the fireballs of
//! Mystery Caves, the minecarts of Dragon Mines, the snowball and barrels that roll along
//! recorded paths, the flame jets of Tiger Temple, the barrels of N. Gin Labs, the plants of
//! Papu's Pyramid, the spiders of Cortex Castle and the seals of Blizzard Bluff. Each tick says
//! where the object looks for a driver ([`crate::Contact`]).
//!
//! The turtles of Mystery Caves are here too, though they do not hurt: like the others they are
//! in the way of the kart, and what a touch does is thrown into a jump.

pub mod armadillo;
pub mod drum;
pub mod fireball;
pub mod flamejet;
pub mod minecart;
pub mod plant;
pub mod roller;
pub mod seal;
pub mod spider;
pub mod turtle;
