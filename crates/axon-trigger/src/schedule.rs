//! What-fires-when descriptors.

use serde::{Deserialize, Serialize};

use crate::cron::CronExpr;

/// A description of when a trigger should fire. Resolution is **nanoseconds
/// since Unix epoch** throughout to match `axon-runtime`'s `Duration` and
/// `time_now` built-ins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// Fire every `period_ns` nanoseconds.
    Every { period_ns: i64 },
    /// Fire once at `when_ns`. Won't fire again afterwards.
    At { when_ns: i64 },
    /// Fire whenever the cron expression matches the current minute.
    Cron(CronExpr),
}

impl Schedule {
    pub fn every_seconds(s: i64) -> Self {
        Self::Every {
            period_ns: s.saturating_mul(1_000_000_000),
        }
    }
    pub fn every_minutes(m: i64) -> Self {
        Self::Every {
            period_ns: m.saturating_mul(60 * 1_000_000_000),
        }
    }
    pub fn cron(expr: &str) -> Result<Self, String> {
        Ok(Self::Cron(CronExpr::parse(expr)?))
    }

    /// Is this schedule due *now*? Returns the fire time (≤ `now_ns`) the
    /// scheduler should record, or `None` if the trigger shouldn't fire
    /// this tick.
    ///
    /// Semantics (v0):
    ///   * `Every`: first fire = `now_ns`. After that, fire when
    ///     `last + period ≤ now`, returning the deadline as the fire
    ///     time (so persisted `last_fired_ns` stays on the period grid).
    ///   * `At`: fires exactly once at `when_ns`.
    ///   * `Cron`: fires at the current minute boundary if the expression
    ///     matches and we haven't already fired this minute (`last`
    ///     equals the current minute).
    ///
    /// Catch-up policy is **coalescing**: a process that was offline for
    /// hours fires once on resume, not once per missed window. The fired
    /// time is the latest matching deadline so the trigger record stays
    /// on its period grid.
    pub fn due_at(&self, last_fired_ns: Option<i64>, now_ns: i64) -> Option<i64> {
        match self {
            Schedule::Every { period_ns } => match last_fired_ns {
                None => Some(now_ns),
                Some(l) => {
                    let next = l.saturating_add(*period_ns);
                    if next <= now_ns {
                        // Coalesce missed periods — record the latest
                        // deadline that's ≤ now so subsequent ticks don't
                        // re-fire for windows we already handled.
                        let elapsed = now_ns.saturating_sub(l);
                        let periods = elapsed / period_ns;
                        Some(l + periods * period_ns)
                    } else {
                        None
                    }
                }
            },
            Schedule::At { when_ns } => {
                if last_fired_ns.is_some() {
                    return None;
                }
                if *when_ns <= now_ns {
                    Some(*when_ns)
                } else {
                    None
                }
            }
            Schedule::Cron(expr) => {
                let one_min: i64 = 60 * 1_000_000_000;
                let this_minute = (now_ns / one_min) * one_min;
                if last_fired_ns == Some(this_minute) {
                    return None;
                }
                let p = decompose_unix_ns(this_minute);
                if expr.matches(p.minute, p.hour, p.day, p.month, p.dow) {
                    Some(this_minute)
                } else {
                    None
                }
            }
        }
    }
}

/// Decomposed UTC calendar components, derived from a Unix-epoch ns value
/// without pulling in `chrono`. Accurate for 1970-01-01 .. 2262-04-11.
#[derive(Debug)]
pub(crate) struct CalendarParts {
    pub minute: u8,
    pub hour: u8,
    pub day: u8,
    pub month: u8,
    pub _year: i32,
    pub dow: u8, // 0 = Sunday
}

