use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use anyhow::{Result, anyhow, bail, ensure};
use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use ical::{IcalParser, parser::ical::component::IcalEvent, property::Property};
use rrule::{RRule, RRuleSet, Unvalidated};

use crate::config::FeedConfig;
use crate::model::{Event, EventTime};

const EXPANSION_LIMIT: u16 = 65_535;
const EVENT_LIMIT: usize = 100_000;

#[derive(Clone, Copy, Debug)]
enum Moment {
    Date(NaiveDate),
    Time(DateTime<Tz>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Key {
    Date(NaiveDate),
    Time(i64),
}

impl Moment {
    fn key(self) -> Key {
        match self {
            Self::Date(date) => Key::Date(date),
            Self::Time(time) => Key::Time(time.timestamp()),
        }
    }

    fn wall(self) -> NaiveDateTime {
        match self {
            Self::Date(date) => date.and_hms_opt(0, 0, 0).unwrap(),
            Self::Time(time) => time.naive_local(),
        }
    }

    fn model(self) -> EventTime {
        match self {
            Self::Date(date) => EventTime::AllDay(date),
            Self::Time(time) => EventTime::Timed(time.with_timezone(&Utc)),
        }
    }

    fn same_kind(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Date(_), Self::Date(_)) | (Self::Time(_), Self::Time(_))
        )
    }
}

#[derive(Clone, Copy)]
enum Length {
    Days(i64),
    Exact(Duration),
    Nominal { days: i64, seconds: i64 },
}

impl Length {
    fn end(self, start: Moment) -> Result<Moment> {
        match (self, start) {
            (Self::Days(days), Moment::Date(date)) => Ok(Moment::Date(
                date.checked_add_signed(Duration::try_days(days).ok_or_else(range_error)?)
                    .ok_or_else(range_error)?,
            )),
            (Self::Exact(duration), Moment::Time(time)) => Ok(Moment::Time(
                time.checked_add_signed(duration).ok_or_else(range_error)?,
            )),
            (Self::Nominal { days, seconds }, Moment::Time(time)) => {
                let local = time
                    .naive_local()
                    .checked_add_signed(Duration::try_days(days).ok_or_else(range_error)?)
                    .ok_or_else(range_error)?;
                let end = resolve_local(time.timezone(), local)?
                    .checked_add_signed(Duration::try_seconds(seconds).ok_or_else(range_error)?)
                    .ok_or_else(range_error)?;
                Ok(Moment::Time(end))
            }
            _ => bail!("ICS duration and date types do not match"),
        }
    }
}

fn range_error() -> anyhow::Error {
    anyhow!("ICS date or duration is outside the supported range")
}

fn prop<'a>(properties: &'a [Property], name: &str) -> Option<&'a Property> {
    properties
        .iter()
        .find(|property| property.name.eq_ignore_ascii_case(name))
}

fn value(property: &Property) -> Result<&str> {
    property
        .value
        .as_deref()
        .ok_or_else(|| anyhow!("ICS property has no value"))
}

fn param<'a>(property: &'a Property, name: &str) -> Result<Option<&'a str>> {
    let Some(params) = &property.params else {
        return Ok(None);
    };
    let mut matches = params
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name));
    let Some((_, values)) = matches.next() else {
        return Ok(None);
    };
    ensure!(
        matches.next().is_none() && values.len() == 1,
        "Invalid ICS property parameter"
    );
    Ok(Some(values[0].trim_matches('"')))
}

fn text(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n' | 'N') => result.push('\n'),
                Some(next) => result.push(next),
                None => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn zone(name: &str) -> Result<Tz> {
    name.trim_matches('"')
        .parse()
        .map_err(|_| anyhow!("Unsupported ICS timezone; an IANA timezone is required"))
}

// RFC 5545 chooses the first repeated time and the pre-gap offset for explicit dates.
fn resolve_local(timezone: Tz, local: NaiveDateTime) -> Result<DateTime<Tz>> {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(time) => Ok(time),
        LocalResult::Ambiguous(first, _) => Ok(first),
        LocalResult::None => {
            for hours in 1..=48 {
                let before = local
                    .checked_sub_signed(Duration::hours(hours))
                    .ok_or_else(range_error)?;
                if let Some(time) = timezone.from_local_datetime(&before).earliest() {
                    let utc = local
                        .checked_sub_signed(Duration::seconds(i64::from(
                            time.offset().fix().local_minus_utc(),
                        )))
                        .ok_or_else(range_error)?;
                    return Ok(timezone.from_utc_datetime(&utc));
                }
            }
            bail!("Unable to resolve ICS local time")
        }
    }
}

