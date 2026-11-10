//! Learned bots: a policy that drives a kart from what [`race::nn::observe`] sees, trained from
//! scratch by PPO against the race itself.
//!
//! - [`tracks`]: the race tracks off the disc.
//! - [`env`]: a kart dropped anywhere on a track, driven by the policy's choices, and its reward.
//! - [`policy`]: the actor and the critic.
//! - [`ppo`]: the rollouts and the updates.

pub mod env;
pub mod policy;
pub mod ppo;
pub use race::rng;
pub mod tracks;
