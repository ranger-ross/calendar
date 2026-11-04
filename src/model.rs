use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use chrono_tz::Tz;

pub const UPCOMING_DAYS: i64 = 30;
pub const DISPLAY_WEEKS: usize = 4;

#[derive(Clone, Debug)]
pub enum EventTime {
    AllDay(NaiveDate),
    Timed(DateTime<Utc>),
}

#[derive(Clone, Debug)]
pub struct Event {
    pub title: String,
    pub calendar: String,
    pub start: EventTime,
    pub end: EventTime,
    pub color: String,
}

pub struct View<'a> {
    pub now: DateTime<Tz>,
    pub show_legend: bool,
    pub events: &'a [Event],
    pub updated_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

pub fn calendar_range(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = today - Duration::days(today.weekday().num_days_from_sunday().into());
    (start, start + Duration::weeks(DISPLAY_WEEKS as i64))
}

pub fn fetch_range(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let (start, end) = calendar_range(today);
    (start, end.max(today + Duration::days(UPCOMING_DAYS)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_window_covers_upcoming_events_across_month_and_year_boundaries() {
        let parse = |date: &str| NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        for (today, expected_start, expected_end) in [
            ("2026-02-28", "2026-02-22", "2026-03-30"),
            ("2026-12-31", "2026-12-27", "2027-01-30"),
            ("2026-11-03", "2026-11-01", "2026-12-03"),
        ] {
            assert_eq!(
                fetch_range(parse(today)),
                (parse(expected_start), parse(expected_end))
            );
        }
    }

    #[test]
    fn calendar_starts_with_current_week_and_spans_four_sunday_first_weeks() {
        for (today, expected_start, expected_end) in [
            ("2026-02-12", "2026-02-08", "2026-03-08"),
            ("2026-11-03", "2026-11-01", "2026-11-29"),
            ("2026-11-07", "2026-11-01", "2026-11-29"),
            ("2026-11-08", "2026-11-08", "2026-12-06"),
            ("2026-08-31", "2026-08-30", "2026-09-27"),
            ("2028-02-29", "2028-02-27", "2028-03-26"),
            ("2026-12-31", "2026-12-27", "2027-01-24"),
        ] {
            let parse = |date: &str| NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
            assert_eq!(
                calendar_range(parse(today)),
                (parse(expected_start), parse(expected_end))
            );
        }
    }
}
