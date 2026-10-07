use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use super::error::GrpcChannelConfigError;

/// How many requests a channel starts in a window of time.
///
/// A window opens at the first request that finds none open and lasts `per`; a request that finds
/// the window's `calls` taken waits for its end, except a retry the retry policy chose, which is
/// skipped instead. A request is an attempt: a call's first, each retry and each resend of a
/// request its peer never processed all start one, because the server sees each of them as a
/// request. Windows are fixed, so up to twice `calls` requests can start within `per` across a
/// boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RateLimitConfig {
    /// The requests that start in one window; at least 1.
    pub calls: usize,
    /// The window's length; not zero, and held to ten years when longer.
    pub per: Duration,
}

impl RateLimitConfig {
    /// A limit of `calls` requests in each window of `per`; refused when the channel is created
    /// unless `calls` is at least 1 and `per` is not zero.
    pub fn new(calls: usize, per: Duration) -> Self {
        Self { calls, per }
    }

    pub(crate) fn admissible(&self) -> Result<(), GrpcChannelConfigError> {
        let refuse = |why: &str| {
            Err(GrpcChannelConfigError::RateLimit {
                why: why.to_owned(),
            })
        };
        if self.calls == 0 {
            return refuse("a `calls` of zero starts no request at all; 1 is the least");
        }
        if self.per.is_zero() {
            return refuse("a `per` of zero is a window that ends as it opens");
        }
        Ok(())
    }
}

/// The longest window, so that adding one to an `Instant` cannot overflow it.
const LONGEST_WINDOW: Duration = Duration::from_secs(10 * 365 * 24 * 3600);

/// A channel's turns: `calls` of them in each window, and a request that finds none left waits for
/// the next window, or, through `try_admit`, is told there is none free.
///
/// A window opens when a request asks while none is open, and lasts `per`, as tower's does; the
/// next one opens when the first request after the end of this one is let through.
/// Requests are let through in the order they reach the limiter, since the lock they wait on is
/// fair.
pub(crate) struct RateLimiter {
    calls: usize,
    per: Duration,
    window: Mutex<Window>,
}

struct Window {
    /// When the open window ends; none before the first request.
    ends: Option<Instant>,
    /// The turns it has left.
    left: usize,
}

impl RateLimiter {
    pub(crate) fn new(config: RateLimitConfig) -> Self {
        Self {
            calls: config.calls,
            per: config.per.min(LONGEST_WINDOW),
            window: Mutex::new(Window {
                ends: None,
                left: 0,
            }),
        }
    }

    /// Completes when a request may start, taking its turn.
    ///
    /// The turn is taken as this completes and not before, so a call that stops waiting - a
    /// deadline, a cancel, its channel closing - drops the future having taken none. The lock is
    /// held while the request waits for the window to end, which is what keeps the order.
    pub(crate) async fn admit(&self) {
        let mut window = self.window.lock().await;
        loop {
            let now = Instant::now();
            if window.ends.is_none_or(|ends| now >= ends) {
                // A clock that cannot hold the end of a window leaves nothing to wait for.
                window.ends = now.checked_add(self.per);
                window.left = self.calls;
            }
            match window.ends {
                Some(ends) if window.left == 0 => tokio::time::sleep_until(ends).await,
                _ => {
                    window.left = window.left.saturating_sub(1);
                    return;
                }
            }
        }
    }

    /// Takes a turn if one is free now, and otherwise none: a request that would have to wait,
    /// for the window to end or behind another that is waiting, is told so instead.
    pub(crate) fn try_admit(&self) -> bool {
        let Ok(mut window) = self.window.try_lock() else {
            return false;
        };
        let now = Instant::now();
        if window.ends.is_none_or(|ends| now >= ends) {
            window.ends = now.checked_add(self.per);
            window.left = self.calls;
        }
        if window.left == 0 {
            return false;
        }
        window.left -= 1;
        true
    }
}
