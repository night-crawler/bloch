use std::{hint::cold_path, sync::LazyLock};

use chrono::{DateTime, Duration, NaiveDate, TimeDelta, TimeZone as _, Utc};
use chrono_tz::{Tz, TzOffset};

use crate::Error;

const EPOCH_DATE: NaiveDate = NaiveDate::from_yo_opt(1970, 1).expect("1970 day 1 is a valid date");

pub const UTC_ALIASES: [Tz; 11] = [
    Tz::UTC,
    Tz::GMT,
    Tz::Zulu,
    Tz::Etc__UTC,
    Tz::Etc__GMT,
    Tz::Universal,
    Tz::UCT,
    Tz::Etc__Zulu,
    Tz::Etc__Universal,
    Tz::Etc__GMTPlus0,
    Tz::Etc__GMTMinus0,
];

// A reminder for future self: either a lock or an unsound transmute into TzOffset because they have
// no public anything to const construct it.
static UTC_ALIAS_OFFSETS: LazyLock<[TzOffset; UTC_ALIASES.len()]> = LazyLock::new(|| {
    UTC_ALIASES.map(|tz| tz.offset_from_utc_datetime(&DateTime::UNIX_EPOCH.naive_utc()))
});

#[inline]
pub fn utc_alias_offset(tz: Tz) -> Option<TzOffset> {
    let idx = UTC_ALIASES.iter().position(|&alias| alias == tz)?;
    Some(UTC_ALIAS_OFFSETS[idx])
}

pub fn date16(days: u16) -> NaiveDate {
    EPOCH_DATE + Duration::days(i64::from(days))
}

pub fn date32(days: i32) -> crate::Result<NaiveDate> {
    match EPOCH_DATE.checked_add_signed(Duration::days(i64::from(days))) {
        Some(date) => Ok(date),
        None => {
            cold_path();
            Err(Error::ValueOutOfRange(
                "Date32",
                "NaiveDate",
                days.to_string(),
            ))
        }
    }
}

pub fn days16(date: NaiveDate) -> crate::Result<u16> {
    match u16::try_from((date - EPOCH_DATE).num_days()) {
        Ok(days) => Ok(days),
        Err(_) => {
            cold_path();
            Err(Error::ValueOutOfRange(
                "NaiveDate",
                "Date",
                date.to_string(),
            ))
        }
    }
}

pub fn days32(date: NaiveDate) -> crate::Result<i32> {
    match i32::try_from((date - EPOCH_DATE).num_days()) {
        Ok(days) => Ok(days),
        Err(_) => {
            cold_path();
            Err(Error::ValueOutOfRange(
                "NaiveDate",
                "Date32",
                date.to_string(),
            ))
        }
    }
}

/// Whole ticks of `10^-precision` seconds in `total_nanos`, floored like ClickHouse stores them.
pub fn ticks(total_nanos: i128, precision: u8, what: &'static str) -> crate::Result<i64> {
    let Some(exp) = MAX_PRECISION.checked_sub(precision) else {
        return Err(precision_out_of_range(precision));
    };
    let divisor = 10i128.pow(u32::from(exp));
    match i64::try_from(total_nanos.div_euclid(divisor)) {
        Ok(ticks) => Ok(ticks),
        Err(_) => {
            cold_path();
            Err(Error::ValueOutOfRange(
                what,
                "Int64 ticks",
                total_nanos.to_string(),
            ))
        }
    }
}

pub fn total_nanos(delta: TimeDelta) -> i128 {
    i128::from(delta.num_seconds()) * 1_000_000_000 + i128::from(delta.subsec_nanos())
}

#[inline(always)]
pub fn datetime32(secs: u32) -> DateTime<Utc> {
    // SAFETY: every u32 timestamp is within chrono's DateTime range, and
    // zero nanoseconds is valid.
    unsafe { DateTime::<Utc>::from_timestamp(i64::from(secs), 0).unwrap_unchecked() }
}

#[inline(always)]
pub fn datetime32_tz(secs: u32, tz: Tz) -> DateTime<Tz> {
    datetime32_resolved(secs, tz, utc_alias_offset(tz))
}

#[inline(always)]
pub fn datetime32_resolved(secs: u32, tz: Tz, utc_offset: Option<TzOffset>) -> DateTime<Tz> {
    let dt_utc = datetime32(secs);
    match utc_offset {
        Some(offset) => DateTime::from_naive_utc_and_offset(dt_utc.naive_utc(), offset),
        None => dt_utc.with_timezone(&tz),
    }
}

/// Finest tick both `DateTime64` and `Time64` accept (`TIME64_MAX_SCALE`, `DataTypeDateTime64`).
const MAX_PRECISION: u8 = 9;

/// `None` when `precision > MAX_PRECISION`.
#[inline]
fn split_ticks(ticks: i64, precision: u8) -> Option<(i64, u32)> {
    const POW10: [i64; MAX_PRECISION as usize + 1] = [
        1,
        10,
        100,
        1_000,
        10_000,
        100_000,
        1_000_000,
        10_000_000,
        100_000_000,
        1_000_000_000,
    ];

    let &pow = POW10.get(usize::from(precision))?;
    let secs = ticks.div_euclid(pow);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "0 <= rem_euclid(pow) < pow, so the product is < 10^9 and fits u32"
    )]
    let nsec = (ticks.rem_euclid(pow) * POW10[usize::from(MAX_PRECISION - precision)]) as u32;
    Some((secs, nsec))
}

#[cold]
#[inline(never)]
fn precision_out_of_range(precision: u8) -> Error {
    Error::ValueOutOfRange("u8", "precision (0..=9)", precision.to_string())
}

