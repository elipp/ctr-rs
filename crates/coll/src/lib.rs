//! Track collision, ported from the reference `ctr-native` (`game/COLL.c`,
//! `include/namespace_Coll.h`).
//!
//! This is the `COLL_FIXED_*` family: a swept point test of a segment against
//! the track's quad blocks, found through the BSP tree. It is plain integer
//! code with no allocation; all of its working state lives in [`Collision`]
//! (the reference's scratchpad plus the GTE it uses), which is cheap to clone.
//!
//! The `COLL_MOVED_*` family (the player's swept test, [`Collision::search_moved`])
//! shares the BSP walk, scratchpad and normals; it adds the clip classification
//! of each hit, the scrub-depth tracking and the step-flag triggers.
//!
//! Left out for now:
//! * the low-LOD path for moved searches (the fixed one, used by the camera's
//!   ground search, is ported but not yet tested against the reference; every
//!   racer and bot search sets [`search_flags::HIGH_LOD`]);
//! * the `ThBuckColl` view of the scratchpad union, which overlays
//!   `QuadBlockColl` and is only used by those searches;
//! * `COLL_MOVED_PlayerSearch` and `COLL_MOVED_ScrubImpact` themselves, which
//!   drive and mutate a `Driver`; only their search setup is here
//!   ([`Collision::begin_player_search`], [`Collision::prepare_sweep`]).
//!
//! Pointers in the reference scratchpad (quad blocks, level vertices) are
//! indices into [`Level`] here. `tests/reference.rs` checks this port against
//! the C code on real tracks.

pub mod doors;
pub mod instance;

mod bsp;
mod helpers;
mod search;
mod triangle;
mod types;

use bsp::search_bsp;
use helpers::*;
pub use types::*;

pub use helpers::triangle_barycentrics;