fn parse_moment(property: &Property, input: &str, fallback: Tz) -> Result<Moment> {
    let kind = param(property, "VALUE")?.unwrap_or("DATE-TIME");
    if kind.eq_ignore_ascii_case("DATE") {
        ensure!(
            param(property, "TZID")?.is_none(),
            "TZID is invalid on an ICS DATE"
        );
        return NaiveDate::parse_from_str(input, "%Y%m%d")
            .map(Moment::Date)
            .map_err(|_| anyhow!("Invalid ICS DATE"));
    }
    ensure!(
        kind.eq_ignore_ascii_case("DATE-TIME") || kind.eq_ignore_ascii_case("PERIOD"),
        "Unsupported ICS date value type"
    );
    let utc = input.ends_with('Z');
    let local = NaiveDateTime::parse_from_str(input.trim_end_matches('Z'), "%Y%m%dT%H%M%S")
        .map_err(|_| anyhow!("Invalid ICS DATE-TIME"))?;
    let tzid = param(property, "TZID")?;
    ensure!(
        !utc || tzid.is_none(),
        "UTC ICS dates cannot also specify TZID"
    );
    let timezone = if utc {
        chrono_tz::UTC
    } else {
        tzid.map(zone).transpose()?.unwrap_or(fallback)
    };
    Ok(Moment::Time(resolve_local(timezone, local)?))
}

fn date_property(property: &Property, fallback: Tz) -> Result<Moment> {
    parse_moment(property, value(property)?, fallback)
}

fn duration(input: &str) -> Result<(i64, i64)> {
    let input = input.strip_prefix('+').unwrap_or(input);
    let mut rest = input
        .strip_prefix('P')
        .ok_or_else(|| anyhow!("Invalid ICS DURATION"))?;
    let (mut days, mut seconds, mut time, mut seen, mut order) = (0i64, 0i64, false, false, 0);
    while !rest.is_empty() {
        if let Some(next) = rest.strip_prefix('T') {
            ensure!(!time && !next.is_empty(), "Invalid ICS DURATION");
            time = true;
            rest = next;
            continue;
        }
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        ensure!(digits > 0 && digits < rest.len(), "Invalid ICS DURATION");
        let number: i64 = rest[..digits].parse().map_err(|_| range_error())?;
        let unit = rest.as_bytes()[digits];
        ensure!(unit.is_ascii(), "Invalid ICS DURATION unit");
        rest = &rest[digits + 1..];
        let (position, multiplier) = match (time, unit) {
            (false, b'W') => {
                ensure!(!seen && rest.is_empty(), "Invalid ICS week duration");
                (1, 7)
            }
            (false, b'D') => (2, 1),
            (true, b'H') => (3, 3600),
            (true, b'M') => (4, 60),
            (true, b'S') => (5, 1),
            _ => bail!("Invalid ICS DURATION"),
        };
        ensure!(position > order, "Invalid ICS DURATION order");
        order = position;
        seen = true;
        let amount = number.checked_mul(multiplier).ok_or_else(range_error)?;
        let target = if time { &mut seconds } else { &mut days };
        *target = target.checked_add(amount).ok_or_else(range_error)?;
    }
    ensure!(seen, "Empty ICS DURATION");
    Ok((days, seconds))
}

fn length(
    properties: &[Property],
    start: Moment,
    fallback: Tz,
    inherited: Option<Length>,
) -> Result<Length> {
    let end = prop(properties, "DTEND");
    let dur = prop(properties, "DURATION");
    ensure!(
        end.is_none() || dur.is_none(),
        "ICS event specifies both DTEND and DURATION"
    );
    if let Some(end) = end {
        let end = date_property(end, fallback)?;
        return match (start, end) {
            (Moment::Date(start), Moment::Date(end)) => {
                ensure!(end > start, "ICS all-day DTEND must follow DTSTART");
                Ok(Length::Days((end - start).num_days()))
            }
            (Moment::Time(start), Moment::Time(end)) => {
                ensure!(end >= start, "ICS DTEND precedes DTSTART");
                Ok(Length::Exact(end.signed_duration_since(start)))
            }
            _ => bail!("ICS DTSTART and DTEND types do not match"),
        };
    }
    if let Some(dur) = dur {
        let (days, seconds) = duration(value(dur)?)?;
        return match start {
            Moment::Date(_) => {
                ensure!(
                    seconds == 0 && days > 0,
                    "All-day ICS duration must be positive whole days"
                );
                Ok(Length::Days(days))
            }
            Moment::Time(_) => Ok(Length::Nominal { days, seconds }),
        };
    }
    Ok(inherited.unwrap_or(match start {
        Moment::Date(_) => Length::Days(1),
        Moment::Time(_) => Length::Exact(Duration::zero()),
    }))
}

