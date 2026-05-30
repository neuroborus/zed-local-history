use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

const LOCAL_DISPLAY: &[time::format_description::FormatItem<'static>] =
    time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

/// Initialize local timezone detection for display formatting.
///
/// Binary entry points should call this once before formatting timestamps for humans.
pub fn init_local_offset_detection() {
    #[cfg(feature = "local-offset")]
    unsafe {
        time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Unsound);
    }
}

pub fn format_timestamp_local(raw: &str) -> String {
    format_timestamp_with_offset(raw, detect_local_offset()).unwrap_or_else(|| raw.to_string())
}

pub fn format_timestamp_with_offset(raw: &str, offset: UtcOffset) -> Option<String> {
    let timestamp = OffsetDateTime::parse(raw, &Rfc3339).ok()?;
    let localized = timestamp.to_offset(offset);
    let formatted = localized.format(LOCAL_DISPLAY).ok()?;

    if offset == UtcOffset::UTC {
        Some(format!("{formatted} UTC"))
    } else {
        Some(formatted)
    }
}

fn detect_local_offset() -> UtcOffset {
    local_utc_offset().unwrap_or(UtcOffset::UTC)
}

fn local_utc_offset() -> Option<UtcOffset> {
    UtcOffset::current_local_offset().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        format_timestamp_local, format_timestamp_with_offset, init_local_offset_detection,
    };
    use time::UtcOffset;

    #[test]
    fn converts_utc_timestamp_to_requested_offset() {
        let offset = UtcOffset::from_hms(2, 0, 0).expect("offset must be valid");

        assert_eq!(
            format_timestamp_with_offset("2026-05-02T14:18:51Z", offset),
            Some("2026-05-02 16:18:51".to_string())
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
            Some("2026-05-30 22:33:31".to_string())
        );
    }

    #[test]
    fn local_format_uses_offset_after_init() {
        init_local_offset_detection();
        let formatted = format_timestamp_local("2026-05-30T20:33:31.587487099Z");
        assert_ne!(formatted, "2026-05-30T20:33:31.587487099Z");

        if let Ok(offset) = UtcOffset::current_local_offset() {
            if offset == UtcOffset::UTC {
                assert_eq!(formatted, "2026-05-30 20:33:31 UTC");
            } else {
                assert_eq!(formatted, "2026-05-30 22:33:31");
            }
        }
    }
}
