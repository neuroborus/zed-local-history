use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

const LOCAL_DISPLAY: &[time::format_description::FormatItem<'static>] =
    time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

/// Initialize local timezone detection for display formatting.
///
/// Binary entry points should call this once before formatting timestamps for humans.
pub fn init_local_offset_detection() {
    unsafe {
        time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Unsound);
    }
}

pub fn format_timestamp_local(raw: &str) -> String {
    let timestamp = match OffsetDateTime::parse(raw, &Rfc3339) {
        Ok(timestamp) => timestamp,
        Err(_) => return raw.to_string(),
    };
    let offset = local_utc_offset_at(timestamp).unwrap_or(UtcOffset::UTC);

    format_timestamp(timestamp, offset).unwrap_or_else(|| raw.to_string())
}

pub fn format_timestamp_with_offset(raw: &str, offset: UtcOffset) -> Option<String> {
    let timestamp = OffsetDateTime::parse(raw, &Rfc3339).ok()?;
    format_timestamp(timestamp, offset)
}

fn format_timestamp(timestamp: OffsetDateTime, offset: UtcOffset) -> Option<String> {
    let localized = timestamp.to_offset(offset);
    let formatted = localized.format(LOCAL_DISPLAY).ok()?;
    Some(format!("{formatted} {}", offset_suffix(offset)))
}

fn offset_suffix(offset: UtcOffset) -> String {
    if offset == UtcOffset::UTC {
        return "UTC".to_string();
    }

    let seconds = offset.whole_seconds();
    let sign = if seconds < 0 { '-' } else { '+' };
    let absolute = seconds.abs();
    let hours = absolute / 3_600;
    let minutes = (absolute % 3_600) / 60;
    let seconds = absolute % 60;

    if seconds == 0 {
        format!("{sign}{hours:02}:{minutes:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    }
}

fn local_utc_offset_at(timestamp: OffsetDateTime) -> Option<UtcOffset> {
    UtcOffset::local_offset_at(timestamp).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        format_timestamp_local, format_timestamp_with_offset, init_local_offset_detection,
        offset_suffix,
    };
    use time::format_description::well_known::Rfc3339;
    use time::{OffsetDateTime, UtcOffset};

    #[test]
    fn converts_utc_timestamp_to_requested_offset() {
        let offset = UtcOffset::from_hms(2, 0, 0).expect("offset must be valid");

        assert_eq!(
            format_timestamp_with_offset("2026-05-02T14:18:51Z", offset),
            Some("2026-05-02 16:18:51 +02:00".to_string())
        );
    }

    #[test]
    fn utc_offset_appends_utc_suffix() {
        assert_eq!(
            format_timestamp_with_offset("2026-05-02T14:18:51Z", UtcOffset::UTC),
            Some("2026-05-02 14:18:51 UTC".to_string())
        );
    }

    #[test]
    fn converts_subsecond_rfc3339_timestamp_to_requested_offset() {
        let offset = UtcOffset::from_hms(2, 0, 0).expect("offset must be valid");

        assert_eq!(
            format_timestamp_with_offset("2026-05-30T20:33:31.587487099Z", offset),
            Some("2026-05-30 22:33:31 +02:00".to_string())
        );
    }

    #[test]
    fn negative_offset_appends_offset_suffix() {
        let offset = UtcOffset::from_hms(-5, 30, 0).expect("offset must be valid");

        assert_eq!(
            format_timestamp_with_offset("2026-05-02T14:18:51Z", offset),
            Some("2026-05-02 08:48:51 -05:30".to_string())
        );
    }

    #[test]
    fn non_minute_offset_includes_seconds_suffix() {
        let offset = UtcOffset::from_hms(1, 2, 3).expect("offset must be valid");

        assert_eq!(offset_suffix(offset), "+01:02:03");
        assert_eq!(
            format_timestamp_with_offset("2026-05-02T14:18:51Z", offset),
            Some("2026-05-02 15:20:54 +01:02:03".to_string())
        );
    }

    #[test]
    fn local_format_uses_offset_after_init() {
        init_local_offset_detection();
        let raw = "2026-05-30T20:33:31.587487099Z";
        let formatted = format_timestamp_local(raw);
        assert_ne!(formatted, "2026-05-30T20:33:31.587487099Z");

        let timestamp = OffsetDateTime::parse(raw, &Rfc3339).expect("timestamp must parse");
        let expected_offset = UtcOffset::local_offset_at(timestamp).unwrap_or(UtcOffset::UTC);

        assert_eq!(
            formatted,
            format_timestamp_with_offset(raw, expected_offset).expect("timestamp must format")
        );
    }
}