pub(crate) fn decompose_unix_ns(ns: i64) -> CalendarParts {
    let total_seconds = ns / 1_000_000_000;
    let days_from_epoch = total_seconds.div_euclid(86_400);
    let secs_of_day = total_seconds.rem_euclid(86_400);
    let minute = ((secs_of_day / 60) % 60) as u8;
    let hour = (secs_of_day / 3600) as u8;
    // 1970-01-01 was a Thursday → day-of-week index 4 (0 = Sunday).
    let dow = ((days_from_epoch + 4).rem_euclid(7)) as u8;
    let (year, month, day) = civil_from_days(days_from_epoch);
    CalendarParts {
        minute,
        hour,
        day,
        month,
        _year: year,
        dow,
    }
}

/// Howard Hinnant's date algorithm — `days` is days from 1970-01-01.
/// Returns `(year, month, day)` with month in 1..=12.
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y_final = if m <= 2 { y + 1 } else { y };
    (y_final as i32, m as u8, d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> i64 {
        // Compute via inverse of the algorithm: simple loop from epoch.
        // For tests we just need *some* known epoch; use deterministic math.
        // Easier: derive days_from_epoch by counting forward.
        let mut days: i64 = 0;
        let mut y = 1970i32;
        while y < year {
            days += if is_leap(y) { 366 } else { 365 };
            y += 1;
        }
        let month_days = [31u8, 28 + is_leap(year) as u8, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for m in 1..month {
            days += month_days[(m - 1) as usize] as i64;
        }
        days += (day - 1) as i64;
        let total_secs = days * 86_400 + (hour as i64) * 3600 + (minute as i64) * 60;
        total_secs * 1_000_000_000
    }

    fn is_leap(y: i32) -> bool {
        (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
    }

    #[test]
    fn every_first_fire_is_immediate_then_period_grid() {
        let s = Schedule::every_seconds(60);
        // No prior fire — due now.
        assert_eq!(s.due_at(None, 30_000_000_000), Some(30_000_000_000));
        // Fired at 0; 30s in is too soon.
        assert_eq!(s.due_at(Some(0), 30_000_000_000), None);
        // 70s in: due at 60s (latest matching deadline).
        assert_eq!(s.due_at(Some(0), 70_000_000_000), Some(60_000_000_000));
    }

    #[test]
    fn every_coalesces_missed_periods() {
        let s = Schedule::every_seconds(60);
        // Last fire at 0, now is 5 minutes later — fire once at 5m,
        // not 5 times.
        let fire = s.due_at(Some(0), 300_000_000_000).unwrap();
        assert_eq!(fire, 300_000_000_000);
    }

    #[test]
    fn at_fires_once_then_never_again() {
        let s = Schedule::At { when_ns: 100_000_000_000 };
        assert_eq!(s.due_at(None, 50_000_000_000), None);
        assert_eq!(s.due_at(None, 200_000_000_000), Some(100_000_000_000));
        assert_eq!(s.due_at(Some(100_000_000_000), 300_000_000_000), None);
    }

    #[test]
    fn cron_fires_on_matching_minute() {
        // 2024-05-15 was a Wednesday.
        let s = Schedule::cron("0 9 * * 3").unwrap();
        let wed_9 = ns(2024, 5, 15, 9, 0);
        assert_eq!(s.due_at(None, wed_9), Some(wed_9));
        // Already fired this minute → no refire.
        assert_eq!(s.due_at(Some(wed_9), wed_9 + 30_000_000_000), None);
        // Same day at 10:00 — no match (hour=10).
        let wed_10 = ns(2024, 5, 15, 10, 0);
        assert_eq!(s.due_at(None, wed_10), None);
    }

    #[test]
    fn calendar_decomposition_handles_known_dates() {
        // Unix epoch:
        let p = decompose_unix_ns(0);
        assert_eq!((p._year, p.month, p.day, p.dow), (1970, 1, 1, 4));
        // 2026-05-19 (today) 00:00 UTC:
        let t = ns(2026, 5, 19, 0, 0);
        let p = decompose_unix_ns(t);
        assert_eq!((p._year, p.month, p.day), (2026, 5, 19));
    }
}