#[derive(Clone)]
struct Item {
    event: IcalEvent,
    recurrence: Option<Moment>,
    revision: (u32, Option<DateTime<Utc>>, Option<DateTime<Utc>>),
}

fn revision_time(properties: &[Property], name: &str) -> Result<Option<DateTime<Utc>>> {
    prop(properties, name)
        .map(|property| {
            ensure!(
                value(property)?.ends_with('Z'),
                "ICS revision timestamp must be UTC"
            );
            match date_property(property, chrono_tz::UTC)? {
                Moment::Time(time) => Ok(time.with_timezone(&Utc)),
                _ => bail!("Invalid ICS revision timestamp"),
            }
        })
        .transpose()
}

fn cancelled(properties: &[Property]) -> bool {
    prop(properties, "STATUS")
        .and_then(|p| p.value.as_deref())
        .is_some_and(|status| status.eq_ignore_ascii_case("CANCELLED"))
}

struct Window {
    start: NaiveDate,
    end: NaiveDate,
    utc_start: DateTime<Utc>,
    utc_end: DateTime<Utc>,
}

impl Window {
    fn contains(&self, start: Moment, end: Moment) -> bool {
        match (start, end) {
            (Moment::Date(start), Moment::Date(end)) => start < self.end && end > self.start,
            (Moment::Time(start), Moment::Time(end)) => {
                start < self.utc_end
                    && (end > self.utc_start || (start == end && start >= self.utc_start))
            }
            _ => false,
        }
    }
}

fn emit(
    events: &mut Vec<Event>,
    start: Moment,
    length: Length,
    title: &str,
    calendar: &str,
    source: &FeedConfig,
    window: &Window,
) -> Result<()> {
    let end = length.end(start)?;
    if window.contains(start, end) {
        ensure!(
            events.len() < EVENT_LIMIT,
            "ICS event expansion limit exceeded"
        );
        events.push(Event {
            title: title.to_owned(),
            calendar: calendar.to_owned(),
            start: start.model(),
            end: end.model(),
            color: source.color.clone(),
        });
    }
    Ok(())
}

