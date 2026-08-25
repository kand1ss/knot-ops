use std::time::SystemTime;

pub struct TimestampUtils;
impl TimestampUtils {
    pub fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    pub fn format_uptime(started_at_ms: u64) -> String {
        let now_ms = Self::now_ms();

        if started_at_ms > now_ms {
            return "just started".to_string();
        }

        let elapsed_secs = (now_ms - started_at_ms) / 1000;
        Self::format_duration(elapsed_secs)
    }

    pub fn format_duration(secs: u64) -> String {
        if secs < 60 {
            return format!("{}s", secs);
        }

        let minutes = secs / 60;
        let seconds = secs % 60;

        if minutes < 60 {
            return format!("{}m {}s", minutes, seconds);
        }

        let hours = minutes / 60;
        let mins = minutes % 60;

        if hours < 24 {
            return format!("{}h {}m", hours, mins);
        }

        let days = hours / 24;
        let hrs = hours % 24;

        format!("{}d {}h", days, hrs)
    }
}
