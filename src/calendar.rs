//! Optional calendar for displaying real dates alongside integer-day CPM.
//!
//! Scheduling itself stays in whole project days. This module only maps a
//! project day offset to a civil date, optionally skipping weekends. The
//! arithmetic is self-contained: no external date crate is used.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A civil (Gregorian) date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DateFields")]
pub struct Date {
    /// Year, for example 2026.
    pub year: i32,
    /// Month, from 1 (January) to 12 (December).
    pub month: u32,
    /// Day of month, from 1 to 31.
    pub day: u32,
}

#[derive(Deserialize)]
struct DateFields {
    year: i32,
    month: u32,
    day: u32,
}

impl TryFrom<DateFields> for Date {
    type Error = String;

    fn try_from(fields: DateFields) -> Result<Self, Self::Error> {
        Self::new(fields.year, fields.month, fields.day).ok_or_else(|| {
            format!(
                "Invalid date {}-{}-{}",
                fields.year, fields.month, fields.day
            )
        })
    }
}

const MIN_SERIAL: i64 = days_from_civil(i32::MIN, 1, 1);
const MAX_SERIAL: i64 = days_from_civil(i32::MAX, 12, 31);

impl Date {
    /// Build a date, returning `None` when the components are out of range.
    #[must_use]
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        let date = Self { year, month, day };
        (1..=12).contains(&month).then_some(date)?;
        let days_in_month = days_in_month(year, month);
        (1..=days_in_month).contains(&day).then_some(date)
    }

    /// Current UTC civil date.
    #[must_use]
    pub fn today() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64);
        Self::from_serial((seconds / 86_400) as i64)
    }

    /// Days since 1970-01-01 (negative for earlier dates).
    #[must_use]
    pub fn serial(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// Build a date from a day serial, clamping out-of-range values.
    #[must_use]
    pub fn from_serial(serial: i64) -> Self {
        Self::try_from_serial(serial).unwrap_or({
            if serial < MIN_SERIAL {
                Self {
                    year: i32::MIN,
                    month: 1,
                    day: 1,
                }
            } else {
                Self {
                    year: i32::MAX,
                    month: 12,
                    day: 31,
                }
            }
        })
    }

    /// Build a date when the serial is inside the representable civil range.
    #[must_use]
    pub fn try_from_serial(serial: i64) -> Option<Self> {
        if !(MIN_SERIAL..=MAX_SERIAL).contains(&serial) {
            return None;
        }
        let (year, month, day) = civil_from_days(serial);
        Some(Self { year, month, day })
    }
}

impl std::fmt::Display for Date {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

/// Calendar settings attached to a project document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    /// Real date of project day zero. `None` keeps the integer-day ruler.
    #[serde(default)]
    pub start_date: Option<Date>,
    /// When true, day offsets skip Saturdays and Sundays.
    #[serde(default)]
    pub skip_weekends: bool,
}

impl Calendar {
    /// Date of a project day offset, or `None` when no start date is set.
    ///
    /// Extremely large offsets also return `None` instead of wrapping or
    /// looping once per calendar day.
    #[must_use]
    pub fn date_for_day(&self, day: i64) -> Option<Date> {
        let start = self.start_date?;
        let serial = if self.skip_weekends {
            shift_workdays(start.serial(), day)?
        } else {
            start.serial().checked_add(day)?
        };
        Date::try_from_serial(serial)
    }

    /// Short ruler label for a project day offset.
    #[must_use]
    pub fn label_for_day(&self, day: i64) -> String {
        match self.date_for_day(day) {
            Some(date) => date.to_string(),
            None => format!("D{day}"),
        }
    }
}

/// Shift `start` by `count` working days (negative counts move backwards).
fn shift_workdays(start: i64, count: i64) -> Option<i64> {
    if !(MIN_SERIAL..=MAX_SERIAL).contains(&start) {
        return None;
    }
    if count == 0 {
        return Some(start);
    }

    let direction = if count < 0 { -1 } else { 1 };
    let mut remaining = count.unsigned_abs();
    let mut serial = start;

    // A weekend start needs at most two calendar steps before the first
    // counted working day, in either direction.
    while remaining > 0 && is_weekend(serial) {
        serial = serial.checked_add(direction)?;
        if !is_weekend(serial) {
            remaining -= 1;
        }
    }

    // Whole weeks preserve the weekday and avoid iterating over huge plans.
    let weeks = i128::from(remaining / 5);
    let shifted = i128::from(serial) + i128::from(direction) * weeks * 7;
    serial = i64::try_from(shifted).ok()?;
    if !(MIN_SERIAL..=MAX_SERIAL).contains(&serial) {
        return None;
    }

    let mut remainder = (remaining % 5) as u8;
    while remainder > 0 {
        serial = serial.checked_add(direction)?;
        if !is_weekend(serial) {
            remainder -= 1;
        }
    }
    (MIN_SERIAL..=MAX_SERIAL)
        .contains(&serial)
        .then_some(serial)
}