// Expand wall-clock dates in UTC: rrule's timezone conversion shifts DST gaps.
// Resolve them ourselves so gaps do not consume COUNT and folds use the first time.
fn expand_rule(
    rule: &Property,
    start: Moment,
    wall_start: NaiveDateTime,
    window: &Window,
) -> Result<Vec<Moment>> {
    let mut parts = Vec::new();
    let (mut count, mut until) = (None, None);
    let mut names = BTreeSet::new();
    for part in value(rule)?.split(';') {
        let (name, value) = part
            .split_once('=')
            .ok_or_else(|| anyhow!("Invalid ICS RRULE"))?;
        ensure!(
            names.insert(name.to_ascii_uppercase()),
            "Duplicate ICS RRULE part"
        );
        match name.to_ascii_uppercase().as_str() {
            "COUNT" => {
                let parsed: usize = value
                    .parse()
                    .map_err(|_| anyhow!("Invalid ICS recurrence COUNT"))?;
                ensure!(parsed > 0, "Invalid ICS recurrence COUNT");
                count = Some(parsed);
            }
            "UNTIL" => {
                let property = Property {
                    name: "UNTIL".into(),
                    params: if matches!(start, Moment::Date(_)) {
                        Some(vec![("VALUE".into(), vec!["DATE".into()])])
                    } else {
                        None
                    },
                    value: None,
                };
                let fallback = match start {
                    Moment::Time(time) => time.timezone(),
                    _ => chrono_tz::UTC,
                };
                let parsed = parse_moment(&property, value, fallback)?;
                ensure!(
                    start.same_kind(parsed),
                    "ICS UNTIL and DTSTART types do not match"
                );
                until = Some(parsed);
            }
            "FREQ" | "INTERVAL" | "BYSECOND" | "BYMINUTE" | "BYHOUR" | "BYDAY" | "BYMONTHDAY"
            | "BYYEARDAY" | "BYWEEKNO" | "BYMONTH" | "BYSETPOS" | "WKST" => {
                if matches!(start, Moment::Date(_)) {
                    ensure!(
                        !matches!(
                            name.to_ascii_uppercase().as_str(),
                            "BYSECOND" | "BYMINUTE" | "BYHOUR"
                        ),
                        "Time components are invalid in all-day ICS recurrence"
                    );
                    if name.eq_ignore_ascii_case("FREQ") {
                        ensure!(
                            !matches!(
                                value.to_ascii_uppercase().as_str(),
                                "SECONDLY" | "MINUTELY" | "HOURLY"
                            ),
                            "Subdaily all-day ICS recurrence is unsupported"
                        );
                    }
                }
                parts.push(part.to_ascii_uppercase());
            }
            _ => bail!("Unsupported ICS RRULE feature"),
        }
    }
    ensure!(
        count.is_none() || until.is_none(),
        "ICS RRULE cannot contain both COUNT and UNTIL"
    );
    let dt_start = rrule::Tz::UTC.from_utc_datetime(&wall_start);
    let parsed: RRule<Unvalidated> = parts
        .join(";")
        .parse()
        .map_err(|_| anyhow!("Invalid or unsupported ICS RRULE"))?;
    let parsed = parsed
        .validate(dt_start)
        .map_err(|_| anyhow!("Invalid or unsupported ICS RRULE"))?;
    // Two days cover all IANA offset differences between the display and event zones.
    let mut last = window
        .end
        .checked_add_signed(Duration::days(2))
        .ok_or_else(range_error)?
        .and_hms_opt(0, 0, 0)
        .unwrap();
    if let Some(until) = until {
        last = last.min(
            until
                .wall()
                .checked_add_signed(Duration::days(2))
                .ok_or_else(range_error)?,
        );
    }
    if dt_start.naive_utc() > last {
        return Ok(Vec::new());
    }
    let set = RRuleSet::new(dt_start)
        .rrule(parsed)
        .before(rrule::Tz::UTC.from_utc_datetime(&last));
    let mut limit = count.map_or(EXPANSION_LIMIT, |count| {
        count.saturating_add(1).min(usize::from(EXPANSION_LIMIT)) as u16
    });
    loop {
        let result = set.clone().all(limit);
        let mut dates = Vec::new();
        for wall in result.dates {
            let moment = match start {
                Moment::Date(_) => Moment::Date(wall.date_naive()),
                Moment::Time(_) if wall.naive_utc() == wall_start => start,
                Moment::Time(time) => {
                    let Some(time) = time
                        .timezone()
                        .from_local_datetime(&wall.naive_utc())
                        .earliest()
                    else {
                        continue;
                    };
                    Moment::Time(time)
                }
            };
            if until.is_some_and(|until| moment.key() > until.key()) {
                return Ok(dates);
            }
            dates.push(moment);
            if count.is_some_and(|count| dates.len() >= count) {
                return Ok(dates);
            }
        }
        if !result.limited {
            return Ok(dates);
        }
        ensure!(
            limit < EXPANSION_LIMIT,
            "ICS recurrence expansion limit exceeded"
        );
        limit = limit.saturating_mul(2);
    }
}

