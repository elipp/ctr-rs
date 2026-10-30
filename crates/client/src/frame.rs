//! Frame rate limiting and the draw count log. The sim runs on its own clock (30 ticks a second),
//! so neither the display's rate nor the limit changes how the game plays.

use bevy::prelude::*;

use crate::config::Config;

/// Holds the loop to `Config::fps_max` (0 for no limit): sleeps most of what is left of the
/// frame, then spins the last stretch, as a sleep wakes up late.
pub(crate) fn limit_frame_rate(
    config: Res<Config>,
    mut previous: Local<Option<std::time::Instant>>,
) {
    use std::time::{Duration, Instant};
    const SPIN: Duration = Duration::from_micros(1500);
    let rate = config.fps_max as f64;
    let start = previous.unwrap_or_else(Instant::now);
    if rate > 0.0 {
        let target = start + Duration::from_secs_f64(1.0 / rate);
        if let Some(rest) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(rest.saturating_sub(SPIN));
        }
        while Instant::now() < target {
            std::hint::spin_loop();
        }
    }
    // From the target, not from now, so that one late frame does not slow the ones after it;
    // but never fall behind by more than a frame, or a stall would be followed by a burst.
    let now = Instant::now();
    *previous = Some(if rate > 0.0 {
        let step = Duration::from_secs_f64(1.0 / rate);
        (start + step).max(now.checked_sub(step).unwrap_or(now))
    } else {
        now
    });
}

/// Logs how many meshes the scene draws, a few frames in (once everything is spawned).
pub(crate) fn log_draws(
    mut frame: Local<u32>,
    meshes: Query<(&Mesh3d, Option<&Visibility>, Option<&InheritedVisibility>)>,
    assets: Res<Assets<Mesh>>,
) {
    *frame += 1;
    if *frame != 30 {
        return;
    }
    let (mut entities, mut visible, mut vertices) = (0, 0, 0);
    for (mesh, _, inherited) in &meshes {
        entities += 1;
        if inherited.is_none_or(|v| v.get()) {
            visible += 1;
            vertices += assets.get(&mesh.0).map_or(0, |m| m.count_vertices());
        }
    }
    debug!("{entities} mesh entities, {visible} visible, {vertices} vertices");
}
