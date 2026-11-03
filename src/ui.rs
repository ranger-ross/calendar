use crate::model::{Event, EventTime, View};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Offset, Timelike};
use chrono_tz::Tz;
use std::fmt::Write;

pub fn date(view: &View) -> String {
    view.now.format("%A, %B %-d, %Y").to_string()
}

pub fn clock(view: &View) -> String {
    format!(
        "{}<sup>{}</sup> <span class=\"meridiem\">{}</span>",
        view.now.format("%-I:%M"),
        view.now.format("%S"),
        view.now.format("%P")
    )
}

pub fn status(view: &View) -> String {
    match (&view.error, view.updated_at) {
        (Some(error), Some(_)) => {
            format!("Calendar update failed: {error}. Showing saved events; last synced")
        }
        (Some(error), None) => format!("Calendar unavailable: {error}. Retrying automatically."),
        (None, Some(_)) => "Last synced".to_owned(),
        (None, None) => "Loading calendar feeds...".to_owned(),
    }
}

fn relative_age(seconds: i64) -> String {
    if seconds < 60 {
        return "less than a minute ago".to_owned();
    }
    let (count, unit) = if seconds < 3_600 {
        (seconds / 60, "minute")
    } else if seconds < 86_400 {
        (seconds / 3_600, "hour")
    } else {
        (seconds / 86_400, "day")
    };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

pub fn page(view: &View) -> String {
    let mut html = String::with_capacity(24_000);
    html.push_str("<!doctype html><html lang=\"en\" data-theme=\"dark\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\"><meta name=\"apple-mobile-web-app-capable\" content=\"yes\"><meta name=\"apple-mobile-web-app-status-bar-style\" content=\"black-translucent\"><meta name=\"theme-color\" content=\"#10151d\"><title>Calendar</title><link rel=\"stylesheet\" href=\"/calendar.css\"></head>");
    write!(html, "<body data-now-ms=\"{}\" data-offset-seconds=\"{}\"><main class=\"dashboard\"><header class=\"dashboard-header\"><button id=\"theme-toggle\" type=\"button\" aria-label=\"Toggle light theme\" aria-pressed=\"false\">Light theme</button><div id=\"date\" class=\"date\">", view.now.timestamp_millis(), view.now.offset().fix().local_minus_utc()).unwrap();
    escape_into(&mut html, &date(view));
    html.push_str("</div><div id=\"clock\" class=\"clock\" aria-label=\"Current time\">");
    html.push_str(&clock(view));
    html.push_str("</div></header><section id=\"calendar\" aria-label=\"Monthly calendar\">");
    html.push_str(&calendar(view));
    html.push_str("</section><footer><div id=\"status\" role=\"status\" aria-live=\"polite\"");
    if view.error.is_some() {
        html.push_str(" class=\"has-error\"");
    }
    html.push_str("><span id=\"status-message\">");
    escape_into(&mut html, &status(view));
    html.push_str("</span><span id=\"sync-age\"");
    if let Some(updated) = view.updated_at {
        write!(
            html,
            " data-updated-at-ms=\"{}\"> ",
            updated.timestamp_millis()
        )
        .unwrap();
        html.push_str(&relative_age(
            view.now.signed_duration_since(updated).num_seconds(),
        ));
    } else {
        html.push('>');
    }
    html.push_str(
        "</span></div></footer></main><script src=\"/calendar.js\"></script></body></html>",
    );
    html
}

struct Span<'a> {
    event: &'a Event,
    first: NaiveDate,
    last: NaiveDate,
    timed: Option<(DateTime<Tz>, DateTime<Tz>)>,
    color: &'a str,
    ink: &'static str,
}

fn span<'a>(event: &'a Event, timezone: Tz) -> Option<Span<'a>> {
    let color = safe_color(&event.color);
    match (&event.start, &event.end) {
        (EventTime::AllDay(start), EventTime::AllDay(end)) if end > start => Some(Span {
            event,
            first: *start,
            last: end.pred_opt()?,
            timed: None,
            color,
            ink: color_ink(color),
        }),
        (EventTime::Timed(start), EventTime::Timed(end)) if end >= start => {
            let start = start.with_timezone(&timezone);
            let end = end.with_timezone(&timezone);
            let last = if end > start
                && end.time().num_seconds_from_midnight() == 0
                && end.nanosecond() == 0
            {
                end.date_naive().pred_opt()?
            } else {
                end.date_naive()
            };
            Some(Span {
                event,
                first: start.date_naive(),
                last,
                timed: Some((start, end)),
                color,
                ink: "#ffffff",
            })
        }
        _ => None,
    }
}

