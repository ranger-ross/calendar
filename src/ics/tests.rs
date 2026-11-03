use super::*;

fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}

fn source() -> FeedConfig {
    FeedConfig {
        url: "https://example.invalid/private-secret".into(),
        color: "#123456".into(),
        name: None,
    }
}

fn calendar(body: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}\r\nEND:VCALENDAR\r\n")
}

fn event(body: &str) -> String {
    format!("BEGIN:VEVENT\r\n{body}\r\nEND:VEVENT\r\n")
}

fn parse(body: &str, start: &str, end: &str) -> Vec<Event> {
    parse_feed(
        &calendar(body),
        &source(),
        date(start),
        date(end),
        chrono_tz::America::New_York,
    )
    .unwrap()
}

fn instant(value: &EventTime) -> String {
    match value {
        EventTime::Timed(time) => time.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        EventTime::AllDay(date) => date.to_string(),
    }
}

#[test]
fn weekly_recurrence_exceptions_and_fall_dst() {
    let body = [
        event("UID:weekly\r\nDTSTART;TZID=America/New_York:20261027T090000\r\nDTEND;TZID=America/New_York:20261027T100000\r\nRRULE:FREQ=WEEKLY;COUNT=6\r\nEXDATE;TZID=America/New_York:20261110T090000\r\nSUMMARY:Weekly"),
        event("UID:weekly\r\nRECURRENCE-ID;TZID=America/New_York:20261117T090000\r\nDTSTART;TZID=America/New_York:20261118T100000\r\nSUMMARY:Moved"),
        event("UID:weekly\r\nRECURRENCE-ID;TZID=America/New_York:20261124T090000\r\nSTATUS:CANCELLED"),
    ].concat();
    let events = parse(&body, "2026-10-25", "2026-12-06");
    assert_eq!(
        events.iter().map(|e| instant(&e.start)).collect::<Vec<_>>(),
        [
            "2026-10-27T13:00:00Z",
            "2026-11-03T14:00:00Z",
            "2026-11-18T15:00:00Z",
            "2026-12-01T14:00:00Z",
        ]
    );
    assert_eq!(events[2].title, "Moved");
    assert_eq!(instant(&events[2].end), "2026-11-18T16:00:00Z");
    assert!(events.iter().all(|event| event.color == "#123456"));
}

#[test]
fn spring_gap_does_not_consume_count_and_fall_fold_uses_first_time() {
    let events = parse(
        &event(
            "UID:spring\r\nDTSTART;TZID=America/New_York:20260307T023000\r\nRRULE:FREQ=DAILY;COUNT=3",
        ),
        "2026-03-01",
        "2026-03-15",
    );
    assert_eq!(
        events.iter().map(|e| instant(&e.start)).collect::<Vec<_>>(),
        [
            "2026-03-07T07:30:00Z",
            "2026-03-09T06:30:00Z",
            "2026-03-10T06:30:00Z",
        ]
    );
    let events = parse(
        &event(
            "UID:fall\r\nDTSTART;TZID=America/New_York:20261031T013000\r\nRRULE:FREQ=DAILY;COUNT=3",
        ),
        "2026-10-30",
        "2026-11-04",
    );
    assert_eq!(
        events.iter().map(|e| instant(&e.start)).collect::<Vec<_>>(),
        [
            "2026-10-31T05:30:00Z",
            "2026-11-01T05:30:00Z",
            "2026-11-02T06:30:00Z",
        ]
    );
}

#[test]
fn folding_preserves_spaces_before_a_line_break() {
    let events = parse(
        &event(
            "UID:fold-space\r\nDTSTART;VALUE=DATE:20261103\r\nSUMMARY:Long \r\n folded \r\n title",
        ),
        "2026-11-01",
        "2026-12-01",
    );
    assert_eq!(events[0].title, "Long folded title");
}

#[test]
fn folded_escaped_titles_and_multiple_calendars() {
    let input = calendar(&format!(
        "X-WR-CALNAME:Public\\, Holidays\r\n{}",
        event(
            "UID:one\r\nDTSTART;VALUE=DATE:20261102\r\nSUMMARY:Long folded\r\n  title\\, slash\\\\semi\\;line\\nNext"
        )
    )) + &calendar(&event(
        "UID:two\r\nDTSTART;VALUE=DATE:20261103\r\nSUMMARY:Second",
    ));
    let events = parse_feed(
        &input,
        &source(),
        date("2026-11-01"),
        date("2026-12-01"),
        chrono_tz::UTC,
    )
    .unwrap();
    assert_eq!(events[0].title, "Long folded title, slash\\semi;line\nNext");
    assert_eq!(events[0].calendar, "Public, Holidays");
    assert_eq!(events[1].calendar, "Calendar");
    let mut source = source();
    source.name = Some("Configured".into());
    let events = parse_feed(
        &input,
        &source,
        date("2026-11-01"),
        date("2026-12-01"),
        chrono_tz::UTC,
    )
    .unwrap();
    assert!(events.iter().all(|event| event.calendar == "Configured"));
}

