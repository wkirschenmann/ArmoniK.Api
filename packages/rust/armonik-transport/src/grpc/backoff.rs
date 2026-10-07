//! How long a channel holds off dialling after a dial fails: gRPC's connection backoff, whose
//! constants are those of `doc/connection-backoff.md`.
//!
//! Only a call that waits for the channel to be ready reads it. A call that does not dials at once
//! and fails with what the dial says.

use std::time::Duration;

use tokio::time::Instant;

/// The wait after the first failed dial.
const INITIAL: Duration = Duration::from_secs(1);

/// What the wait is multiplied by after each failed dial.
const MULTIPLIER: f64 = 1.6;

/// What the wait grows to and no further, jitter aside.
const MAX: Duration = Duration::from_secs(120);

/// How far a wait may fall short of its bound or exceed it, as a fraction of the bound.
const JITTER: f64 = 0.2;

/// The channel's failed dials in a row, and when it may dial again.
#[derive(Debug, Default)]
pub(crate) struct Backoff {
    failures: u32,
    retry_at: Option<Instant>,
}

impl Backoff {
    /// A dial failed at `now`.
    ///
    /// One failure per round: while the channel is held off already, another dial failing is the
    /// same round's news. Without that, calls that do not wait, which dial whatever the backoff,
    /// and dials in parallel would each advance it, and the wait would grow with the rate of
    /// calls rather than with time.
    pub(crate) fn failed(&mut self, now: Instant) {
        if self.pending(now).is_some() {
            return;
        }
        let wait = jittered(bound(self.failures), fastrand::f64());
        self.failures = self.failures.saturating_add(1);
        self.retry_at = Some(now + wait);
    }

    /// A dial opened a session.
    pub(crate) fn succeeded(&mut self) {
        *self = Self::default();
    }

    /// When the channel may dial again, if that is still ahead of `now`.
    pub(crate) fn pending(&self, now: Instant) -> Option<Instant> {
        self.retry_at.filter(|at| *at > now)
    }
}

/// What the wait after `failures` failed dials is bounded by.
fn bound(failures: u32) -> Duration {
    // Capped before the cast, which wraps a count past `i32::MAX` into a negative power.
    let seconds = INITIAL.as_secs_f64() * MULTIPLIER.powi(failures.min(64) as i32);
    Duration::from_secs_f64(seconds.min(MAX.as_secs_f64()))
}

/// `bound` moved by up to the jitter either way, by `draw` in `0..1`.
fn jittered(bound: Duration, draw: f64) -> Duration {
    bound.mul_f64(1.0 + JITTER * (2.0 * draw - 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bound_grows_by_the_multiplier_to_the_maximum() {
        assert_eq!(bound(0), Duration::from_secs(1));
        assert_eq!(bound(1), Duration::from_millis(1600));
        assert_eq!(bound(2), Duration::from_millis(2560));
        assert!(bound(10) < MAX);
        assert_eq!(bound(11), Duration::from_secs(120));
        assert_eq!(bound(u32::MAX), Duration::from_secs(120));
    }

    #[test]
    fn the_jitter_keeps_a_wait_within_a_fifth_of_its_bound() {
        let bound = Duration::from_secs(10);
        assert_eq!(jittered(bound, 0.0), Duration::from_secs(8));
        assert_eq!(jittered(bound, 0.5), bound);
        assert_eq!(jittered(bound, 1.0), Duration::from_secs(12));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_dial_holds_the_channel_off_until_one_succeeds() {
        let mut backoff = Backoff::default();
        let now = Instant::now();
        assert_eq!(backoff.pending(now), None);

        backoff.failed(now);
        let first = backoff.pending(now).expect("held off");
        assert!(first >= now + Duration::from_millis(800));
        assert!(first <= now + Duration::from_millis(1200));
        assert_eq!(backoff.pending(first), None, "and free again at the time");

        backoff.failed(first);
        let second = backoff.pending(first).expect("held off again");
        assert!(second >= first + Duration::from_millis(1280));

        // The same round: a dial failing while the channel is held off changes nothing.
        backoff.failed(first);
        assert_eq!(backoff.pending(first), Some(second));

        backoff.succeeded();
        assert_eq!(backoff.pending(first), None);
        backoff.failed(first);
        let third = backoff.pending(first).expect("held off");
        assert!(
            third <= first + Duration::from_millis(1200),
            "from the start"
        );
    }
}