pub fn datetime64(timestamp: i64, precision: u8) -> crate::Result<DateTime<Utc>> {
    let Some((secs, nsec)) = split_ticks(timestamp, precision) else {
        return Err(precision_out_of_range(precision));
    };
    match DateTime::<Utc>::from_timestamp(secs, nsec) {
        Some(dt) => Ok(dt),
        None => {
            cold_path();
            Err(Error::ValueOutOfRange(
                "DateTime64",
                "DateTime<Utc>",
                timestamp.to_string(),
            ))
        }
    }
}

/// `Err` when `precision > 9` or when the result does not fit a [`TimeDelta`].
pub fn time64(ticks: i64, precision: u8) -> crate::Result<TimeDelta> {
    let Some((secs, nsec)) = split_ticks(ticks, precision) else {
        return Err(precision_out_of_range(precision));
    };
    match TimeDelta::new(secs, nsec) {
        Some(td) => Ok(td),
        None => {
            cold_path();
            Err(Error::ValueOutOfRange(
                "Time64",
                "TimeDelta",
                ticks.to_string(),
            ))
        }
    }
}

pub fn datetime64_tz(timestamp: i64, precision: u8, tz: Tz) -> crate::Result<DateTime<Tz>> {
    datetime64_resolved(timestamp, precision, tz, utc_alias_offset(tz))
}

#[inline]
pub fn datetime64_resolved(
    timestamp: i64,
    precision: u8,
    tz: Tz,
    utc_offset: Option<TzOffset>,
) -> crate::Result<DateTime<Tz>> {
    let dt_utc = datetime64(timestamp, precision)?;
    Ok(match utc_offset {
        Some(offset) => DateTime::from_naive_utc_and_offset(dt_utc.naive_utc(), offset),
        None => dt_utc.with_timezone(&tz),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECS: u32 = 1_700_000_000;

    #[test]
    fn datetime32_tz_preserves_zone_identity() {
        for tz in [
            Tz::UTC,
            Tz::Zulu,
            Tz::Universal,
            Tz::UCT,
            Tz::Etc__UTC,
            Tz::GMT,
            Tz::Etc__GMT,
            Tz::Etc__GMTPlus0,
        ] {
            let actual = datetime32_tz(SECS, tz);
            assert_eq!(actual.timezone(), tz, "zone identity lost for {tz}");
        }
    }

    #[test]
    fn datetime32_tz_matches_with_timezone() {
        for tz in [Tz::GMT, Tz::Etc__GMT, Tz::Zulu] {
            let actual = datetime32_tz(SECS, tz);
            let expected = datetime32(SECS).with_timezone(&tz);
            assert_eq!(
                actual.to_string(),
                expected.to_string(),
                "fast path diverges from with_timezone for {tz}"
            );
        }
    }

    #[test]
    fn datetime32_tz_gmt_abbreviation() {
        let actual = datetime32_tz(SECS, Tz::GMT);
        assert_eq!(
            actual.format("%Z").to_string(),
            "GMT",
            "GMT column rendered with the wrong abbreviation"
        );
    }

    #[test]
    fn datetime64_scales_every_precision() {
        // 2023-11-14T22:13:20.123456789Z at each precision.
        let base = i64::from(SECS);
        let cases: [(u8, i64, u32); 10] = [
            (0, base, 0),
            (1, base * 10 + 1, 100_000_000),
            (2, base * 100 + 12, 120_000_000),
            (3, base * 1_000 + 123, 123_000_000),
            (4, base * 10_000 + 1_234, 123_400_000),
            (5, base * 100_000 + 12_345, 123_450_000),
            (6, base * 1_000_000 + 123_456, 123_456_000),
            (7, base * 10_000_000 + 1_234_567, 123_456_700),
            (8, base * 100_000_000 + 12_345_678, 123_456_780),
            (9, base * 1_000_000_000 + 123_456_789, 123_456_789),
        ];
        for (precision, ts, nanos) in cases {
            let dt = datetime64(ts, precision).unwrap_or_else(|e| panic!("p{precision}: {e}"));
            assert_eq!(dt.timestamp(), base, "seconds wrong at p{precision}");
            assert_eq!(
                dt.timestamp_subsec_nanos(),
                nanos,
                "fraction wrong at p{precision}"
            );
        }
    }

    #[test]
    fn datetime64_negative_floors_toward_neg_inf() {
        let dt = datetime64(-1_500, 3).unwrap();
        assert_eq!(
            dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "1969-12-31T23:59:58.500Z"
        );
        // Whole == 0 case from ClickHouse writeDateTimeText: -0.877 -> 23:59:59.123.
        let dt = datetime64(-877, 3).unwrap();
        assert_eq!(
            dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "1969-12-31T23:59:59.123Z"
        );
        // Exact negative second has no fraction.
        let dt = datetime64(-1_000_000, 6).unwrap();
        assert_eq!(
            dt.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            "1969-12-31T23:59:59.000000Z"
        );
    }

    #[test]
    fn datetime64_rejects_precision_above_nine() {
        assert!(matches!(
            datetime64(0, 10),
            Err(Error::ValueOutOfRange("u8", _, _))
        ));
        assert!(matches!(
            datetime64(0, u8::MAX),
            Err(Error::ValueOutOfRange("u8", _, _))
        ));
    }

    #[test]
    fn datetime64_rejects_unrepresentable_timestamp() {
        assert!(matches!(
            datetime64(i64::MAX, 0),
            Err(Error::ValueOutOfRange("DateTime64", _, _))
        ));
    }
}
