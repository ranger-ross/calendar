use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use chrono_tz::Tz;

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
    pub range_start: NaiveDate,
    pub weeks: usize,
    pub show_legend: bool,
    pub events: &'a [Event],
    pub updated_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

pub fn month_range(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = today.with_day(1).expect("valid first day");
    let next = (first + Duration::days(32))
        .with_day(1)
        .expect("valid first day");
    let start = first - Duration::days(first.weekday().num_days_from_sunday().into());
    let end = next + Duration::days(((7 - next.weekday().num_days_from_sunday()) % 7).into());
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_grid_covers_exact_full_sunday_first_weeks() {
        for (today, expected_start, expected_end) in [
            ("2026-02-12", "2026-02-01", "2026-03-01"),
            ("2026-11-03", "2026-11-01", "2026-12-06"),
            ("2026-08-31", "2026-07-26", "2026-09-06"),
            ("2028-02-29", "2028-01-30", "2028-03-05"),
            ("2026-12-31", "2026-11-29", "2027-01-03"),
        ] {
            let parse = |date: &str| NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
            assert_eq!(
                month_range(parse(today)),
                (parse(expected_start), parse(expected_end))
            );
        }
    }
}