#[test]
fn all_day_exclusive_ends_duration_defaults_and_overlap() {
    let body = [
        event("UID:overlap\r\nDTSTART;VALUE=DATE:20261030\r\nDTEND;VALUE=DATE:20261103"),
        event("UID:default\r\nDTSTART;VALUE=DATE:20261104"),
        event("UID:duration\r\nDTSTART;VALUE=DATE:20261105\r\nDURATION:P2D"),
        event("UID:excluded\r\nDTSTART;VALUE=DATE:20261029\r\nDTEND;VALUE=DATE:20261101"),
        event("UID:recurring\r\nDTSTART;VALUE=DATE:20261001\r\nDURATION:P3D\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=31;COUNT=1"),
    ].concat();
    let events = parse(&body, "2026-11-01", "2026-11-10");
    assert_eq!(
        events
            .iter()
            .map(|e| (instant(&e.start), instant(&e.end)))
            .collect::<Vec<_>>(),
        [
            ("2026-10-30".into(), "2026-11-03".into()),
            ("2026-10-31".into(), "2026-11-03".into()),
            ("2026-11-04".into(), "2026-11-05".into()),
            ("2026-11-05".into(), "2026-11-07".into()),
        ]
    );
}

#[test]
fn timed_nominal_days_differ_from_exact_hours_across_dst() {
    let body = [
        event("UID:days\r\nDTSTART;TZID=America/New_York:20261031T120000\r\nDURATION:P1D\r\nSUMMARY:Days"),
        event("UID:hours\r\nDTSTART;TZID=America/New_York:20261031T120000\r\nDURATION:PT24H\r\nSUMMARY:Hours"),
        event("UID:point\r\nDTSTART:20261102T120000Z\r\nSUMMARY:Point"),
    ].concat();
    let events = parse(&body, "2026-11-01", "2026-11-03");
    assert_eq!(events.len(), 3);
    assert_eq!(instant(&events[0].end), "2026-11-01T17:00:00Z");
    assert_eq!(instant(&events[1].end), "2026-11-01T16:00:00Z");
    assert_eq!(instant(&events[2].start), instant(&events[2].end));
}

#[test]
fn rdates_exdates_and_dtstart_form_a_set() {
    let body = event(
        "UID:set\r\nDTSTART;VALUE=DATE:20261102\r\nRRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=2\r\nRDATE;VALUE=DATE:20261102,20261103,20261106\r\nEXDATE;VALUE=DATE:20261103",
    );
    let events = parse(&body, "2026-11-01", "2026-11-15");
    assert_eq!(
        events.iter().map(|e| instant(&e.start)).collect::<Vec<_>>(),
        ["2026-11-02", "2026-11-06", "2026-11-10"]
    );
    let excluded = event("UID:single\r\nDTSTART;VALUE=DATE:20261102\r\nEXDATE;VALUE=DATE:20261102");
    assert!(parse(&excluded, "2026-11-01", "2026-11-15").is_empty());
}

#[test]
fn moved_instances_cross_both_range_boundaries() {
    let body = [
        event("UID:moved\r\nDTSTART;VALUE=DATE:20261001\r\nRRULE:FREQ=MONTHLY;COUNT=3"),
        event("UID:moved\r\nRECURRENCE-ID;VALUE=DATE:20261001\r\nDTSTART;VALUE=DATE:20261105\r\nSUMMARY:In"),
        event("UID:moved\r\nRECURRENCE-ID;VALUE=DATE:20261101\r\nDTSTART;VALUE=DATE:20261205\r\nSUMMARY:Out"),
    ].concat();
    let events = parse(&body, "2026-11-01", "2026-12-01");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].title, "In");
}

#[test]
fn duplicate_revisions_and_cancelled_series() {
    let body = [
        event("UID:revision\r\nDTSTART;VALUE=DATE:20261102\r\nSEQUENCE:2\r\nDTSTAMP:20261002T000000Z\r\nSUMMARY:Newest"),
        event("UID:revision\r\nDTSTART;VALUE=DATE:20261103\r\nSEQUENCE:2\r\nDTSTAMP:20261001T000000Z\r\nSUMMARY:Older"),
        event("UID:revision\r\nDTSTART;VALUE=DATE:20261104\r\nSEQUENCE:1\r\nDTSTAMP:20261003T000000Z\r\nSUMMARY:Old sequence"),
        event("UID:cancelled\r\nDTSTART;VALUE=DATE:20261101\r\nRRULE:FREQ=DAILY;COUNT=3\r\nSEQUENCE:0"),
        event("UID:cancelled\r\nSTATUS:CANCELLED\r\nSEQUENCE:1"),
        event("UID:cancelled\r\nRECURRENCE-ID;VALUE=DATE:20261101\r\nDTSTART;VALUE=DATE:20261102"),
    ].concat();
    let events = parse(&body, "2026-11-01", "2026-12-01");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].title, "Newest");
}

