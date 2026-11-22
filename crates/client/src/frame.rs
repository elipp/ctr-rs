//! Frame rate limiting and the draw count log. The sim runs on its own clock (30 ticks a second),
//! so neither the display's rate nor the limit changes how the game plays.
//!
//! The limit waits outside the schedule, so that a frame reads the input that came during the
//! wait: a sleep inside a system holds back a frame already decided, a whole frame of latency at
//! 30 fps. winit's own timed wait cannot follow a target that moves (a change of `WinitSettings`
//! forces an update at once), so the loop waits for good and a timer wakes it through the event
//! loop proxy, a little early: a thread natively, `setTimeout` in a browser. `spin` takes up the
//! rest, as either wakes late by a millisecond or a few.
//!
//! The frames lie on a grid of the fixed clock that meets a tick every `q` ticks (`fps / 30` as
//! `p / q`), so at a whole multiple of the tick rate every frame plays the same ticks and draws
//! at the same fractions between them; a frame on a free clock would sit at some phase of its
//! own, and if near a tick, play it a frame early or late by the jitter. The grid follows the
//! virtual clock's speed too (`lobby::pace`); while that is stopped the frames go by real time.
//!
//! A browser shows a frame at the display's next refresh whatever we do, so there a limit only
//! tells below the refresh rate, and comes out even only at a divisor of it.

use std::time::Duration;

use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::winit::{EventLoopProxy, UpdateMode, WinitSettings, WinitUserEvent};

use crate::config::Config;

pub(crate) struct PacePlugin;

impl Plugin for PacePlugin {
    fn build(&self, app: &mut App) {
        use bevy::time::TimeSystems;
        app.add_systems(First, (spin.before(TimeSystems), schedule.after(TimeSystems)));
    }

    fn finish(&self, app: &mut App) {
        let proxy = (**app.world().resource::<bevy::winit::EventLoopProxyWrapper>()).clone();
        app.insert_resource(Pacer {
            waker: Waker::start(proxy),
            target: None,
        });
    }
}

/// How early the timer wakes the loop, for `spin` to make up. A browser's timers are coarser,
/// and its clock too (`performance.now()` is rounded against timing attacks).
#[cfg(not(target_arch = "wasm32"))]
const SPIN: Duration = Duration::from_micros(1500);
#[cfg(target_arch = "wasm32")]
const SPIN: Duration = Duration::from_millis(3);
/// The most `spin` waits: a wakeup not of ours (Ctrl+C, a window added) comes at any time.
const SPIN_MOST: Duration = SPIN.saturating_mul(3);
/// How far past a grid point a frame starts, so that it surely has the tick there.
const PAST_POINT: Duration = Duration::from_micros(250);

/// Only the pacer's wakeups update: an input event waits in winit's queue for the next frame.
const WOKEN: UpdateMode = UpdateMode::Reactive {
    wait: Duration::MAX,
    react_to_device_events: false,
    react_to_user_events: true,
    react_to_window_events: false,
};

#[derive(Resource)]
struct Pacer {
    waker: Waker,
    /// When the next frame starts; none without a limit.
    target: Option<Instant>,
}

#[cfg(not(target_arch = "wasm32"))]
struct Waker(std::sync::mpsc::Sender<Instant>);

#[cfg(not(target_arch = "wasm32"))]
impl Waker {
    fn start(proxy: EventLoopProxy<WinitUserEvent>) -> Self {
        let (wake, at) = std::sync::mpsc::channel::<Instant>();
        std::thread::Builder::new()
            .name("frame pacer".into())
            .spawn(move || {
                let mut sent: Option<Instant> = None;
                while let Ok(mut next) = at.recv() {
                    while let Ok(later) = at.try_recv() {
                        next = later;
                    }
                    // A stray update asks for the wakeup already given again.
                    if sent.is_some_and(|sent| next <= sent) {
                        continue;
                    }
                    // A sleep, not a timed `recv`: on Windows only the sleep has the fine timer.
                    std::thread::sleep(next.saturating_duration_since(Instant::now()));
                    if proxy.send_event(WinitUserEvent::WakeUp).is_err() {
                        break;
                    }
                    sent = Some(next);
                }
            })
            .expect("the frame pacer thread starts");
        Self(wake)
    }

    fn wake_at(&mut self, at: Instant) {
        let _ = self.0.send(at);
    }
}

