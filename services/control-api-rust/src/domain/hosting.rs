//! Rules for machines rented from providers (ADR 0050): prices, budgets, lifetimes.
//!
//! Pure functions, so "would this machine break the budget" and "is it time to delete it" are
//! tested without a provider.

use chrono::{DateTime, Duration, Utc};

/// The longest a machine may be asked to live, in minutes: one week. A longer need is a machine
/// somebody should run themselves.
pub const MAX_TTL_MINUTES: i64 = 7 * 24 * 60;

/// Whether adding a machine of `new_hourly_milli` keeps the total within `max_hourly_milli`.
/// A cap of 0 means no cap.
pub fn within_budget(max_hourly_milli: i64, running_hourly_milli: i64, new_hourly_milli: i64) -> bool {
    max_hourly_milli == 0 || running_hourly_milli + new_hourly_milli <= max_hourly_milli
}

/// A lifetime in minutes, kept within 15 minutes and a week.
pub fn clamp_ttl(minutes: i64) -> i64 {
    minutes.clamp(15, MAX_TTL_MINUTES)
}

/// What a machine has cost so far, in thousandths of a currency unit, rounded up to the started
/// hour the way most providers bill.
pub fn cost_so_far(hourly_milli: i64, created: DateTime<Utc>, until: DateTime<Utc>) -> i64 {
    let minutes = (until - created).num_minutes().max(0);
    let hours = (minutes + 59) / 60;
    hours.max(1) * hourly_milli
}

/// Why a machine should be deleted now, if it should.
#[derive(Debug, PartialEq, Eq)]
pub enum Reaping {
    Keep,
    /// Its lifetime is over.
    Expired,
    /// It has had nothing on it for as long as the host allows.
    Idle,
}

/// Decide for one machine. `empty` is whether the node holds no sandboxes, models or folders.
pub fn reaping(
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    idle_minutes: i32,
    idle_since: Option<DateTime<Utc>>,
    empty: bool,
) -> Reaping {
    if now >= expires_at {
        return Reaping::Expired;
    }
    if empty && idle_minutes > 0 {
        if let Some(since) = idle_since {
            if now - since >= Duration::minutes(i64::from(idle_minutes)) {
                return Reaping::Idle;
            }
        }
    }
    Reaping::Keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000, 0).expect("time") + Duration::minutes(minutes)
    }

    #[test]
    fn a_budget_of_zero_has_no_cap_and_otherwise_the_total_must_fit() {
        assert!(within_budget(0, 99_000, 99_000));
        assert!(within_budget(1_000, 600, 400));
        assert!(!within_budget(1_000, 600, 401));
    }

    #[test]
    fn a_lifetime_stays_between_a_quarter_hour_and_a_week() {
        assert_eq!(clamp_ttl(1), 15);
        assert_eq!(clamp_ttl(120), 120);
        assert_eq!(clamp_ttl(10_000_000), MAX_TTL_MINUTES);
    }

    #[test]
    fn cost_rounds_up_to_the_started_hour() {
        assert_eq!(cost_so_far(7, at(0), at(1)), 7);
        assert_eq!(cost_so_far(7, at(0), at(60)), 7);
        assert_eq!(cost_so_far(7, at(0), at(61)), 14);
        assert_eq!(cost_so_far(7, at(0), at(-5)), 7, "a clock that went back still costs the first hour");
    }

    #[test]
    fn a_machine_is_reaped_when_expired_or_idle_long_enough() {
        assert_eq!(reaping(at(10), at(60), 30, None, true), Reaping::Keep);
        assert_eq!(reaping(at(60), at(60), 30, None, false), Reaping::Expired);
        assert_eq!(reaping(at(50), at(600), 30, Some(at(10)), true), Reaping::Idle);
        assert_eq!(reaping(at(35), at(600), 30, Some(at(10)), true), Reaping::Keep, "not idle long enough");
        assert_eq!(reaping(at(50), at(600), 30, Some(at(10)), false), Reaping::Keep, "something is on it");
        assert_eq!(reaping(at(500), at(600), 0, Some(at(10)), true), Reaping::Keep, "idle deletion is off");
    }
}