#[test]
fn floating_times_use_feed_timezone_and_iana_vtimezone_is_accepted() {
    let body = format!(
        "X-WR-TIMEZONE:Europe/London\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nEND:VTIMEZONE\r\n{}",
        event("UID:floating\r\nDTSTART:20260701T090000\r\nRRULE:FREQ=DAILY;UNTIL=20260702T080000Z")
    );
    let events = parse(&body, "2026-07-01", "2026-07-04");
    assert_eq!(
        events.iter().map(|e| instant(&e.start)).collect::<Vec<_>>(),
        ["2026-07-01T08:00:00Z", "2026-07-02T08:00:00Z"]
    );
}

#[test]
fn rdate_periods_can_override_duration_and_overlap() {
    let body = event(
        "UID:period\r\nDTSTART:20261001T120000Z\r\nDURATION:PT1H\r\nRDATE;VALUE=PERIOD:20261031T120000Z/20261102T120000Z,20261104T120000Z/PT2H",
    );
    let events = parse(&body, "2026-11-01", "2026-11-05");
    assert_eq!(events.len(), 2);
    assert_eq!(instant(&events[0].end), "2026-11-02T12:00:00Z");
    assert_eq!(instant(&events[1].end), "2026-11-04T14:00:00Z");
}

#[test]
fn unsupported_features_and_malformed_input_fail_without_feed_details() {
    for body in [
        event("UID:bad\r\nDTSTART;TZID=private-secret:20261102T120000"),
        event("UID:bad\r\nDTSTART:20261102T120000Z\r\nRRULE:FREQ=DAILY;RSCALE=private-secret"),
        event(
            "UID:bad\r\nRECURRENCE-ID;RANGE=THISANDFUTURE:20261102T120000Z\r\nDTSTART:20261103T120000Z",
        ),
        event("UID:bad\r\nDTSTART:20261102T120000Z\r\nDURATION:P1\u{e9}"),
        event("DTSTART:private-secret"),
    ] {
        let error = parse_feed(
            &calendar(&body),
            &source(),
            date("2026-11-01"),
            date("2026-12-01"),
            chrono_tz::UTC,
        )
        .unwrap_err();
        assert!(!format!("{error:#}").contains("private-secret"));
    }
}

#[test]
fn expansion_limit_is_an_error_not_partial_results() {
    let body = event("UID:unbounded\r\nDTSTART:20261101T000000Z\r\nRRULE:FREQ=SECONDLY");
    let error = parse_feed(
        &calendar(&body),
        &source(),
        date("2026-11-01"),
        date("2026-11-03"),
        chrono_tz::UTC,
    )
    .unwrap_err();
    assert!(error.to_string().contains("expansion limit"));
}

#[test]
fn explicit_gap_start_preserves_the_original_recurrence_wall_time() {
    let events = parse(
        &event(
            "UID:gap-start\r\nDTSTART;TZID=America/New_York:20260308T023000\r\nRRULE:FREQ=DAILY;COUNT=2",
        ),
        "2026-03-01",
        "2026-03-15",
    );
    assert_eq!(
        events.iter().map(|e| instant(&e.start)).collect::<Vec<_>>(),
        ["2026-03-08T07:30:00Z", "2026-03-09T06:30:00Z",]
    );
}

#[test]
fn all_day_until_is_inclusive_and_override_revisions_are_deduplicated() {
    let body = [
        event("UID:until\r\nDTSTART;VALUE=DATE:20261101\r\nRRULE:FREQ=DAILY;UNTIL=20261103"),
        event("UID:until\r\nRECURRENCE-ID;VALUE=DATE:20261102\r\nDTSTART;VALUE=DATE:20261105\r\nSEQUENCE:1"),
        event("UID:until\r\nRECURRENCE-ID;VALUE=DATE:20261102\r\nSTATUS:CANCELLED\r\nSEQUENCE:2"),
    ].concat();
    let events = parse(&body, "2026-11-01", "2026-11-10");
    assert_eq!(
        events.iter().map(|e| instant(&e.start)).collect::<Vec<_>>(),
        ["2026-11-01", "2026-11-03"]
    );
}
