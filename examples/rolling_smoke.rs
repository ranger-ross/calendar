#[path = "../src/model.rs"]
mod model;
#[path = "../src/ui.rs"]
mod ui;

use chrono::{DateTime, Duration, NaiveDate, Offset};
use model::{Event, EventTime, View};
use std::{fs, path::PathBuf};

fn main() {
    let output = PathBuf::from(std::env::args().nth(1).unwrap());
    let mut events: Vec<_> = [
        ("2026-11-01", "Previous week"),
        ("2026-11-08", "Current week"),
        ("2026-11-28", "Fourth Saturday"),
        ("2026-11-29", "New fourth week"),
        ("2026-12-05", "Last visible Saturday"),
        ("2026-12-06", "Outside window"),
        ("2027-01-01", "New Year"),
        ("2028-02-29", "Leap day"),
    ]
    .into_iter()
    .map(|(date, title)| {
        let start = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        Event {
            title: title.into(),
            calendar: "Events".into(),
            start: EventTime::AllDay(start),
            end: EventTime::AllDay(start + Duration::days(1)),
            color: "#31a35e".into(),
        }
    })
    .collect();
    let start = DateTime::parse_from_rfc3339("2026-11-08T09:00:00+09:00")
        .unwrap()
        .to_utc();
    events.push(Event {
        title: "Planning session".into(),
        calendar: "Events".into(),
        start: EventTime::Timed(start),
        end: EventTime::Timed(start + Duration::hours(1)),
        color: "#039be5".into(),
    });
    for (name, timestamp, show_legend) in [
        ("saturday", "2026-11-07T23:59:59+09:00", false),
        ("sunday", "2026-11-08T00:00:01+09:00", false),
        ("legend", "2026-11-08T00:00:01+09:00", true),
        ("year", "2026-12-31T12:00:00+09:00", false),
        ("leap", "2028-02-29T12:00:00+09:00", false),
    ] {
        let now = DateTime::parse_from_rfc3339(timestamp)
            .unwrap()
            .with_timezone(&chrono_tz::Asia::Tokyo);
        let view = View {
            now,
            show_legend,
            events: &events,
            updated_at: Some(now.to_utc()),
            error: None,
        };
        let fetched = model::fetch_range(now.date_naive());
        let displayed = model::calendar_range(now.date_naive());
        assert!(fetched.0 <= displayed.0 && fetched.1 >= displayed.1);
        fs::write(output.join(format!("{name}.html")), ui::page(&view)).unwrap();
        fs::write(
            output.join(format!("{name}.grid.html")),
            ui::calendar(&view),
        )
        .unwrap();
        fs::write(
            output.join(format!("{name}.upcoming.html")),
            ui::upcoming(&view),
        )
        .unwrap();
        fs::write(
            output.join(format!("{name}.meta")),
            format!(
                "{}\n{}\n{}\n{}\n{}",
                now.timestamp_millis(),
                now.offset().fix().local_minus_utc(),
                ui::date(&view),
                ui::clock(&view),
                ui::status(&view),
            ),
        )
        .unwrap();
    }
}