fn recurrence_dates(
    properties: &[Property],
    start: Moment,
    length: Length,
    timezone: Tz,
    window: &Window,
) -> Result<BTreeMap<Key, (Moment, Length)>> {
    ensure!(
        prop(properties, "EXRULE").is_none(),
        "Unsupported ICS EXRULE; use EXDATE"
    );
    let mut dates = BTreeMap::new();
    dates.insert(start.key(), (start, length));
    let wall_start = if matches!(start, Moment::Time(_)) {
        let property =
            prop(properties, "DTSTART").ok_or_else(|| anyhow!("ICS event has no DTSTART"))?;
        NaiveDateTime::parse_from_str(value(property)?.trim_end_matches('Z'), "%Y%m%dT%H%M%S")
            .map_err(|_| anyhow!("Invalid ICS DATE-TIME"))?
    } else {
        start.wall()
    };
    for rule in properties
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("RRULE"))
    {
        for date in expand_rule(rule, start, wall_start, window)? {
            dates.insert(date.key(), (date, length));
            ensure!(
                dates.len() <= EVENT_LIMIT,
                "ICS recurrence expansion limit exceeded"
            );
        }
    }
    for property in properties
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("RDATE"))
    {
        for entry in value(property)?.split(',') {
            let (date, period) = entry
                .split_once('/')
                .map_or((entry, None), |(date, end)| (date, Some(end)));
            let date = parse_moment(property, date, timezone)?;
            ensure!(
                start.same_kind(date),
                "ICS RDATE and DTSTART types do not match"
            );
            let length = if let Some(end) = period {
                ensure!(
                    param(property, "VALUE")?.is_some_and(|v| v.eq_ignore_ascii_case("PERIOD")),
                    "ICS RDATE period requires VALUE=PERIOD"
                );
                if end.starts_with('P') || end.starts_with('+') {
                    let (days, seconds) = duration(end)?;
                    ensure!(
                        days > 0 || seconds > 0,
                        "ICS RDATE period must have positive duration"
                    );
                    Length::Nominal { days, seconds }
                } else {
                    match (date, parse_moment(property, end, timezone)?) {
                        (Moment::Time(start), Moment::Time(end)) if end > start => {
                            Length::Exact(end.signed_duration_since(start))
                        }
                        _ => bail!("Invalid ICS RDATE period"),
                    }
                }
            } else {
                ensure!(
                    !param(property, "VALUE")?.is_some_and(|v| v.eq_ignore_ascii_case("PERIOD")),
                    "ICS RDATE period has no end"
                );
                length
            };
            dates.insert(date.key(), (date, length));
            ensure!(
                dates.len() <= EVENT_LIMIT,
                "ICS recurrence expansion limit exceeded"
            );
        }
    }
    for property in properties
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("EXDATE"))
    {
        for entry in value(property)?.split(',') {
            let date = parse_moment(property, entry, timezone)?;
            ensure!(
                start.same_kind(date),
                "ICS EXDATE and DTSTART types do not match"
            );
            dates.remove(&date.key());
        }
    }
    Ok(dates)
}

fn unfold_lines(text: &str) -> std::borrow::Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut output: Option<String> = None;
    let mut start = 0;
    for (index, pair) in bytes.windows(2).enumerate() {
        if pair[0] == b'\n' && matches!(pair[1], b' ' | b'\t') {
            let end = if index > 0 && bytes[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            output
                .get_or_insert_with(|| String::with_capacity(text.len()))
                .push_str(&text[start..end]);
            start = index + 2;
        }
    }
    match output {
        Some(mut output) => {
            output.push_str(&text[start..]);
            std::borrow::Cow::Owned(output)
        }
        None => std::borrow::Cow::Borrowed(text),
    }
}

