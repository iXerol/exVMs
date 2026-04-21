use chrono::format::strftime::StrftimeItems;
use chrono::format::{Fixed, Item, Numeric};
use chrono::{DateTime, FixedOffset};

use crate::{ExvmsError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilenameTemplate {
    segments: Vec<Segment>,
    contains_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Id,
    Title,
    Date(Vec<Item<'static>>),
    Time(Vec<Item<'static>>),
    DateTime(Vec<Item<'static>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateTimeFieldKind {
    Date,
    Time,
    DateTime,
}

impl FilenameTemplate {
    pub fn default_template() -> Self {
        Self::parse("{title}").expect("default filename template is valid")
    }

    pub fn parse(input: &str) -> Result<Self> {
        if input.trim().is_empty() {
            return Err(ExvmsError::InvalidFilenameFormat(
                "template cannot be empty".to_string(),
            ));
        }

        let mut segments = Vec::new();
        let mut last = 0;
        let mut contains_id = false;

        let mut chars = input.char_indices().peekable();
        while let Some((idx, ch)) = chars.next() {
            match ch {
                '{' => {
                    if last < idx {
                        segments.push(Segment::Literal(validate_literal(&input[last..idx])?));
                    }

                    let mut end = None;
                    for (close_idx, close_ch) in chars.by_ref() {
                        if close_ch == '}' {
                            end = Some(close_idx);
                            break;
                        }
                    }
                    let end = end.ok_or_else(|| {
                        ExvmsError::InvalidFilenameFormat("unclosed `{` in template".to_string())
                    })?;
                    let content = &input[idx + ch.len_utf8()..end];
                    let segment = parse_field(content)?;
                    contains_id |= matches!(segment, Segment::Id);
                    segments.push(segment);
                    last = end + '}'.len_utf8();
                }
                '}' => {
                    return Err(ExvmsError::InvalidFilenameFormat(
                        "unexpected `}` in template".to_string(),
                    ));
                }
                _ => {}
            }
        }

        if last < input.len() {
            segments.push(Segment::Literal(validate_literal(&input[last..])?));
        }

        if segments.is_empty() {
            return Err(ExvmsError::InvalidFilenameFormat(
                "template cannot be empty".to_string(),
            ));
        }

        Ok(Self {
            segments,
            contains_id,
        })
    }

    pub fn contains_id(&self) -> bool {
        self.contains_id
    }

    pub fn render_stem(&self, id: i64, title: &str, recorded_at: DateTime<FixedOffset>) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(value) => out.push_str(value),
                Segment::Id => out.push_str(&id.to_string()),
                Segment::Title => out.push_str(title),
                Segment::Date(items) | Segment::Time(items) | Segment::DateTime(items) => {
                    out.push_str(
                        &recorded_at
                            .format_with_items(items.iter().cloned())
                            .to_string(),
                    );
                }
            }
        }
        out
    }
}

fn validate_literal(literal: &str) -> Result<String> {
    if literal.contains(['/', '\\']) {
        return Err(ExvmsError::InvalidFilenameFormat(
            "template cannot contain path separators".to_string(),
        ));
    }
    Ok(literal.to_string())
}

fn parse_field(content: &str) -> Result<Segment> {
    if content.is_empty() {
        return Err(ExvmsError::InvalidFilenameFormat(
            "template field cannot be empty".to_string(),
        ));
    }

    match content {
        "id" => Ok(Segment::Id),
        "title" => Ok(Segment::Title),
        _ => parse_datetime_field(content),
    }
}

fn parse_datetime_field(content: &str) -> Result<Segment> {
    let (kind, format) = if let Some(format) = content.strip_prefix("date:") {
        (DateTimeFieldKind::Date, format)
    } else if let Some(format) = content.strip_prefix("time:") {
        (DateTimeFieldKind::Time, format)
    } else if let Some(format) = content.strip_prefix("datetime:") {
        (DateTimeFieldKind::DateTime, format)
    } else {
        return Err(ExvmsError::InvalidFilenameFormat(format!(
            "unknown template field `{content}`"
        )));
    };

    if format.is_empty() {
        return Err(ExvmsError::InvalidFilenameFormat(format!(
            "`{content}` is missing a format string"
        )));
    }
    if format.contains(['/', '\\']) {
        return Err(ExvmsError::InvalidFilenameFormat(format!(
            "`{content}` cannot contain path separators"
        )));
    }

    let items = StrftimeItems::new(format).parse_to_owned().map_err(|_| {
        ExvmsError::InvalidFilenameFormat(format!("invalid strftime format in `{content}`"))
    })?;
    validate_datetime_items(kind, &items).map_err(|message| {
        ExvmsError::InvalidFilenameFormat(format!("invalid `{content}` format: {message}"))
    })?;

    Ok(match kind {
        DateTimeFieldKind::Date => Segment::Date(items),
        DateTimeFieldKind::Time => Segment::Time(items),
        DateTimeFieldKind::DateTime => Segment::DateTime(items),
    })
}

fn validate_datetime_items(
    kind: DateTimeFieldKind,
    items: &[Item<'static>],
) -> std::result::Result<(), String> {
    for item in items {
        let is_allowed = match item {
            Item::Literal(_) | Item::OwnedLiteral(_) | Item::Space(_) | Item::OwnedSpace(_) => true,
            Item::Numeric(numeric, _) => kind.allows_numeric(numeric.clone()),
            Item::Fixed(fixed) => kind.allows_fixed(fixed.clone()),
            Item::Error => false,
        };
        if !is_allowed {
            return Err(format!("directive `{item:?}` is not allowed for {kind}"));
        }
    }
    Ok(())
}

impl DateTimeFieldKind {
    fn allows_numeric(self, numeric: Numeric) -> bool {
        use DateTimeFieldKind::{Date, DateTime, Time};
        match self {
            Date => matches!(
                numeric,
                Numeric::Year
                    | Numeric::YearDiv100
                    | Numeric::YearMod100
                    | Numeric::IsoYear
                    | Numeric::IsoYearDiv100
                    | Numeric::IsoYearMod100
                    | Numeric::Quarter
                    | Numeric::Month
                    | Numeric::Day
                    | Numeric::WeekFromSun
                    | Numeric::WeekFromMon
                    | Numeric::IsoWeek
                    | Numeric::NumDaysFromSun
                    | Numeric::WeekdayFromMon
                    | Numeric::Ordinal
            ),
            Time => matches!(
                numeric,
                Numeric::Hour
                    | Numeric::Hour12
                    | Numeric::Minute
                    | Numeric::Second
                    | Numeric::Nanosecond
            ),
            DateTime => true,
        }
    }

    fn allows_fixed(self, fixed: Fixed) -> bool {
        use DateTimeFieldKind::{Date, DateTime, Time};
        match self {
            Date => matches!(
                fixed,
                Fixed::ShortMonthName
                    | Fixed::LongMonthName
                    | Fixed::ShortWeekdayName
                    | Fixed::LongWeekdayName
            ),
            Time => matches!(
                fixed,
                Fixed::LowerAmPm
                    | Fixed::UpperAmPm
                    | Fixed::Nanosecond
                    | Fixed::Nanosecond3
                    | Fixed::Nanosecond6
                    | Fixed::Nanosecond9
                    | Fixed::TimezoneName
                    | Fixed::TimezoneOffsetColon
                    | Fixed::TimezoneOffsetDoubleColon
                    | Fixed::TimezoneOffsetTripleColon
                    | Fixed::TimezoneOffsetColonZ
                    | Fixed::TimezoneOffset
                    | Fixed::TimezoneOffsetZ
            ),
            DateTime => true,
        }
    }
}

impl std::fmt::Display for DateTimeFieldKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Date => f.write_str("date"),
            Self::Time => f.write_str("time"),
            Self::DateTime => f.write_str("datetime"),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone, Timelike};

    use super::*;

    fn sample_datetime() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(9 * 3600)
            .unwrap()
            .with_ymd_and_hms(2023, 7, 11, 0, 34, 59)
            .single()
            .unwrap()
            .with_nanosecond(26_490_000)
            .unwrap()
    }

    #[test]
    fn default_template_matches_title_only_stem() {
        let template = FilenameTemplate::default_template();
        assert_eq!(
            template.render_stem(7, "Voice Memo", sample_datetime()),
            "Voice Memo"
        );
    }

    #[test]
    fn parses_and_renders_id_title_and_datetime_fields() {
        let template =
            FilenameTemplate::parse("{id} {date:%Y-%m-%d} {time:%H-%M-%S} {title}").unwrap();
        assert!(template.contains_id());
        assert_eq!(
            template.render_stem(7, "Voice Memo", sample_datetime()),
            "7 2023-07-11 00-34-59 Voice Memo"
        );
    }

    #[test]
    fn renders_datetime_with_fractional_seconds_and_offset() {
        let template = FilenameTemplate::parse("{datetime:%Y-%m-%dT%H-%M-%S%.3f%z}").unwrap();
        assert_eq!(
            template.render_stem(7, "Voice Memo", sample_datetime()),
            "2023-07-11T00-34-59.026+0900"
        );
    }

    #[test]
    fn rejects_unknown_field() {
        let err = FilenameTemplate::parse("{unknown}").unwrap_err();
        assert!(matches!(err, ExvmsError::InvalidFilenameFormat(_)));
    }

    #[test]
    fn rejects_unclosed_field() {
        let err = FilenameTemplate::parse("{title").unwrap_err();
        assert!(matches!(err, ExvmsError::InvalidFilenameFormat(_)));
    }

    #[test]
    fn rejects_empty_template() {
        let err = FilenameTemplate::parse("  ").unwrap_err();
        assert!(matches!(err, ExvmsError::InvalidFilenameFormat(_)));
    }

    #[test]
    fn rejects_date_field_with_time_directive() {
        let err = FilenameTemplate::parse("{date:%H:%M}").unwrap_err();
        assert!(matches!(err, ExvmsError::InvalidFilenameFormat(_)));
    }

    #[test]
    fn rejects_time_field_with_date_directive() {
        let err = FilenameTemplate::parse("{time:%Y-%m-%d}").unwrap_err();
        assert!(matches!(err, ExvmsError::InvalidFilenameFormat(_)));
    }

    #[test]
    fn rejects_invalid_strftime_directive() {
        let err = FilenameTemplate::parse("{datetime:%Y-%Q}").unwrap_err();
        assert!(matches!(err, ExvmsError::InvalidFilenameFormat(_)));
    }

    #[test]
    fn rejects_path_separators_in_template_literal() {
        let err = FilenameTemplate::parse("foo/bar {title}").unwrap_err();
        assert!(matches!(err, ExvmsError::InvalidFilenameFormat(_)));
    }
}
