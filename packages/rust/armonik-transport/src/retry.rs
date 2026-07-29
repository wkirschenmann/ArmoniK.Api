//! Replaying failed requests according to a [`RetryPolicy`].
//!
//! The policy itself is pure: it decides *whether* to retry and *how long* to wait, but never
//! sleeps. Callers supply their own delay future, which keeps this module free of any runtime
//! dependency — `tokio` is only a direct dependency of the crate under the server features — and
//! lets the tests run against a fake clock instead of real time.

use std::future::Future;
use std::time::Duration;

use super::RetryPolicy;

/// Streaming shape of a gRPC method.
///
/// A request can only be replayed if the transport is able to reproduce it, so the shape of the
/// method decides whether retrying is allowed at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodKind {
    /// One request, one response.
    Unary,
    /// A stream of requests, one response.
    ClientStreaming,
    /// One request, a stream of responses.
    ServerStreaming,
    /// A stream of requests and a stream of responses.
    BidiStreaming,
}

impl RetryPolicy {
    /// Whether a failure with this status code makes a request eligible for a retry.
    pub fn is_retryable_status(&self, code: tonic::Code) -> bool {
        self.retryable_status_codes.contains(&code)
    }

    /// Whether a request of this shape may be replayed.
    ///
    /// Unary requests always can. Server-streaming requests can as long as no response has been
    /// handed to the caller yet, since replaying after that would duplicate messages. Requests that
    /// carry a client-side stream never can: reproducing them would mean buffering everything the
    /// caller wrote, which this implementation deliberately does not do. That is narrower than the
    /// gRPC retry specification, which does buffer.
    pub fn may_replay(&self, kind: MethodKind, messages_received: u64) -> bool {
        match kind {
            MethodKind::Unary => true,
            MethodKind::ServerStreaming => messages_received == 0,
            MethodKind::ClientStreaming | MethodKind::BidiStreaming => false,
        }
    }

    /// How long to wait before the next attempt, given how many attempts have already been made.
    ///
    /// The delay grows geometrically from [`RetryPolicy::initial_backoff`] and is capped by
    /// [`RetryPolicy::max_backoff`]. An overflowing multiplier saturates at the cap rather than
    /// panicking.
    pub fn backoff(&self, attempts_made: u32) -> Duration {
        let exponent = i32::try_from(attempts_made.saturating_sub(1)).unwrap_or(i32::MAX);
        let seconds = self.initial_backoff.as_secs_f64() * self.backoff_multiplier.powi(exponent);
        let capped = seconds.min(self.max_backoff.as_secs_f64());
        Duration::try_from_secs_f64(capped).unwrap_or(self.max_backoff)
    }

    /// Decide whether to retry, and how long to wait first.
    ///
    /// Returns [`None`] when the request must not be replayed: attempts exhausted, a status code
    /// outside the policy, or a shape that cannot be reproduced.
    pub fn should_retry(
        &self,
        kind: MethodKind,
        attempts_made: u32,
        messages_received: u64,
        code: tonic::Code,
    ) -> Option<Duration> {
        if attempts_made >= self.max_attempts
            || !self.is_retryable_status(code)
            || !self.may_replay(kind, messages_received)
        {
            return None;
        }
        Some(self.backoff(attempts_made))
    }
}

