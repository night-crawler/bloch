use std::hint::cold_path;

use chrono::TimeDelta;

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Nanosecond,
    Microsecond,
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nanosecond => "Nanosecond",
            Self::Microsecond => "Microsecond",
            Self::Millisecond => "Millisecond",
            Self::Second => "Second",
            Self::Minute => "Minute",
            Self::Hour => "Hour",
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Quarter => "Quarter",
            Self::Year => "Year",
        }
    }

    pub const fn is_fixed_length(self) -> bool {
        !matches!(self, Self::Month | Self::Quarter | Self::Year)
    }

    pub const fn unit_nanos(self) -> Option<i64> {
        match self {
            Self::Nanosecond => Some(1),
            Self::Microsecond => Some(1_000),
            Self::Millisecond => Some(1_000_000),
            Self::Second => Some(1_000_000_000),
            Self::Minute => Some(60_000_000_000),
            Self::Hour => Some(3_600_000_000_000),
            Self::Day => Some(86_400_000_000_000),
            Self::Week => Some(604_800_000_000_000),
            Self::Month | Self::Quarter | Self::Year => None,
        }
    }

    pub fn to_time_delta(self, count: i64) -> crate::Result<TimeDelta> {
        let delta = match self {
            Self::Nanosecond => Some(TimeDelta::nanoseconds(count)),
            Self::Microsecond => Some(TimeDelta::microseconds(count)),
            Self::Millisecond => TimeDelta::try_milliseconds(count),
            Self::Second => TimeDelta::try_seconds(count),
            Self::Minute => TimeDelta::try_minutes(count),
            Self::Hour => TimeDelta::try_hours(count),
            Self::Day => TimeDelta::try_days(count),
            Self::Week => TimeDelta::try_weeks(count),
            Self::Month | Self::Quarter | Self::Year => {
                cold_path();
                return Err(Error::MismatchedType(self.as_str(), "TimeDelta"));
            }
        };
        match delta {
            Some(delta) => Ok(delta),
            None => {
                cold_path();
                Err(Error::Overflow(format!("{count} {}", self.as_str())))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Interval {
    pub kind: Kind,
    pub count: i64,
}

impl TryFrom<Interval> for TimeDelta {
    type Error = Error;

    #[inline]
    fn try_from(interval: Interval) -> crate::Result<Self> {
        interval.kind.to_time_delta(interval.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_kinds_are_rejected() {
        for kind in [Kind::Month, Kind::Quarter, Kind::Year] {
            assert!(matches!(
                kind.to_time_delta(1),
                Err(Error::MismatchedType(name, "TimeDelta")) if name == kind.as_str()
            ));
        }
    }

    #[test]
    fn overflow_is_reported() {
        assert!(matches!(
            Kind::Week.to_time_delta(i64::MAX),
            Err(Error::Overflow(_))
        ));
    }
}
