//! On, off, on: for text and icons that blink or flash. The clock is the caller's: the sim's
//! ticks for what stops with a paused race (retail's flashes are of its frames), real time for
//! what doesn't (a lobby's labels, the console's cursor).

use render::font::Gradient;

/// Whether a blink that is on for `period` and off for the next is on at `elapsed`, in seconds
/// of its clock.
pub(crate) fn on(elapsed: f32, period: f32) -> bool {
    ((elapsed / period) as u32).is_multiple_of(2)
}

/// [`on`] for a clock of ticks: on for `ticks`, then off for as many.
pub(crate) fn on_ticks(tick: u32, ticks: u32) -> bool {
    (tick / ticks).is_multiple_of(2)
}

/// Retail's text flash: orange while on, white while off.
pub(crate) fn flash(on: bool) -> Gradient {
    if on { Gradient::ORANGE } else { Gradient::WHITE }
}
