//! A single token bucket guarding synthetic-input throughput.
//!
//! **This is a safety guard, not a security control.** `org.wgaf.Input1`
//! lives on the session bus, and every process running as this user can open
//! `/dev/uinput` directly — that is exactly what the udev-rule/`input`-group
//! model grants, to the account rather than to wgaf. A hostile local process
//! gains nothing here it does not already have, and a limit it can bypass by
//! opening the device itself protects nobody.
//!
//! What it does guard against is the user's own runaway script: a loop bug
//! floods synthetic input, the desktop becomes unusable, and the user cannot
//! regain control because their own keystrokes compete with the flood.
//!
//! The arithmetic below is deliberately kept free of clocks and sleeping —
//! [`TokenBucket::acquire`] takes the current instant as an argument and
//! returns what *should* happen. That keeps it unit-testable with an injected
//! clock; a wall-clock test of a rate limiter is a flaky test.

use std::time::Duration;

use tokio::time::Instant;

/// What [`TokenBucket::acquire`] decided about one request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Acquired {
    /// Budget was available. Proceed immediately.
    Ready,
    /// Over budget. Proceed after sleeping this long — the caller is not
    /// refused, only slowed, so a legitimate long automation script still
    /// completes.
    After(Duration),
    /// The backlog is so far beyond budget that this is a runaway rather
    /// than a slow script. Refuse, and do not charge the request.
    Runaway {
        /// How long the caller would have had to wait. Reported to the user
        /// so the refusal names a number rather than a mood.
        would_wait: Duration,
    },
}

/// Signed token bucket. Tokens may go **negative**: a request costing more
/// than the entire capacity (a 4096-character `TypeText` is roughly 16,000
/// kernel events, against a capacity of a few thousand) takes the bucket into
/// debt and waits for it to be repaid, rather than waiting forever for a
/// capacity that can never cover it in one go. A bucket that only ever
/// admitted requests up to its capacity would deadlock on exactly the call
/// `input_max_type_text_chars` already permits at its default.
#[derive(Debug)]
pub(crate) struct TokenBucket {
    /// Events per second replenished. `0` disables the limiter entirely.
    rate: u32,
    /// Maximum tokens that can accumulate while idle — the burst allowance.
    capacity: f64,
    /// Current balance. Negative means debt owed.
    tokens: f64,
    /// When `tokens` was last brought up to date.
    last_refill: Instant,
    /// Refuse rather than sleep beyond this. See [`Acquired::Runaway`].
    max_delay: Duration,
}

impl TokenBucket {
    /// `rate` is events per second and doubles as the burst capacity, so an
    /// idle daemon can absorb one second's worth of work instantly. A `rate`
    /// of `0` disables the limiter — every [`Self::acquire`] returns
    /// [`Acquired::Ready`].
    pub(crate) fn new(rate: u32, max_delay: Duration, now: Instant) -> Self {
        Self {
            rate,
            capacity: f64::from(rate),
            tokens: f64::from(rate),
            last_refill: now,
            max_delay,
        }
    }

    /// Whether this bucket is switched off (`rate == 0`).
    pub(crate) fn is_disabled(&self) -> bool {
        self.rate == 0
    }

    /// Charges `cost` events against the budget as of `now`.
    ///
    /// Pure arithmetic: it neither reads a clock nor sleeps. The caller does
    /// both, which is what makes this testable.
    pub(crate) fn acquire(&mut self, cost: u32, now: Instant) -> Acquired {
        if self.is_disabled() {
            return Acquired::Ready;
        }

        // Refill for elapsed time, capped at capacity so idle time cannot
        // bank unlimited burst. `saturating_duration_since` guards against a
        // non-monotonic `now` (a paused test clock, mainly) producing a
        // negative refill.
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * f64::from(self.rate)).min(self.capacity);
        self.last_refill = now;

        let projected = self.tokens - f64::from(cost);
        if projected >= 0.0 {
            self.tokens = projected;
            return Acquired::Ready;
        }

        let wait = Duration::from_secs_f64(-projected / f64::from(self.rate));
        if wait > self.max_delay {
            // Deliberately does *not* charge the request. A refused call
            // should not deepen the debt for the calls behind it — that
            // would turn one runaway into a cascade of refusals long after
            // the runaway stopped.
            return Acquired::Runaway { would_wait: wait };
        }

