use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};

const APPLE_TO_UNIX_SECONDS: f64 = 978_307_200.0;

pub fn apple_seconds_to_system_time(apple_seconds: f64) -> SystemTime {
    let unix_seconds = apple_seconds + APPLE_TO_UNIX_SECONDS;
    if unix_seconds >= 0.0 {
        UNIX_EPOCH + Duration::from_secs_f64(unix_seconds)
    } else {
        UNIX_EPOCH - Duration::from_secs_f64(-unix_seconds)
    }
}

pub fn format_apple_local(apple_seconds: f64) -> String {
    let system_time = apple_seconds_to_system_time(apple_seconds);
    let dt: DateTime<Local> = system_time.into();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn format_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_epoch_converts_to_unix_offset() {
        assert_eq!(
            apple_seconds_to_system_time(0.0),
            UNIX_EPOCH + Duration::from_secs(978_307_200)
        );
    }

    #[test]
    fn duration_formats_minutes_and_hours() {
        assert_eq!(format_duration(62.4), "1:02");
        assert_eq!(format_duration(3661.0), "1:01:01");
    }
}
