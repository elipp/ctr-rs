//! Frame rate limiting and the draw count log.

use bevy::prelude::*;

/// The frame rate the viewer is held to, in Hz; `CTR_FPS` overrides it, 0 for no limit.
pub(crate) const FRAME_RATE: f64 = 120.0;

// Holds the loop to [`FRAME_RATE`] now that the display does not (no vsync): sleeps most of
// what is left of the frame, then spins the last stretch, as a sleep wakes up late.
pub(crate) fn limit_frame_rate(
    mut previous: Local<Option<std::time::Instant>>,
    mut configured: Local<Option<f64>>,
) {
    use std::time::{Duration, Instant};
    const SPIN: Duration = Duration::from_micros(1500);
    let rate = *configured.get_or_insert_with(|| {
        std::env::var("CTR_FPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(FRAME_RATE)
    });
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
    info!("{entities} mesh entities, {visible} visible, {vertices} vertices");
}