pub fn parse_feed(
    text_input: &str,
    source: &FeedConfig,
    start: NaiveDate,
    end: NaiveDate,
    timezone: Tz,
) -> Result<Vec<Event>> {
    ensure!(start < end, "Invalid ICS display range");
    let window = Window {
        start,
        end,
        utc_start: resolve_local(timezone, start.and_hms_opt(0, 0, 0).unwrap())?
            .with_timezone(&Utc),
        utc_end: resolve_local(timezone, end.and_hms_opt(0, 0, 0).unwrap())?.with_timezone(&Utc),
    };
    let mut output = Vec::new();
    let mut calendars = 0;
    // Preserve fold-boundary spaces before ical trims each physical line.
    let unfolded = unfold_lines(text_input.trim_start_matches('\u{feff}'));
    for calendar in IcalParser::new(Cursor::new(unfolded.as_bytes())) {
        let calendar = calendar.map_err(|_| anyhow!("Invalid ICS calendar syntax"))?;
        calendars += 1;
        if let Some(scale) = prop(&calendar.properties, "CALSCALE") {
            ensure!(
                value(scale)?.eq_ignore_ascii_case("GREGORIAN"),
                "Unsupported ICS calendar scale"
            );
        }
        for definition in &calendar.timezones {
            let id = prop(&definition.properties, "TZID")
                .ok_or_else(|| anyhow!("ICS VTIMEZONE has no TZID"))?;
            zone(value(id)?)?;
        }
        let fallback = prop(&calendar.properties, "X-WR-TIMEZONE")
            .map(|p| value(p).and_then(zone))
            .transpose()?
            .unwrap_or(timezone);
        let label = source
            .name
            .clone()
            .or_else(|| {
                prop(&calendar.properties, "X-WR-CALNAME")
                    .and_then(|p| p.value.as_deref())
                    .map(text)
            })
            .unwrap_or_else(|| "Calendar".into());
        let method_cancel = prop(&calendar.properties, "METHOD")
            .and_then(|p| p.value.as_deref())
            .is_some_and(|method| method.eq_ignore_ascii_case("CANCEL"));
        let mut series: BTreeMap<String, BTreeMap<Option<Key>, Item>> = BTreeMap::new();
        for mut event in calendar.events {
            if method_cancel && prop(&event.properties, "STATUS").is_none() {
                event.properties.push(Property {
                    name: "STATUS".into(),
                    params: None,
                    value: Some("CANCELLED".into()),
                });
            }
            let uid =
                prop(&event.properties, "UID").ok_or_else(|| anyhow!("ICS event has no UID"))?;
            let uid = value(uid)?.to_owned();
            ensure!(!uid.is_empty(), "ICS event has an empty UID");
            let recurrence = prop(&event.properties, "RECURRENCE-ID")
                .map(|property| {
                    ensure!(
                        param(property, "RANGE")?.is_none(),
                        "Unsupported ICS RECURRENCE-ID RANGE"
                    );
                    date_property(property, fallback)
                })
                .transpose()?;
            let sequence = prop(&event.properties, "SEQUENCE")
                .map(|p| {
                    value(p)?
                        .parse::<u32>()
                        .map_err(|_| anyhow!("Invalid ICS SEQUENCE"))
                })
                .transpose()?
                .unwrap_or(0);
            let revision = (
                sequence,
                revision_time(&event.properties, "DTSTAMP")?,
                revision_time(&event.properties, "LAST-MODIFIED")?,
            );
            let items = series.entry(uid).or_default();
            let key = recurrence.map(Moment::key);
            if items
                .get(&key)
                .is_none_or(|previous| revision >= previous.revision)
            {
                items.insert(
                    key,
                    Item {
                        event,
                        recurrence,
                        revision,
                    },
                );
            }
        }
        for (_, mut items) in series {
            let master = items.remove(&None);
            if master
                .as_ref()
                .is_some_and(|master| cancelled(&master.event.properties))
            {
                continue;
            }
            let mut master_length = None;
            let mut master_title = "(Untitled)".to_owned();
            let mut master_start = None;
            if let Some(master) = &master {
                let properties = &master.event.properties;
                let start = date_property(
                    prop(properties, "DTSTART")
                        .ok_or_else(|| anyhow!("ICS event has no DTSTART"))?,
                    fallback,
                )?;
                let length = length(properties, start, fallback, None)?;
                master_start = Some(start);
                master_length = Some(length);
                if let Some(summary) = prop(properties, "SUMMARY") {
                    master_title = text(value(summary)?);
                }
                for (key, (date, length)) in
                    recurrence_dates(properties, start, length, fallback, &window)?
                {
                    if !items.contains_key(&Some(key)) {
                        emit(
                            &mut output,
                            date,
                            length,
                            &master_title,
                            &label,
                            source,
                            &window,
                        )?;
                    }
                }
            }
            // Detached instances are processed independently: an original occurrence can
            // be outside the window while its replacement has moved into the window.
            for (_, item) in items {
                let properties = &item.event.properties;
                if cancelled(properties) {
                    continue;
                }
                ensure!(
                    prop(properties, "RRULE").is_none()
                        && prop(properties, "RDATE").is_none()
                        && prop(properties, "EXRULE").is_none()
                        && prop(properties, "EXDATE").is_none(),
                    "Recurrence properties on an ICS override are unsupported"
                );
                let original = item
                    .recurrence
                    .ok_or_else(|| anyhow!("ICS override has no recurrence ID"))?;
                if let Some(master_start) = master_start {
                    ensure!(
                        master_start.same_kind(original),
                        "ICS recurrence ID and DTSTART types do not match"
                    );
                }
                let start = prop(properties, "DTSTART")
                    .map(|p| date_property(p, fallback))
                    .transpose()?
                    .unwrap_or(original);
                let inherited = master_length
                    .filter(|_| master_start.is_none_or(|master| master.same_kind(start)));
                let length = length(properties, start, fallback, inherited)?;
                let title = prop(properties, "SUMMARY")
                    .map(|p| value(p).map(text))
                    .transpose()?
                    .unwrap_or_else(|| master_title.clone());
                emit(&mut output, start, length, &title, &label, source, &window)?;
            }
        }
    }
    ensure!(calendars > 0, "No ICS calendars found");
    output.sort_by(|left, right| {
        let key = |time: &EventTime| match time {
            EventTime::AllDay(date) => Key::Date(*date),
            EventTime::Timed(time) => Key::Time(time.timestamp()),
        };
        key(&left.start)
            .cmp(&key(&right.start))
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.calendar.cmp(&right.calendar))
            .then_with(|| key(&left.end).cmp(&key(&right.end)))
    });
    Ok(output)
}

#[cfg(test)]
mod tests;
