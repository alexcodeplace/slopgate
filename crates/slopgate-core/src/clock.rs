//! Native UTC timestamps; no dependency on shell utilities or local timezone.
pub fn utc_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "timestamp-unavailable".to_string())
}
#[cfg(test)]
mod tests {
    #[test]
    fn clock_is_rfc3339_utc() {
        let value = super::utc_now();
        assert!(value.ends_with('Z'), "{value}");
        assert!(time::OffsetDateTime::parse(
            &value,
            &time::format_description::well_known::Rfc3339
        )
        .is_ok());
    }
}
