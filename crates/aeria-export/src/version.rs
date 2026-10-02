//! Pack release versions: `YYYY.MM.DD.NNNN`, the UTC date of the release and
//! its number within that day. See `docs/formats/pack-v1.md`.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use crate::error::ExportError;

/// The highest release number of one day.
const MAX_BUILD: u16 = 9999;

/// A calendar date (UTC) of a release.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReleaseDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl ReleaseDate {
    /// The UTC date `seconds` after the Unix epoch.
    #[must_use]
    pub fn from_unix_seconds(seconds: u64) -> Self {
        // Howard Hinnant's civil_from_days, for days since 1970-01-01.
        let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX) + 719_468;
        let era = days.div_euclid(146_097);
        let day_of_era = days.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_index = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_index + 2) / 5 + 1;
        let month = if month_index < 10 {
            month_index + 3
        } else {
            month_index - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        // The algorithm gives month 1..=12 and day 1..=31.
        Self {
            year: u16::try_from(year).unwrap_or(u16::MAX),
            month: u8::try_from(month).unwrap_or(1),
            day: u8::try_from(day).unwrap_or(1),
        }
    }

    fn is_valid(self) -> bool {
        let leap = self.year.is_multiple_of(4)
            && (!self.year.is_multiple_of(100) || self.year.is_multiple_of(400));
        let days = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return false,
        };
        self.year >= 2000 && (1..=days).contains(&self.day)
    }
}

/// A release version: its date and its number within the day, from 1.
/// Versions compare by date, then number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackVersion {
    pub date: ReleaseDate,
    pub build: u16,
}

impl PackVersion {
    /// The version after `latest` for a release made on `today`: the first of
    /// today, or the next number of `latest`'s day when that day is today or
    /// later (a clock behind another maintainer's never goes back).
    ///
    /// # Errors
    /// Returns [`ExportError::Manifest`] when the day already has 9999
    /// releases.
    pub fn next(today: ReleaseDate, latest: Option<Self>) -> Result<Self, ExportError> {
        match latest {
            Some(latest) if latest.date >= today => {
                if latest.build >= MAX_BUILD {
                    return Err(ExportError::Manifest(format!(
                        "{} already has {MAX_BUILD} releases",
                        latest.date_text()
                    )));
                }
                Ok(Self {
                    date: latest.date,
                    build: latest.build + 1,
                })
            }
            _ => Ok(Self {
                date: today,
                build: 1,
            }),
        }
    }

    fn date_text(self) -> String {
        format!(
            "{:04}.{:02}.{:02}",
            self.date.year, self.date.month, self.date.day
        )
    }
}

impl Ord for PackVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.date, self.build).cmp(&(other.date, other.build))
    }
}

impl PartialOrd for PackVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for PackVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{:04}", self.date_text(), self.build)
    }
}

impl FromStr for PackVersion {
    type Err = ExportError;

    /// Reads exactly `YYYY.MM.DD.NNNN` with a valid date and `NNNN` ≥ 1.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid =
            || ExportError::Manifest(format!("{text:?} is not a YYYY.MM.DD.NNNN version"));
        let parts: Vec<&str> = text.split('.').collect();
        let [year, month, day, build] = parts.as_slice() else {
            return Err(invalid());
        };
        let number = |part: &str, width: usize| {
            (part.len() == width && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u16>().ok())
                .flatten()
                .ok_or_else(invalid)
        };
        let date = ReleaseDate {
            year: number(year, 4)?,
            month: u8::try_from(number(month, 2)?).map_err(|_| invalid())?,
            day: u8::try_from(number(day, 2)?).map_err(|_| invalid())?,
        };
        let build = number(build, 4)?;
        if !date.is_valid() || build == 0 {
            return Err(invalid());
        }
        Ok(Self { date, build })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: u16, month: u8, day: u8) -> ReleaseDate {
        ReleaseDate { year, month, day }
    }

    #[test]
    fn versions_read_and_print_with_fixed_widths() {
        let version: PackVersion = "2026.10.01.0001".parse().expect("version");
        assert_eq!(version.date, date(2026, 10, 1));
        assert_eq!(version.build, 1);
        assert_eq!(version.to_string(), "2026.10.01.0001");
        for text in [
            "2026.10.1.0001",
            "2026.10.01.1",
            "2026.10.01.0000",
            "2026.02.30.0001",
            "2026.13.01.0001",
            "2026.10.01",
            "2026.10.01.0001.1",
            "+026.10.01.0001",
            "",
        ] {
            assert!(text.parse::<PackVersion>().is_err(), "{text}");
        }
    }

    #[test]
    fn versions_order_by_date_then_number() {
        let parse = |text: &str| text.parse::<PackVersion>().expect("version");
        assert!(parse("2026.10.01.0002") > parse("2026.10.01.0001"));
        assert!(parse("2026.10.02.0001") > parse("2026.10.01.9999"));
        assert!(parse("2027.01.01.0001") > parse("2026.12.31.0005"));
    }

    #[test]
    fn the_next_version_never_goes_back() {
        let today = date(2026, 10, 1);
        let parse = |text: &str| text.parse::<PackVersion>().expect("version");
        let next = |latest: Option<&str>| {
            PackVersion::next(today, latest.map(parse))
                .expect("next")
                .to_string()
        };
        assert_eq!(next(None), "2026.10.01.0001");
        assert_eq!(next(Some("2026.09.30.0004")), "2026.10.01.0001");
        assert_eq!(next(Some("2026.10.01.0001")), "2026.10.01.0002");
        assert_eq!(next(Some("2026.10.02.0003")), "2026.10.02.0004");
        assert!(PackVersion::next(today, Some(parse("2026.10.01.9999"))).is_err());
    }

    #[test]
    fn unix_time_gives_the_utc_date() {
        assert_eq!(ReleaseDate::from_unix_seconds(0), date(1970, 1, 1));
        assert_eq!(
            ReleaseDate::from_unix_seconds(1_790_899_199),
            date(2026, 10, 1)
        );
        assert_eq!(
            ReleaseDate::from_unix_seconds(951_782_400),
            date(2000, 2, 29)
        );
    }
}