/// Drive `attempt` until it succeeds or the policy gives up.
///
/// `attempt` receives the 1-based attempt number and must start a *fresh* request each time, so
/// nothing observed by a previous attempt leaks into the next one. `sleep` produces the delay
/// future; passing `tokio::time::sleep` is the usual choice.
///
/// A `policy` of [`None`] means no retry: the first failure is returned as is. Because each attempt
/// is fresh, this drives establishing a call — and completing a unary one. A server-streaming call
/// that has already yielded messages must not be replayed through here; use
/// [`RetryPolicy::should_retry`] directly, which takes the message count into account.
///
/// ```ignore
/// let response = armonik::client::retry_with(
///     config.retry.as_ref(),
///     armonik::client::MethodKind::Unary,
///     |_attempt| async { client.versions().list().await },
///     tokio::time::sleep,
/// )
/// .await?;
/// ```
pub async fn retry_with<T, Attempt, AttemptFut, Sleep, SleepFut>(
    policy: Option<&RetryPolicy>,
    kind: MethodKind,
    mut attempt: Attempt,
    mut sleep: Sleep,
) -> Result<T, tonic::Status>
where
    Attempt: FnMut(u32) -> AttemptFut,
    AttemptFut: Future<Output = Result<T, tonic::Status>>,
    Sleep: FnMut(Duration) -> SleepFut,
    SleepFut: Future<Output = ()>,
{
    let mut attempts_made = 0u32;

    loop {
        attempts_made += 1;

        let status = match attempt(attempts_made).await {
            Ok(value) => return Ok(value),
            Err(status) => status,
        };

        let Some(policy) = policy else {
            return Err(status);
        };

        // Each attempt is fresh, so no response has been observed by the caller yet.
        let Some(delay) = policy.should_retry(kind, attempts_made, 0, status.code()) else {
            return Err(status);
        };

        tracing::debug!(
            attempts_made,
            max_attempts = policy.max_attempts,
            delay = %humantime::Duration::from(delay),
            code = ?status.code(),
            "Retrying after a failed attempt"
        );

        sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A clock that records what it was asked to wait for instead of waiting.
    #[derive(Default)]
    struct FakeClock(RefCell<Vec<Duration>>);

    impl FakeClock {
        fn sleep(&self) -> impl FnMut(Duration) -> std::future::Ready<()> + '_ {
            |delay| {
                self.0.borrow_mut().push(delay);
                std::future::ready(())
            }
        }

        fn recorded(&self) -> Vec<Duration> {
            self.0.borrow().clone()
        }
    }

    fn policy() -> RetryPolicy {
        RetryPolicy::default()
    }

    #[test]
    fn backoff_grows_geometrically_up_to_the_cap() {
        let policy = policy();

        // 1s, then x1.5 each time, capped at 5s.
        assert_eq!(policy.backoff(1), Duration::from_secs_f64(1.0));
        assert_eq!(policy.backoff(2), Duration::from_secs_f64(1.5));
        assert_eq!(policy.backoff(3), Duration::from_secs_f64(2.25));
        assert_eq!(policy.backoff(4), Duration::from_secs_f64(3.375));
        assert_eq!(policy.backoff(5), Duration::from_secs_f64(5.0));
        assert_eq!(policy.backoff(6), Duration::from_secs_f64(5.0));
    }

    #[test]
    fn backoff_saturates_instead_of_overflowing() {
        let policy = RetryPolicy {
            backoff_multiplier: 1e300,
            ..policy()
        };
        // A multiplier this large sends the geometric term to infinity; the cap must still hold.
        assert_eq!(policy.backoff(u32::MAX), policy.max_backoff);
    }

    #[test]
    fn backoff_of_the_first_attempt_is_the_initial_delay() {
        let policy = policy();
        // Guards the off-by-one: attempt 0 must not shift the sequence.
        assert_eq!(policy.backoff(0), policy.initial_backoff);
        assert_eq!(policy.backoff(1), policy.initial_backoff);
    }

    #[test]
    fn only_configured_status_codes_are_retryable() {
        let policy = policy();
        assert!(policy.is_retryable_status(tonic::Code::Unavailable));
        assert!(policy.is_retryable_status(tonic::Code::Aborted));
        assert!(policy.is_retryable_status(tonic::Code::Unknown));
        assert!(!policy.is_retryable_status(tonic::Code::InvalidArgument));
        assert!(!policy.is_retryable_status(tonic::Code::DeadlineExceeded));
    }

    #[test]
    fn streams_that_cannot_be_reproduced_are_never_replayed() {
        let policy = policy();

        assert!(policy.may_replay(MethodKind::Unary, 0));
        assert!(policy.may_replay(MethodKind::Unary, 7));

        // A server stream may be re-established only before the caller saw anything.
        assert!(policy.may_replay(MethodKind::ServerStreaming, 0));
        assert!(!policy.may_replay(MethodKind::ServerStreaming, 1));

        // The request stream is not buffered, so these can never be replayed.
        for messages in [0, 1] {
            assert!(!policy.may_replay(MethodKind::ClientStreaming, messages));
            assert!(!policy.may_replay(MethodKind::BidiStreaming, messages));
        }
    }

    #[test]
    fn should_retry_stops_at_the_attempt_limit() {
        let policy = policy();
        let code = tonic::Code::Unavailable;

        for attempts_made in 1..policy.max_attempts {
            assert!(
                policy
                    .should_retry(MethodKind::Unary, attempts_made, 0, code)
                    .is_some(),
                "attempt {attempts_made} should still be retried"
            );
        }
        assert_eq!(
            policy.should_retry(MethodKind::Unary, policy.max_attempts, 0, code),
            None
        );
    }

    #[tokio::test]
    async fn a_successful_attempt_never_sleeps() {
        let clock = FakeClock::default();
        let policy = policy();

        let result = retry_with(
            Some(&policy),
            MethodKind::Unary,
            |_| std::future::ready(Ok::<_, tonic::Status>(42)),
            clock.sleep(),
        )
        .await;

        assert_eq!(result.unwrap(), 42);
        assert!(clock.recorded().is_empty());
    }

    #[tokio::test]
    async fn failures_are_replayed_until_one_succeeds() {
        let clock = FakeClock::default();
        let policy = policy();
        let seen = RefCell::new(Vec::new());

        let result = retry_with(
            Some(&policy),
            MethodKind::Unary,
            |attempt| {
                seen.borrow_mut().push(attempt);
                std::future::ready(if attempt < 3 {
                    Err(tonic::Status::unavailable("not yet"))
                } else {
                    Ok(attempt)
                })
            },
            clock.sleep(),
        )
        .await;

        assert_eq!(result.unwrap(), 3);
        assert_eq!(*seen.borrow(), vec![1, 2, 3]);
        // Two failures, so two waits, following the geometric sequence.
        assert_eq!(
            clock.recorded(),
            vec![Duration::from_secs_f64(1.0), Duration::from_secs_f64(1.5)]
        );
    }

    #[tokio::test]
    async fn the_last_status_is_returned_once_attempts_run_out() {
        let clock = FakeClock::default();
        let policy = policy();
        let attempts = RefCell::new(0u32);

        let result = retry_with(
            Some(&policy),
            MethodKind::Unary,
            |attempt| {
                *attempts.borrow_mut() = attempt;
                std::future::ready(Err::<(), _>(tonic::Status::unavailable(format!(
                    "attempt {attempt}"
                ))))
            },
            clock.sleep(),
        )
        .await;

        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(status.message(), "attempt 5");
        assert_eq!(*attempts.borrow(), policy.max_attempts);
        // One wait fewer than the number of attempts: nothing is slept after the last failure.
        assert_eq!(
            clock.recorded().len() as u32,
            policy.max_attempts - 1,
            "should not wait after the final attempt"
        );
    }

    #[tokio::test]
    async fn a_status_outside_the_policy_fails_immediately() {
        let clock = FakeClock::default();
        let policy = policy();
        let attempts = RefCell::new(0u32);

        let result = retry_with(
            Some(&policy),
            MethodKind::Unary,
            |_| {
                *attempts.borrow_mut() += 1;
                std::future::ready(Err::<(), _>(tonic::Status::invalid_argument("nope")))
            },
            clock.sleep(),
        )
        .await;

        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
        assert_eq!(*attempts.borrow(), 1);
        assert!(clock.recorded().is_empty());
    }

    #[tokio::test]
    async fn without_a_policy_the_first_failure_is_final() {
        let clock = FakeClock::default();
        let attempts = RefCell::new(0u32);

        let result = retry_with(
            None,
            MethodKind::Unary,
            |_| {
                *attempts.borrow_mut() += 1;
                std::future::ready(Err::<(), _>(tonic::Status::unavailable("down")))
            },
            clock.sleep(),
        )
        .await;

        assert_eq!(result.unwrap_err().code(), tonic::Code::Unavailable);
        assert_eq!(*attempts.borrow(), 1);
        assert!(clock.recorded().is_empty());
    }

    #[tokio::test]
    async fn client_streaming_is_never_replayed() {
        let clock = FakeClock::default();
        let policy = policy();
        let attempts = RefCell::new(0u32);

        let result = retry_with(
            Some(&policy),
            MethodKind::ClientStreaming,
            |_| {
                *attempts.borrow_mut() += 1;
                std::future::ready(Err::<(), _>(tonic::Status::unavailable("down")))
            },
            clock.sleep(),
        )
        .await;

        assert_eq!(result.unwrap_err().code(), tonic::Code::Unavailable);
        assert_eq!(
            *attempts.borrow(),
            1,
            "a client stream cannot be reproduced, so it must not be retried"
        );
        assert!(clock.recorded().is_empty());
    }
}