/// Monday = 0, Sunday = 6.
#[must_use]
pub fn weekday(serial: i64) -> u32 {
    ((serial.rem_euclid(7) + 3) % 7) as u32
}

/// Whether a day serial falls on a Saturday or Sunday.
#[must_use]
pub fn is_weekend(serial: i64) -> bool {
    weekday(serial) >= 5
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to a civil date (Howard Hinnant's algorithm).
const fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = month as i64 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

/// Civil date from a day serial (Howard Hinnant's algorithm).
fn civil_from_days(serial: i64) -> (i32, u32, u32) {
    let shifted = serial + 719468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146096
    } / 146097;
    let day_of_era = shifted - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        let date = Date::new(2026, 11, 22).expect("valid date");
        assert_eq!(Date::from_serial(date.serial()), date);
        assert_eq!(date.to_string(), "2026-11-22");
        assert_eq!(Date::new(2026, 2, 30), None);
    }

    #[test]
    fn known_weekdays_and_weekend() {
        // 1970-01-01 was a Thursday (Monday = 0 -> 3).
        assert_eq!(weekday(0), 3);
        assert!(is_weekend(Date::new(2026, 11, 21).expect("date").serial()));
        assert!(!is_weekend(Date::new(2026, 11, 23).expect("date").serial()));
        assert!(weekday(i64::MAX) < 7);
        assert!(weekday(i64::MIN) < 7);
    }

    #[test]
    fn weekend_skipping_moves_friday_to_monday() {
        let calendar = Calendar {
            start_date: Date::new(2026, 11, 20), // Friday
            skip_weekends: true,
        };
        assert_eq!(
            calendar.date_for_day(0).expect("day 0").to_string(),
            "2026-11-20"
        );
        assert_eq!(
            calendar.date_for_day(1).expect("day 1").to_string(),
            "2026-11-23"
        );
        assert_eq!(
            calendar.date_for_day(3).expect("day 3").to_string(),
            "2026-11-25"
        );
        assert_eq!(
            calendar.date_for_day(-1).expect("previous workday"),
            Date::new(2026, 11, 19).expect("date")
        );
    }

    #[test]
    fn no_start_date_keeps_integer_day_labels() {
        let calendar = Calendar::default();
        assert_eq!(calendar.label_for_day(4), "D4");
        assert_eq!(calendar.date_for_day(4), None);
    }

    #[test]
    fn extreme_offsets_are_rejected_without_wrapping_or_long_loops() {
        let calendar = Calendar {
            start_date: Date::new(2026, 11, 23),
            skip_weekends: true,
        };
        assert_eq!(calendar.date_for_day(i64::MIN), None);
        assert_eq!(calendar.date_for_day(i64::MAX), None);
        assert_eq!(Date::try_from_serial(i64::MIN), None);
        assert_eq!(Date::try_from_serial(i64::MAX), None);
        assert_eq!(Date::from_serial(i64::MIN).year, i32::MIN);
        assert_eq!(Date::from_serial(i64::MAX).year, i32::MAX);
    }

    #[test]
    fn today_is_a_valid_round_tripping_date() {
        let today = Date::today();
        assert!(Date::new(today.year, today.month, today.day).is_some());
        assert_eq!(Date::from_serial(today.serial()), today);
    }

    #[test]
    fn invalid_persisted_dates_are_rejected() {
        let valid = Date::new(2026, 11, 23).expect("date");
        let json = serde_json::to_string(&valid).expect("serialize date");
        assert_eq!(
            serde_json::from_str::<Date>(&json).expect("deserialize date"),
            valid
        );
        assert!(serde_json::from_str::<Date>(r#"{"year":2026,"month":2,"day":30}"#).is_err());
        assert!(serde_json::from_str::<Date>(r#"{"year":2026,"month":13,"day":1}"#).is_err());
    }
}