pub fn calendar(view: &View) -> String {
    let mut html = String::with_capacity(20_000);
    let mut events: Vec<_> = view
        .events
        .iter()
        .filter_map(|event| span(event, view.now.timezone()))
        .collect();
    events.sort_by(|a, b| {
        a.timed
            .is_some()
            .cmp(&b.timed.is_some())
            .then_with(|| a.first.cmp(&b.first))
            .then_with(|| {
                a.timed
                    .as_ref()
                    .map(|(start, _)| start)
                    .cmp(&b.timed.as_ref().map(|(start, _)| start))
            })
            .then_with(|| a.event.title.cmp(&b.event.title))
    });
    if view.show_legend {
        html.push_str("<div class=\"legend\" aria-label=\"Calendar legend\"><span class=\"legend-kind\"><i class=\"legend-all-day\" aria-hidden=\"true\"></i>All day</span><span class=\"legend-kind\"><i class=\"legend-timed\" aria-hidden=\"true\"></i>Timed</span>");
        let mut calendars: Vec<(&str, &str)> = Vec::new();
        for event in view.events.iter() {
            let color = safe_color(&event.color);
            if !calendars
                .iter()
                .any(|&(name, existing)| name == event.calendar && existing == color)
            {
                calendars.push((&event.calendar, color));
            }
        }
        for (name, color) in calendars {
            write!(html, "<span class=\"legend-calendar\"><i style=\"background-color:#{color}\" aria-hidden=\"true\"></i>").unwrap();
            escape_into(&mut html, name);
            html.push_str("</span>");
        }
        html.push_str("</div>");
    }
    write!(html, "<table class=\"month-table weeks-{}{}\"><caption class=\"sr-only\">{}; events in {}</caption><thead><tr>", view.weeks, if view.show_legend { " with-legend" } else { "" }, view.now.format("%B %Y"), view.now.timezone()).unwrap();
    for (short, full) in [
        ("Sun", "Sunday"),
        ("Mon", "Monday"),
        ("Tue", "Tuesday"),
        ("Wed", "Wednesday"),
        ("Thu", "Thursday"),
        ("Fri", "Friday"),
        ("Sat", "Saturday"),
    ] {
        write!(
            html,
            "<th scope=\"col\"><abbr title=\"{full}\">{short}</abbr></th>"
        )
        .unwrap();
    }
    html.push_str("</tr></thead><tbody>");
    let today = view.now.date_naive();
    for week in 0..view.weeks {
        html.push_str("<tr>");
        for weekday in 0..7 {
            let day = view.range_start + Duration::days((week * 7 + weekday) as i64);
            html.push_str("<td class=\"day");
            if day.month() != today.month() || day.year() != today.year() {
                html.push_str(" adjacent");
            }
            if day == today {
                html.push_str(" today");
            }
            write!(html, "\"><div class=\"day-inner\"><time class=\"day-number\" datetime=\"{day}\" aria-label=\"{}\"{}>{}</time>", day.format("%A, %B %-d, %Y"), if day == today { " aria-current=\"date\"" } else { "" }, day.day()).unwrap();
            if day.day() == 1 {
                write!(
                    html,
                    " <span class=\"month-name\">{}</span>",
                    day.format("%B")
                )
                .unwrap();
            }
            write!(
                html,
                "<div class=\"events\" tabindex=\"0\" aria-label=\"Events for {}\">",
                day.format("%B %-d")
            )
            .unwrap();
            for event in &events {
                if event.first <= day && day <= event.last {
                    render_event(&mut html, event, day);
                }
            }
            html.push_str("</div></div></td>");
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table>");
    html
}

fn render_event(html: &mut String, span: &Span, day: NaiveDate) {
    let color = span.color;
    let kind = if span.timed.is_some() {
        "timed"
    } else {
        "all-day"
    };
    write!(
        html,
        "<div class=\"event {kind}\" style=\"--event-color:#{color};--event-ink:{}\" title=\"",
        span.ink
    )
    .unwrap();
    escape_into(html, &span.event.calendar);
    html.push_str(": ");
    escape_into(html, &span.event.title);
    html.push_str("\">");
    if let Some((start, end)) = &span.timed {
        html.push_str("<span class=\"event-time\">");
        if start.date_naive() < day {
            html.push_str("&larr; 12:00 am");
        } else {
            write!(html, "{}", start.format("%-I:%M %P")).unwrap();
        }
        html.push_str(" - ");
        if end.date_naive() > day {
            html.push_str("12:00 am");
            if span.last > day {
                html.push_str(" &rarr;");
            }
        } else {
            write!(html, "{}", end.format("%-I:%M %P")).unwrap();
        }
        html.push_str("</span>");
    } else {
        html.push_str("<span class=\"sr-only\">All day: </span>");
    }
    html.push_str("<span class=\"event-title\">");
    escape_into(html, &span.event.title);
    html.push_str("</span><span class=\"sr-only\">; ");
    escape_into(html, &span.event.calendar);
    html.push_str("</span></div>");
}

fn safe_color(color: &str) -> &str {
    let hex = color.strip_prefix('#').unwrap_or(color);
    if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        hex
    } else {
        "8ab4f8"
    }
}

fn color_ink(hex: &str) -> &'static str {
    let value = u32::from_str_radix(hex, 16).unwrap_or(0x8ab4f8);
    let channel = |value: u32| {
        let value = f64::from(value & 255) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance =
        0.2126 * channel(value >> 16) + 0.7152 * channel(value >> 8) + 0.0722 * channel(value);
    if luminance > 0.179 {
        "#10151d"
    } else {
        "#ffffff"
    }
}

fn escape_into(html: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            _ => html.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn day(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    fn timed(start: &str, end: &str) -> Event {
        Event {
            title: "Night shift".into(),
            calendar: "Work".into(),
            color: "#abcdef".into(),
            start: EventTime::Timed(
                DateTime::parse_from_rfc3339(start)
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            end: EventTime::Timed(
                DateTime::parse_from_rfc3339(end)
                    .unwrap()
                    .with_timezone(&Utc),
            ),
        }
    }

    fn view(events: &[Event]) -> View<'_> {
        View {
            now: chrono_tz::America::New_York
                .with_ymd_and_hms(2026, 11, 3, 21, 51, 2)
                .unwrap(),
            range_start: day(1),
            weeks: 5,
            show_legend: false,
            events,
            updated_at: None,
            error: None,
        }
    }

    #[test]
    fn all_day_end_is_exclusive_and_each_occupied_day_is_rendered() {
        let events = [Event {
            title: "Three days".into(),
            calendar: "Home".into(),
            color: "#123456".into(),
            start: EventTime::AllDay(day(2)),
            end: EventTime::AllDay(day(5)),
        }];
        let span = span(&events[0], chrono_tz::UTC).unwrap();
        assert_eq!((span.first, span.last), (day(2), day(4)));
        assert_eq!(
            calendar(&view(&events))
                .matches("class=\"event all-day\"")
                .count(),
            3
        );
    }

    #[test]
    fn midnight_end_uses_display_timezone_and_excludes_next_day() {
        let event = timed("2026-11-04T04:00:00Z", "2026-11-04T05:00:00Z");
        let span = span(&event, chrono_tz::America::New_York).unwrap();
        assert_eq!((span.first, span.last), (day(3), day(3)));
    }

    #[test]
    fn timed_events_continue_across_every_occupied_day() {
        let event = timed("2026-11-03T23:00:00Z", "2026-11-06T06:00:00Z");
        let events = [event];
        let html = calendar(&view(&events));
        assert_eq!(html.matches("class=\"event timed\"").count(), 4);
    }

    #[test]
    fn repeated_dst_hour_is_not_treated_as_an_empty_event() {
        let event = timed("2026-11-01T05:30:00Z", "2026-11-01T06:30:00Z");
        let span = span(&event, chrono_tz::America::New_York).unwrap();
        assert_eq!((span.first, span.last), (day(1), day(1)));
    }

    #[test]
    fn untrusted_text_and_invalid_colors_cannot_become_markup() {
        let events = [Event {
            title: "<script>\"&'".into(),
            calendar: "<img onerror='bad'>".into(),
            color: "#fff;\" onmouseover=\"bad".into(),
            start: EventTime::AllDay(day(3)),
            end: EventTime::AllDay(day(4)),
        }];
        let html = calendar(&view(&events));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("onmouseover"));
        assert!(html.contains("&lt;script&gt;&quot;&amp;&#39;"));
        assert!(html.contains("--event-color:#8ab4f8"));
        assert_eq!(safe_color("#aBc123"), "aBc123");
        assert_eq!(safe_color("abc"), "8ab4f8");
    }

    #[test]
    fn zero_duration_midnight_event_is_visible_on_its_start_day() {
        let event = timed("2026-11-03T05:00:00Z", "2026-11-03T05:00:00Z");
        let span = span(&event, chrono_tz::America::New_York).unwrap();
        assert_eq!((span.first, span.last), (day(3), day(3)));
    }
}