        self.tokens = projected;
        Acquired::After(wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_DELAY: Duration = Duration::from_secs(30);

    /// Every test drives this clock by hand. Nothing here sleeps.
    fn clock() -> Instant {
        Instant::now()
    }

    #[test]
    fn spends_burst_capacity_without_waiting() {
        let start = clock();
        let mut bucket = TokenBucket::new(1000, MAX_DELAY, start);
        assert_eq!(bucket.acquire(1000, start), Acquired::Ready);
    }

    #[test]
    fn waits_once_the_burst_is_spent() {
        let start = clock();
        let mut bucket = TokenBucket::new(1000, MAX_DELAY, start);
        assert_eq!(bucket.acquire(1000, start), Acquired::Ready);

        // 500 events over budget at 1000/s is half a second.
        let Acquired::After(wait) = bucket.acquire(500, start) else {
            panic!("expected a throttle, not an immediate pass");
        };
        assert!(
            (wait.as_secs_f64() - 0.5).abs() < 1e-6,
            "expected ~0.5s, got {wait:?}"
        );
    }

    #[test]
    fn refills_over_time() {
        let start = clock();
        let mut bucket = TokenBucket::new(1000, MAX_DELAY, start);
        assert_eq!(bucket.acquire(1000, start), Acquired::Ready);
        // A full second later the bucket is back to capacity.
        let later = start + Duration::from_secs(1);
        assert_eq!(bucket.acquire(1000, later), Acquired::Ready);
    }

    #[test]
    fn refill_is_capped_at_capacity() {
        let start = clock();
        let mut bucket = TokenBucket::new(1000, MAX_DELAY, start);
        // An hour idle must not bank an hour's worth of burst.
        let much_later = start + Duration::from_secs(3600);
        assert_eq!(bucket.acquire(1000, much_later), Acquired::Ready);
        let Acquired::After(_) = bucket.acquire(1, much_later) else {
            panic!("idle time banked more than one capacity's worth of burst");
        };
    }

    /// The case a capacity-bounded bucket would deadlock on: `TypeText` at
    /// its default character cap costs far more than the whole bucket.
    #[test]
    fn a_request_larger_than_capacity_waits_rather_than_deadlocking() {
        let start = clock();
        let mut bucket = TokenBucket::new(1000, MAX_DELAY, start);
        // 16,000 events against 1000 capacity at 1000/s: 15s of debt.
        let Acquired::After(wait) = bucket.acquire(16_000, start) else {
            panic!("a cost above capacity must still be admitted, only delayed");
        };
        assert!(
            (wait.as_secs_f64() - 15.0).abs() < 1e-6,
            "expected ~15s, got {wait:?}"
        );
    }

    #[test]
    fn refuses_and_does_not_charge_beyond_the_ceiling() {
        let start = clock();
        let mut bucket = TokenBucket::new(1000, MAX_DELAY, start);
        let before = bucket.tokens;

        let Acquired::Runaway { would_wait } = bucket.acquire(100_000, start) else {
            panic!("expected a runaway refusal");
        };
        assert!(would_wait > MAX_DELAY);
        assert_eq!(
            bucket.tokens, before,
            "a refused request must not deepen the debt for the calls behind it"
        );
    }

    #[test]
    fn a_rate_of_zero_disables_the_limiter() {
        let start = clock();
        let mut bucket = TokenBucket::new(0, MAX_DELAY, start);
        assert!(bucket.is_disabled());
        assert_eq!(bucket.acquire(u32::MAX, start), Acquired::Ready);
    }

    /// A clock that goes backwards must not manufacture budget.
    #[test]
    fn a_backwards_clock_refills_nothing() {
        let start = clock() + Duration::from_secs(10);
        let mut bucket = TokenBucket::new(1000, MAX_DELAY, start);
        assert_eq!(bucket.acquire(1000, start), Acquired::Ready);
        let earlier = start - Duration::from_secs(5);
        let Acquired::After(_) = bucket.acquire(1, earlier) else {
            panic!("a backwards clock granted a refill");
        };
    }
}