#[cfg(target_arch = "wasm32")]
struct Waker {
    proxy: EventLoopProxy<WinitUserEvent>,
    set: Option<Instant>,
}

#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = setTimeout)]
        pub fn set_timeout(handler: &JsValue, ms: f64) -> f64;
    }
}

#[cfg(target_arch = "wasm32")]
impl Waker {
    fn start(proxy: EventLoopProxy<WinitUserEvent>) -> Self {
        Self { proxy, set: None }
    }

    fn wake_at(&mut self, at: Instant) {
        // A stray update asks for the wakeup already set again.
        if self.set.is_some_and(|set| at <= set) {
            return;
        }
        self.set = Some(at);
        let proxy = self.proxy.clone();
        let wake = wasm_bindgen::closure::Closure::once_into_js(move || {
            let _ = proxy.send_event(WinitUserEvent::WakeUp);
        });
        let ms = at.saturating_duration_since(Instant::now()).as_secs_f64() * 1000.0;
        js::set_timeout(&wake, ms);
    }
}

fn spin(pacer: Res<Pacer>) {
    let Some(target) = pacer.target else { return };
    let until = target.min(Instant::now() + SPIN_MOST);
    while Instant::now() < until {
        std::hint::spin_loop();
    }
}

/// From where this frame started on the clocks, so how long it takes is no matter.
fn schedule(
    config: Res<Config>,
    mut pacer: ResMut<Pacer>,
    mut winit: ResMut<WinitSettings>,
    real: Res<Time<Real>>,
    virt: Res<Time<Virtual>>,
    fixed: Res<Time<Fixed>>,
) {
    let mode = if config.fps_max == 0 {
        UpdateMode::Continuous
    } else {
        WOKEN
    };
    if winit.focused_mode != mode || winit.unfocused_mode != mode {
        winit.focused_mode = mode;
        winit.unfocused_mode = mode;
    }
    if config.fps_max == 0 {
        pacer.target = None;
        return;
    }
    let start = real.last_update().unwrap_or_else(Instant::now);
    let speed = virt.effective_speed_f64();
    let target = if speed > 0.0 {
        let ahead = to_grid_point(virt.elapsed(), fixed.timestep(), config.fps_max);
        start + (ahead + PAST_POINT).div_f64(speed)
    } else {
        // A frame late by a whole period starts the grid over, rather than a burst after it.
        let period = Duration::from_secs_f64(1.0 / config.fps_max as f64);
        match pacer.target {
            Some(last) if last + period > start => last + period,
            _ => start + period,
        }
    };
    pacer.target = Some(target);
    pacer.waker.wake_at(target.checked_sub(SPIN).unwrap_or(target));
}

/// How far the clock at `now` is from the next point of the frame grid, strictly after it.
/// In whole nanoseconds, so that the grid meets the ticks exactly, not just nearly.
fn to_grid_point(now: Duration, tick: Duration, fps: u32) -> Duration {
    let hz = crate::GAME_TICK_HZ as u32;
    let common = gcd(fps, hz);
    let (p, q) = (u128::from(fps / common), u128::from(hz / common));
    let block = tick.as_nanos() * q;
    let into = now.as_nanos() % block;
    let mut m = into * p / block + 1;
    while m * block / p <= into {
        m += 1;
    }
    Duration::from_nanos((m * block / p - into) as u64)
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_nanos(33_333_333);

    #[test]
    fn a_multiple_of_the_tick_rate_meets_every_tick() {
        assert_eq!(to_grid_point(TICK * 7, TICK, 60), TICK / 2);
        assert_eq!(
            to_grid_point(TICK * 7 + Duration::from_millis(1), TICK, 30),
            TICK - Duration::from_millis(1)
        );
        assert_eq!(
            to_grid_point(TICK * 7 - Duration::from_nanos(1), TICK, 90),
            Duration::from_nanos(1)
        );
    }

    #[test]
    fn a_divisor_meets_every_qth_tick() {
        assert_eq!(to_grid_point(TICK * 7, TICK, 15), TICK);
        assert_eq!(to_grid_point(TICK * 8, TICK, 15), TICK * 2);
    }

    #[test]
    fn any_other_rate_meets_a_tick_now_and_then() {
        // 144 = 30 · 24 / 5: 24 frames to 5 ticks.
        let mut now = TICK * 10;
        for _ in 0..24 {
            now += to_grid_point(now, TICK, 144);
        }
        assert_eq!(now, TICK * 15);
    }
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
