//! Minimal but correct cron-expression parser & matcher.
//!
//! Format: `minute hour day-of-month month day-of-week`, all required.
//! Each field accepts: `*`, a literal `N`, a `low-high` range, a comma list
//! `a,b,c`, and a step `*/N` or `low-high/N`.
//!
//! Reference field ranges:
//!   * minute:        0..=59
//!   * hour:          0..=23
//!   * day-of-month:  1..=31
//!   * month:         1..=12
//!   * day-of-week:   0..=6   (0 = Sunday)
//!
//! Matching semantics match POSIX cron: when BOTH `day-of-month` and
//! `day-of-week` are restricted (neither is `*`), a fire occurs if EITHER
//! matches — the historical, surprising-but-spec rule.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronExpr {
    pub source: String,
    pub minutes: Vec<u8>,
    pub hours: Vec<u8>,
    pub days_of_month: Vec<u8>,
    pub months: Vec<u8>,
    pub days_of_week: Vec<u8>,
    pub dom_restricted: bool,
    pub dow_restricted: bool,
}

impl CronExpr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let fields: Vec<&str> = s.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "cron: expected 5 fields, got {}: `{s}`",
                fields.len()
            ));
        }
        Ok(Self {
            source: s.to_string(),
            minutes: parse_field(fields[0], 0, 59)?,
            hours: parse_field(fields[1], 0, 23)?,
            days_of_month: parse_field(fields[2], 1, 31)?,
            months: parse_field(fields[3], 1, 12)?,
            days_of_week: parse_field(fields[4], 0, 6)?,
            dom_restricted: fields[2] != "*",
            dow_restricted: fields[4] != "*",
        })
    }

    /// Does this expression fire at the given calendar instant?
    pub fn matches(
        &self,
        minute: u8,
        hour: u8,
        day_of_month: u8,
        month: u8,
        day_of_week: u8,
    ) -> bool {
        if !self.minutes.contains(&minute) {
            return false;
        }
        if !self.hours.contains(&hour) {
            return false;
        }
        if !self.months.contains(&month) {
            return false;
        }
        let dom_ok = self.days_of_month.contains(&day_of_month);
        let dow_ok = self.days_of_week.contains(&day_of_week);
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom_ok || dow_ok, // POSIX OR rule
            (true, false) => dom_ok,
            (false, true) => dow_ok,
            (false, false) => true,
        }
    }
}

fn parse_field(field: &str, lo: u8, hi: u8) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    for piece in field.split(',') {
        let (range_part, step) = match piece.split_once('/') {
            Some((r, s)) => {
                let step: u8 = s
                    .parse()
                    .map_err(|_| format!("cron: bad step `{s}` in `{field}`"))?;
                if step == 0 {
                    return Err(format!("cron: step cannot be zero in `{field}`"));
                }
                (r, step)
            }
            None => (piece, 1u8),
        };
        let (low, high) = if range_part == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let a: u8 = a
                .parse()
                .map_err(|_| format!("cron: bad range low `{a}` in `{field}`"))?;
            let b: u8 = b
                .parse()
                .map_err(|_| format!("cron: bad range high `{b}` in `{field}`"))?;
            if a < lo || b > hi || a > b {
                return Err(format!("cron: range {a}-{b} outside {lo}..={hi}"));
            }
            (a, b)
        } else {
            let n: u8 = range_part
                .parse()
                .map_err(|_| format!("cron: bad value `{range_part}` in `{field}`"))?;
            if n < lo || n > hi {
                return Err(format!("cron: value {n} outside {lo}..={hi}"));
            }
            (n, n)
        };
        let mut v = low;
        while v <= high {
            if !out.contains(&v) {
                out.push(v);
            }
            v = v.checked_add(step).unwrap_or(u8::MAX);
            if step == 0 {
                break;
            }
        }
    }
    out.sort_unstable();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_field_covers_full_range() {
        let e = CronExpr::parse("* * * * *").unwrap();
        assert_eq!(e.minutes.len(), 60);
        assert_eq!(e.hours.len(), 24);
        assert!(e.matches(0, 0, 1, 1, 0));
        assert!(e.matches(59, 23, 31, 12, 6));
    }

    #[test]
    fn every_15_minutes_at_business_hours() {
        let e = CronExpr::parse("*/15 9-17 * * 1-5").unwrap();
        assert_eq!(e.minutes, vec![0, 15, 30, 45]);
        assert_eq!(e.hours, vec![9, 10, 11, 12, 13, 14, 15, 16, 17]);
        // Wednesday at 10:30 → matches
        assert!(e.matches(30, 10, 15, 6, 3));
        // Saturday at 10:30 → no
        assert!(!e.matches(30, 10, 15, 6, 6));
        // Wed at 8:30 → no (hour out of range)
        assert!(!e.matches(30, 8, 15, 6, 3));
    }

    #[test]
    fn dom_dow_or_semantics() {
        // 1st of month OR every Friday
        let e = CronExpr::parse("0 9 1 * 5").unwrap();
        assert!(e.dom_restricted && e.dow_restricted);
        // 1st of the month, not Friday
        assert!(e.matches(0, 9, 1, 6, 3));
        // Friday, not the 1st
        assert!(e.matches(0, 9, 15, 6, 5));
        // Neither
        assert!(!e.matches(0, 9, 15, 6, 3));
    }

    #[test]
    fn bad_field_count_errors() {
        assert!(CronExpr::parse("* * * *").is_err());
        assert!(CronExpr::parse("* * * * * *").is_err());
    }

    #[test]
    fn out_of_range_errors() {
        assert!(CronExpr::parse("60 * * * *").is_err());
        assert!(CronExpr::parse("* 24 * * *").is_err());
        assert!(CronExpr::parse("* * 0 * *").is_err());
    }
}
