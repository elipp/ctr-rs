//! Race times as text. They are in retail's clock units, as `Race::final_ms` and the splits are:
//! its "milliseconds", which a tick adds 32 of.

/// A second of retail's clock.
pub const SECOND: i32 = 0x3c0;

/// `units` as (minutes, seconds, hundredths).
fn parts(units: u32) -> (u32, u32, u32) {
    let second = SECOND as u32;
    (units / (60 * second), units / second % 60, units % second * 100 / second)
}

/// `m:ss:cc`, as `UI_DrawRaceClock` writes it.
pub fn time(units: i32) -> String {
    let (minutes, seconds, hundredths) = parts(units.max(0) as u32);
    format!("{minutes}:{seconds:02}:{hundredths:02}")
}

/// A gap between two times: "-1.25", "+0.40", "+1:02.10".
pub fn delta(units: i32) -> String {
    let sign = match units.signum() {
        -1 => "-",
        1 => "+",
        _ => "",
    };
    let (minutes, seconds, hundredths) = parts(units.unsigned_abs());
    if minutes > 0 {
        format!("{sign}{minutes}:{seconds:02}.{hundredths:02}")
    } else {
        format!("{sign}{seconds}.{hundredths:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_minutes_seconds_and_hundredths() {
        assert_eq!(time(0), "0:00:00");
        assert_eq!(time(47_885), "0:49:88");
        assert_eq!(time(61 * SECOND), "1:01:00");
        assert_eq!(time(85 * SECOND), "1:25:00");
        assert_eq!(time(-32), "0:00:00");
    }

    #[test]
    fn a_delta_has_its_sign_and_minutes_only_if_any() {
        assert_eq!(delta(0), "0.00");
        assert_eq!(delta(-SECOND - SECOND / 4), "-1.25");
        assert_eq!(delta(62 * SECOND), "+1:02.00");
    }
}
