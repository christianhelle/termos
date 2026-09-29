//! Interactive prompt session for running many commands and queries.

use std::time::Duration;

/// Formats how long something took, such as "245 ms" or "1.23 s".
pub fn format_elapsed(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) {
        format!("{} ms", elapsed.as_millis())
    } else {
        format!("{:.2} s", elapsed.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_under_a_second_is_shown_in_milliseconds() {
        assert_eq!(format_elapsed(Duration::from_micros(245_900)), "245 ms");
        assert_eq!(format_elapsed(Duration::ZERO), "0 ms");
    }

    #[test]
    fn elapsed_of_a_second_or_more_is_shown_in_seconds() {
        assert_eq!(format_elapsed(Duration::from_millis(1_000)), "1.00 s");
        assert_eq!(format_elapsed(Duration::from_millis(12_346)), "12.35 s");
    }
}
