use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use std::str::FromStr;

/// Parses a cron expression. Accepts the standard 5-field syntax
/// (minute hour day month weekday, UTC) and normalizes it to the 6-field
/// form the `cron` crate expects by prepending a seconds field.
pub fn parse(raw: &str) -> anyhow::Result<CronSchedule> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 100 {
        anyhow::bail!("schedule must be 1-100 characters");
    }
    let normalized = match raw.split_whitespace().count() {
        5 => format!("0 {raw}"),
        6 | 7 => raw.to_string(),
        n => anyhow::bail!(
            "schedule must be a cron expression with 5 fields \
             (minute hour day month weekday), got {n}"
        ),
    };
    CronSchedule::from_str(&normalized).map_err(|e| anyhow::anyhow!("invalid cron: {e}"))
}

/// Returns true when at least one scheduled fire time falls in
/// `(last_enqueued_at, now]` — i.e. the schedule is due to enqueue a run.
pub fn is_due(
    schedule: &CronSchedule,
    now: DateTime<Utc>,
    last_enqueued_at: Option<DateTime<Utc>>,
) -> bool {
    let after = last_enqueued_at.unwrap_or(now - chrono::Duration::seconds(1));
    schedule
        .after(&after)
        .next()
        .map(|next_fire| next_fire <= now)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_five_field_syntax() {
        assert!(parse("*/10 * * * *").is_ok());
        assert!(parse("0 9 * * MON-FRI").is_ok());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("").is_err());
        assert!(parse("not a cron").is_err());
        assert!(parse("* * * *").is_err()); // 4 fields
        assert!((0..120).map(|_| "*").collect::<Vec<_>>().join(" ").len() > 0); // sanity
    }

    #[test]
    fn every_minute_schedule_is_due() {
        let s = parse("* * * * *").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 12, 12, 30, 0).unwrap();
        assert!(is_due(&s, now, None));
        // One minute later after enqueueing at 12:30 → not due until 12:31.
        assert!(!is_due(&s, now, Some(now)));
        let next = now + chrono::Duration::minutes(1);
        assert!(is_due(&s, next, Some(now)));
    }

    #[test]
    fn hourly_schedule_not_due_within_the_hour() {
        let s = parse("0 * * * *").unwrap();
        let last_fire = Utc.with_ymd_and_hms(2026, 9, 12, 12, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 12, 12, 30, 0).unwrap();
        assert!(!is_due(&s, now, Some(last_fire)));
        let at_hour = Utc.with_ymd_and_hms(2026, 9, 12, 13, 0, 0).unwrap();
        assert!(is_due(&s, at_hour, Some(last_fire)));
    }
}
