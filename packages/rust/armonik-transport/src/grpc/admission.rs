//! What a channel lets start, and what it lets be tried again, by what its server accepts.
//!
//! One estimate of the server's health, kept per channel over a window of time, sorts every attempt
//! that went out into three classes: it was accepted, or it failed because the server is over
//! capacity, or it failed transiently. Two lists of failures, which are data, say which is which.
//! Two readings of the same counts are taken. The retry reading counts every failure, and retries
//! are allowed while it stays under a slack. The rate reading counts overload alone, and while it is over the slack the channel caps the rate of
//! first attempts, which then wait for their turns; a transient failure, a server that is down, is no
//! reason to slow calls down.
//!
//! The state is a few atomic words, so that a decision costs a compare-exchange at most and is
//! correct under any number of threads: the counts are a ring of twelve slots, each one word, and
//! the cap is a cell of the generic cell rate algorithm, one word. The only lock is the queue of the
//! first attempts that wait for a turn, so that they start in the order they arrived.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use super::cause::{self, Cause};
use super::error::GrpcChannelConfigError;
use super::origin::{Origin, Pushback};
use super::rate_limit::{RateLimitConfig, RateLimiter};
use super::status::GrpcStatusCode;

/// How a channel judges the health of its server, and what it does about it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AdaptiveConfig {
    /// The failures that may be an outage: a retry is not worth sending while the server fails more
    /// than it accepts, and a failure of these kinds never lowers the rate of first attempts. A
    /// failure of the server's in neither list counts as an accept.
    pub transient: Vec<Cause>,
    /// The failures that say the server is over capacity: they stop retries as the transient ones do,
    /// and they alone cap the rate of first attempts. Checked before `transient`.
    pub overload: Vec<Cause>,
    /// `K`, of the retry reading: retries stop when the channel has sent more than `K` times what
    /// the server accepted, beyond the slack. At least 1 and at most 100.
    pub multiplier: f64,
    /// `K_t`, of the rate reading: the rate is capped when the channel has sent more than `K_t`
    /// times what the server did not report as overloaded, beyond the slack. At least 1 and at most
    /// 100.
    pub throttle_multiplier: f64,
    /// `S`, the failures beyond `K` times the accepts that go unanswered: a channel with little
    /// traffic does not lose its retries to one failure.
    pub slack: u32,
    /// `W`, how far back the counts reach, between 12 milliseconds and 600 seconds.
    pub window: Duration,
    /// The rate of first attempts, a second, that the cap never goes under, so that the channel
    /// goes on probing the server.
    pub floor_per_second: f64,
}

/// The failures that say a server is over capacity: `RESOURCE_EXHAUSTED` from the server, an HTTP
/// 429, a reset with `ENHANCE_YOUR_CALM` or with `REFUSED_STREAM` (a refusal during a drain comes
/// with a GOAWAY, which is its own origin), and a pushback that asks for a wait.
pub fn default_overload() -> Vec<Cause> {
    vec![
        Cause::Status(GrpcStatusCode::ResourceExhausted),
        Cause::Http(429),
        Cause::Pushback,
        Cause::Reset(11),
        Cause::Reset(7),
    ]
}

/// The failures that may be an outage: `UNAVAILABLE` from the server, a proxy's 408, 500, 502, 503
/// and 504, and a dial or a connection that failed.
pub fn default_transient() -> Vec<Cause> {
    let mut causes = vec![Cause::Status(GrpcStatusCode::Unavailable)];
    causes.extend([408, 500, 502, 503, 504].map(Cause::Http));
    causes.extend([Cause::Dial, Cause::Connection]);
    causes
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            transient: default_transient(),
            overload: default_overload(),
            multiplier: 2.0,
            throttle_multiplier: 2.0,
            slack: 10,
            window: Duration::from_secs(30),
            floor_per_second: 0.5,
        }
    }
}

/// The shortest window: a slot of a twelfth of it is then a millisecond.
const SHORTEST_WINDOW: Duration = Duration::from_millis(12);
const LONGEST_WINDOW: Duration = Duration::from_secs(600);

impl AdaptiveConfig {
    pub(crate) fn admissible(&self) -> Result<(), GrpcChannelConfigError> {
        let refuse = |why: String| Err(GrpcChannelConfigError::Adaptive { why });
        for (name, causes) in [("transient", &self.transient), ("overload", &self.overload)] {
            for cause in causes {
                match cause {
                    Cause::Status(
                        GrpcStatusCode::Ok
                        | GrpcStatusCode::Cancelled
                        | GrpcStatusCode::DeadlineExceeded,
                    ) => {
                        return refuse(format!(
                            "`{name}` names {cause}, which is never counted: it is the server's \
                             success, the caller's cancel or the caller's own deadline"
                        ))
                    }
                    Cause::Http(status) if !(100..=599).contains(status) => {
                        return refuse(format!("`{name}` names HTTP {status}, which is no status"))
                    }
                    _ => {}
                }
            }
        }
        for (name, value) in [
            ("multiplier", self.multiplier),
            ("throttle_multiplier", self.throttle_multiplier),
        ] {
            if !(value.is_finite() && (1.0..=100.0).contains(&value)) {
                return refuse(format!("`{name}` has to be a number from 1 to 100"));
            }
        }
        if self.slack > 1_000_000 {
            return refuse("`slack` has to be at most 1000000".to_owned());
        }
        if !(SHORTEST_WINDOW..=LONGEST_WINDOW).contains(&self.window) {
            return refuse("`window` has to be from 0.012 to 600 seconds".to_owned());
        }
        if !(self.floor_per_second.is_finite()
            && self.floor_per_second > 0.0
            && self.floor_per_second <= 1_000_000.0)
        {
            return refuse("`floor_per_second` has to be above 0 and at most 1000000".to_owned());
        }
        Ok(())
    }
}

/// What an attempt that ended is counted as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// The server took the request and answered.
    Accept,
    /// It failed in a way that may be an outage.
    Transient,
    /// The server said, or the path said for it, that it is over capacity.
    Overload,
}

/// What an attempt that went out and ended counts as, or nothing: where the end came from decides,
/// since the engine maps several origins onto one code, and the lists of `config` say which
/// failures are overload and which are transient.
///
/// An answer of the server's that is in neither list is an accept: an application failure is not a
/// failure of capacity or availability. Nothing is counted for a deadline, a cancel, a GOAWAY, an
/// unsent request, a stream that broke after its head, or what the engine ended itself; nor for a
/// reset, a dial or a connection failure that no list names, which no server answered.
pub(crate) fn classify(
    origin: &Origin,
    code: GrpcStatusCode,
    pushback: Pushback,
    config: &AdaptiveConfig,
) -> Option<Class> {
    match origin {
        Origin::GoAway | Origin::Unsent | Origin::Broke | Origin::Local => return None,
        Origin::Server => match code {
            GrpcStatusCode::Ok => return Some(Class::Accept),
            GrpcStatusCode::DeadlineExceeded | GrpcStatusCode::Cancelled => return None,
            _ => {}
        },
        // Nothing says whether capacity was the cause of an HTTP 200 without a gRPC content type.
        Origin::Http(status) if status.as_u16() == 200 => return None,
        _ => {}
    }
    if cause::names(&config.overload, origin, code, pushback) {
        Some(Class::Overload)
    } else if cause::names(&config.transient, origin, code, pushback) {
        Some(Class::Transient)
    } else {
        match origin {
            // Something answered, and not as a refusal of capacity or a failure to be there.
            Origin::Server | Origin::Http(_) => Some(Class::Accept),
            _ => None,
        }
    }
}

const SLOTS: usize = 12;

/// The most one count of a slot holds, 16 bits.
const LIMIT: u64 = 0xFFFF;

/// The ring's words, on two cache lines of their own.
#[repr(align(64))]
struct Slots([AtomicU64; SLOTS]);

/// A word on a cache line of its own.
#[repr(align(64))]
struct Padded<T>(T);

/// What the window holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Attempts the server accepted.
    pub(crate) accepted: u64,
    /// Attempts that failed transiently.
    pub(crate) transient: u64,
    /// Attempts the server overloaded.
    pub(crate) overloaded: u64,
}

impl Counts {
    /// The attempts that ended and counted.
    pub(crate) fn sent(self) -> u64 {
        self.accepted + self.transient + self.overloaded
    }
}

/// A slot is one word: the low 16 bits of its interval's number, and three counts of 16 bits.
fn tag_of(word: u64) -> u64 {
    word >> 48
}

fn counts_of(word: u64) -> Counts {
    Counts {
        accepted: (word >> 32) & LIMIT,
        transient: (word >> 16) & LIMIT,
        overloaded: word & LIMIT,
    }
}

fn word_of(tag: u64, counts: Counts) -> u64 {
    (tag << 48) | (counts.accepted << 32) | (counts.transient << 16) | counts.overloaded
}

/// The word `cur` becomes when an attempt of `class` is recorded in the interval `tag`: a slot of
/// another interval starts over, and a slot with a count at its limit halves all three first, which
/// keeps the ratios.
fn recorded(cur: u64, tag: u64, class: Class) -> u64 {
    let mut counts = if tag_of(cur) == tag {
        counts_of(cur)
    } else {
        Counts::default()
    };
    let count = match class {
        Class::Accept => &mut counts.accepted,
        Class::Transient => &mut counts.transient,
        Class::Overload => &mut counts.overloaded,
    };
    if *count == LIMIT {
        counts.accepted /= 2;
        counts.transient /= 2;
        counts.overloaded /= 2;
    }
    match class {
        Class::Accept => counts.accepted += 1,
        Class::Transient => counts.transient += 1,
        Class::Overload => counts.overloaded += 1,
    }
    word_of(tag, counts)
}

/// The counts of the last twelve intervals, kept as twelve words.
///
/// An interval is a twelfth of the window, its number the time divided by it, and its slot that
/// number modulo twelve; a slot holds the low 16 bits of the number it was last written for, and a
/// slot whose number is not among the last twelve's is expired. So the window has the granularity of
/// a slot: a count leaves between eleven twelfths of a window and a whole one after it was recorded.
/// A slot that is never written again reads as live once the number repeats, after 196608 intervals.
struct Ring {
    slots: Slots,
    /// The length of an interval, nanoseconds; at least a millisecond.
    interval: u64,
}

impl Ring {
    fn new(window: Duration) -> Self {
        Self {
            slots: Slots(std::array::from_fn(|_| AtomicU64::new(0))),
            interval: u64::try_from(window.as_nanos() / SLOTS as u128)
                .unwrap_or(u64::MAX)
                .max(1_000_000),
        }
    }

    /// Counts an attempt of `class` that ended at `now`, nanoseconds from the epoch, with a
    /// compare-exchange on the one slot that holds the interval; never an unconditional add, so
    /// that a count cannot carry into the interval bits.
    fn record(&self, class: Class, now: u64) {
        let number = now / self.interval;
        let tag = number & 0xFFFF;
        let slot = &self.slots.0[(number % SLOTS as u64) as usize];
        let mut cur = slot.load(Ordering::Relaxed);
        loop {
            match slot.compare_exchange_weak(
                cur,
                recorded(cur, tag, class),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(seen) => cur = seen,
            }
        }
    }

    /// The counts of the slots that hold one of the last twelve intervals at `now`. The slots are
    /// read one at a time, so a read may mix moments a few records apart, which the slack absorbs.
    fn read(&self, now: u64) -> Counts {
        let number = now / self.interval;
        let mut total = Counts::default();
        for back in 0..SLOTS as u64 {
            let Some(held) = number.checked_sub(back) else {
                break;
            };
            let word = self.slots.0[(held % SLOTS as u64) as usize].load(Ordering::Relaxed);
            if tag_of(word) == held & 0xFFFF {
                let counts = counts_of(word);
                total.accepted += counts.accepted;
                total.transient += counts.transient;
                total.overloaded += counts.overloaded;
            }
        }
        total
    }
}

/// A channel's judgment of its server: the counts, and the cap on first attempts they set.
pub(crate) struct Adaptive {
    config: AdaptiveConfig,
    epoch: Instant,
    ring: Ring,
    /// The theoretical arrival time of the cap's cell: nanoseconds from the epoch at which the
    /// next turn of the cap is due.
    tat: Padded<AtomicU64>,
    /// First attempts that have joined the queue of the cap and not left it.
    waiting: Padded<AtomicUsize>,
    /// The queue: whoever holds it is next, and sleeps with it until its turn is due.
    order: Mutex<()>,
}

/// Counts a waiting first attempt, until it takes its turn or stops waiting.
struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn join(waiting: &'a AtomicUsize) -> Self {
        waiting.fetch_add(1, Ordering::SeqCst);
        Self(waiting)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What the counts say at an instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reading {
    /// The counts of the window, read by the test hooks.
    #[allow(dead_code)]
    pub(crate) counts: Counts,
    /// Whether the retry reading is at or under the slack: `R - K * A <= S`.
    pub(crate) retries_open: bool,
    /// The rate of first attempts the channel may start, a second, while the rate reading is over
    /// the slack, and none while it is not.
    pub(crate) cap: Option<f64>,
}

impl Adaptive {
    pub(crate) fn new(config: AdaptiveConfig) -> Self {
        Self {
            ring: Ring::new(config.window),
            config,
            epoch: Instant::now(),
            tat: Padded(AtomicU64::new(0)),
            waiting: Padded(AtomicUsize::new(0)),
            order: Mutex::new(()),
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(
            Instant::now()
                .saturating_duration_since(self.epoch)
                .as_nanos(),
        )
        .unwrap_or(u64::MAX)
    }

    pub(crate) fn record(&self, class: Class) {
        self.ring.record(class, self.now());
    }

    #[cfg(test)]
    pub(crate) fn record_at(&self, class: Class, now: u64) {
        self.ring.record(class, now);
    }

    /// What the counts say at `now`.
    pub(crate) fn reading_at(&self, now: u64) -> Reading {
        let counts = self.ring.read(now);
        let AdaptiveConfig {
            multiplier,
            throttle_multiplier,
            slack,
            window,
            floor_per_second,
            ..
        } = self.config;
        let sent = counts.sent() as f64;
        let slack = f64::from(slack);
        let retries_open = sent - multiplier * counts.accepted as f64 <= slack;

        let not_overloaded = (counts.accepted + counts.transient) as f64;
        let capped = sent - throttle_multiplier * not_overloaded > slack;
        let cap = capped.then(|| {
            // The window, or the age of the estimate when that is less, so that a new channel
            // is not read over a window it has not lived.
            let age = Duration::from_nanos(now).clamp(Duration::from_millis(1), window);
            (throttle_multiplier * not_overloaded / age.as_secs_f64()).max(floor_per_second)
        });
        Reading {
            counts,
            retries_open,
            cap,
        }
    }

    /// Whether the cap has first attempts waiting, or holds the rate now.
    fn busy(&self) -> bool {
        self.waiting.0.load(Ordering::SeqCst) > 0 || self.reading_at(self.now()).cap.is_some()
    }

    /// Whether a failed call may be tried again: the retry reading is at or under the slack, and the
    /// rate is not capped, since a retry is never sent above the cap.
    pub(crate) fn retries_open(&self) -> bool {
        let reading = self.reading_at(self.now());
        reading.retries_open && reading.cap.is_none()
    }

    /// The cell of the cap, read at the rate `cap`: `T = 1 / cap`, and a depth of one second of
    /// turns at that rate, at least one.
    fn take(&self, now: u64, cap: f64) -> Result<(), u64> {
        let interval = ((1e9 / cap) as u64).max(1);
        let burst = (cap.ceil() as u64).max(1);
        let allowed = (burst - 1).saturating_mul(interval);
        let mut tat = self.tat.0.load(Ordering::Relaxed);
        loop {
            let ahead = tat.saturating_sub(now);
            if ahead > allowed {
                return Err(ahead - allowed);
            }
            match self.tat.0.compare_exchange_weak(
                tat,
                tat.max(now).saturating_add(interval),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(()),
                Err(seen) => tat = seen,
            }
        }
    }

    /// Forgets the debt of the cap once the rate is not capped, so that none outlives it: a
    /// compare-exchange from the value read to zero, which a take that came first defeats and the
    /// next uncapped decision tries again.
    fn forget_debt(&self) {
        let tat = self.tat.0.load(Ordering::Relaxed);
        if tat != 0 {
            let _ = self
                .tat
                .0
                .compare_exchange(tat, 0, Ordering::Relaxed, Ordering::Relaxed);
        }
    }

    /// Takes a turn of the cap if the rate is capped, or lets the first attempt start if it is
    /// not, at `now`; none when it must wait, and for how long.
    fn turn(&self, now: u64) -> Result<(), u64> {
        match self.reading_at(now).cap {
            None => {
                self.forget_debt();
                Ok(())
            }
            Some(cap) => self.take(now, cap),
        }
    }

    /// Completes when a first attempt may start, taking its turn.
    ///
    /// While the rate is not capped and nobody waits it completes at once, with no write at all.
    /// Otherwise it joins the queue, whose head sleeps until its turn is due and looks again
    /// every interval of the window, since records of the attempts in flight may lift the cap
    /// meanwhile; the whole queue then goes on, in order. A first attempt that stops waiting at
    /// the cap - a deadline, a cancel, its channel closing - drops the future having taken no turn.
    pub(crate) async fn admit_first(&self) {
        if self.waiting.0.load(Ordering::SeqCst) == 0 && self.turn(self.now()).is_ok() {
            return;
        }
        let _counted = Waiting::join(&self.waiting.0);
        let _head = self.order.lock().await;
        let look_again = Duration::from_nanos(self.ring.interval);
        loop {
            match self.turn(self.now()) {
                Ok(()) => return,
                Err(wait) => tokio::time::sleep(Duration::from_nanos(wait).min(look_again)).await,
            }
        }
    }

    /// Where the estimate stands, for a test.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn state(&self) -> Reading {
        self.reading_at(self.now())
    }
}

/// Everything that decides whether an attempt starts: the configured limit, which counts starts, and
/// the adaptive estimate, which judges what the server accepts. The driver of a call asks this and
/// nothing else.
pub(crate) struct Admission {
    limiter: Option<RateLimiter>,
    adaptive: Option<Adaptive>,
}

impl Admission {
    pub(crate) fn new(limit: Option<RateLimitConfig>, adaptive: Option<AdaptiveConfig>) -> Self {
        Self {
            limiter: limit.map(RateLimiter::new),
            adaptive: adaptive.map(Adaptive::new),
        }
    }

    /// Whether a first attempt may have to wait for its turn: there is a configured limit, or the
    /// rate is capped or has first attempts queued.
    pub(crate) fn may_wait(&self) -> bool {
        self.limiter.is_some() || self.adaptive.as_ref().is_some_and(Adaptive::busy)
    }

    /// Completes when a first attempt, or a resend of a request its peer never processed, may start,
    /// having taken its turn at each gate in turn: the cap of the estimate first, and then the
    /// configured limit.
    pub(crate) async fn first_attempt(&self) {
        if let Some(adaptive) = &self.adaptive {
            adaptive.admit_first().await;
        }
        if let Some(limiter) = &self.limiter {
            limiter.admit().await;
        }
    }

    /// Whether a failed call may be tried again as far as the health of the server goes; asked at the
    /// failure and again when the backoff has passed.
    pub(crate) fn retries_open(&self) -> bool {
        self.adaptive.as_ref().is_none_or(Adaptive::retries_open)
    }

    /// Takes the turn a retry needs of the configured limit, if it is free now.
    pub(crate) fn retry_turn(&self) -> bool {
        self.limiter.as_ref().is_none_or(RateLimiter::try_admit)
    }

    /// Counts an attempt that went out and ended, as what its origin, code and pushback say.
    pub(crate) fn record(&self, origin: &Origin, code: GrpcStatusCode, pushback: Pushback) {
        let Some(adaptive) = &self.adaptive else {
            return;
        };
        if let Some(class) = classify(origin, code, pushback, &adaptive.config) {
            adaptive.record(class);
        }
    }

    /// Where the estimate stands, for a test.
    #[cfg(feature = "test-hooks")]
    pub(crate) fn state(&self) -> Option<Reading> {
        self.adaptive.as_ref().map(Adaptive::state)
    }
}

#[cfg(test)]
mod tests {
    use http::StatusCode;

    use super::*;

    const SECOND: u64 = 1_000_000_000;

    fn adaptive() -> Adaptive {
        Adaptive::new(AdaptiveConfig::default())
    }

    fn counts(accepted: u64, transient: u64, overloaded: u64) -> Counts {
        Counts {
            accepted,
            transient,
            overloaded,
        }
    }

    /// `n` attempts of `class` recorded at `now`.
    fn record(estimate: &Adaptive, class: Class, n: u64, now: u64) {
        for _ in 0..n {
            estimate.record_at(class, now);
        }
    }

    #[test]
    fn every_origin_is_sorted_as_the_default_lists_have_it() {
        let ok = GrpcStatusCode::Ok;
        let config = AdaptiveConfig::default();
        let sort = |origin: Origin, code, pushback| classify(&origin, code, pushback, &config);
        let none = Pushback::Unsaid;
        let wait = Pushback::After(Duration::from_millis(5));

        // The server's own status.
        assert_eq!(sort(Origin::Server, ok, none), Some(Class::Accept));
        for code in [
            GrpcStatusCode::NotFound,
            GrpcStatusCode::InvalidArgument,
            GrpcStatusCode::AlreadyExists,
            GrpcStatusCode::FailedPrecondition,
            GrpcStatusCode::PermissionDenied,
            GrpcStatusCode::Unauthenticated,
            GrpcStatusCode::OutOfRange,
            GrpcStatusCode::Unimplemented,
            GrpcStatusCode::Aborted,
            GrpcStatusCode::Unknown,
            GrpcStatusCode::Internal,
            GrpcStatusCode::DataLoss,
        ] {
            assert_eq!(
                sort(Origin::Server, code, none),
                Some(Class::Accept),
                "{code:?}"
            );
        }
        assert_eq!(
            sort(Origin::Server, GrpcStatusCode::ResourceExhausted, none),
            Some(Class::Overload)
        );
        assert_eq!(
            sort(Origin::Server, GrpcStatusCode::Unavailable, none),
            Some(Class::Transient)
        );
        for code in [GrpcStatusCode::DeadlineExceeded, GrpcStatusCode::Cancelled] {
            assert_eq!(sort(Origin::Server, code, none), None, "{code:?}");
        }

        // A pushback that asks for a wait is overload, whatever the code; one that refuses a
        // retry leaves the code to decide.
        assert_eq!(
            sort(Origin::Server, GrpcStatusCode::Aborted, wait),
            Some(Class::Overload)
        );
        assert_eq!(
            sort(
                Origin::Server,
                GrpcStatusCode::Unavailable,
                Pushback::Refused
            ),
            Some(Class::Transient)
        );

        // A proxy's answer with no status.
        let http = |status: u16| Origin::Http(StatusCode::from_u16(status).expect("a status"));
        assert_eq!(
            sort(http(429), GrpcStatusCode::Unavailable, none),
            Some(Class::Overload)
        );
        for status in [408, 500, 502, 503, 504] {
            assert_eq!(
                sort(http(status), GrpcStatusCode::Unknown, none),
                Some(Class::Transient),
                "{status}"
            );
        }
        for status in [400, 401, 403, 404] {
            assert_eq!(
                sort(http(status), GrpcStatusCode::Internal, none),
                Some(Class::Accept),
                "{status}"
            );
        }
        assert_eq!(
            sort(http(200), GrpcStatusCode::Unknown, none),
            None,
            "an HTTP 200 without a gRPC status says nothing of capacity"
        );
        for status in [301, 413, 507] {
            assert_eq!(
                sort(http(status), GrpcStatusCode::Unknown, none),
                Some(Class::Accept),
                "{status}: an answer no list names"
            );
        }

        // A reset, by its reason, before the head.
        let reset = |reason| sort(Origin::Reset(reason), GrpcStatusCode::Unavailable, none);
        assert_eq!(reset(h2::Reason::ENHANCE_YOUR_CALM), Some(Class::Overload));
        assert_eq!(reset(h2::Reason::REFUSED_STREAM), Some(Class::Overload));
        for reason in [
            h2::Reason::CANCEL,
            h2::Reason::INADEQUATE_SECURITY,
            h2::Reason::INTERNAL_ERROR,
            h2::Reason::PROTOCOL_ERROR,
            h2::Reason::NO_ERROR,
            h2::Reason::FLOW_CONTROL_ERROR,
        ] {
            assert_eq!(reset(reason), None, "{reason:?}: no list names it");
        }

        // The connection and the dial.
        assert_eq!(
            sort(Origin::Connection, GrpcStatusCode::Unavailable, none),
            Some(Class::Transient)
        );
        assert_eq!(
            sort(Origin::Dial, GrpcStatusCode::Unavailable, none),
            Some(Class::Transient)
        );

        // What is nobody's refusal of load.
        for origin in [Origin::GoAway, Origin::Unsent, Origin::Broke, Origin::Local] {
            assert_eq!(
                sort(origin.clone(), GrpcStatusCode::Unavailable, none),
                None,
                "{origin:?}"
            );
        }
    }

    /// The lists are data: what they name decides, and a server failure in neither list is an
    /// accept.
    #[test]
    fn the_lists_say_which_failures_are_which() {
        let mut config = AdaptiveConfig::default();
        let sort = |config: &AdaptiveConfig, origin: Origin, code, pushback| {
            classify(&origin, code, pushback, config)
        };
        let none = Pushback::Unsaid;

        // A server that signals overload as its outage: UNAVAILABLE is overload, and so is a dial.
        config
            .overload
            .extend([Cause::Status(GrpcStatusCode::Unavailable), Cause::Dial]);
        config.transient.retain(|cause| {
            !matches!(
                cause,
                Cause::Status(GrpcStatusCode::Unavailable) | Cause::Dial
            )
        });
        assert_eq!(
            sort(&config, Origin::Server, GrpcStatusCode::Unavailable, none),
            Some(Class::Overload)
        );
        assert_eq!(
            sort(&config, Origin::Dial, GrpcStatusCode::Unavailable, none),
            Some(Class::Overload)
        );

        // Named in both, the overload list is read first.
        config
            .transient
            .push(Cause::Status(GrpcStatusCode::ResourceExhausted));
        assert_eq!(
            sort(
                &config,
                Origin::Server,
                GrpcStatusCode::ResourceExhausted,
                none
            ),
            Some(Class::Overload)
        );

        // A status in neither list is an accept, and a failure that no server answered, named in
        // neither, counts nothing.
        let empty = AdaptiveConfig {
            transient: Vec::new(),
            overload: Vec::new(),
            ..AdaptiveConfig::default()
        };
        assert_eq!(
            sort(&empty, Origin::Server, GrpcStatusCode::Unavailable, none),
            Some(Class::Accept)
        );
        assert_eq!(
            sort(&empty, Origin::Dial, GrpcStatusCode::Unavailable, none),
            None
        );
        assert_eq!(
            sort(
                &empty,
                Origin::Connection,
                GrpcStatusCode::Unavailable,
                none
            ),
            None
        );
        assert_eq!(
            sort(
                &empty,
                Origin::Reset(h2::Reason::INTERNAL_ERROR),
                GrpcStatusCode::Internal,
                none
            ),
            None
        );

        // A pushback counts only if a list names it.
        let wait = Pushback::After(Duration::from_millis(5));
        assert_eq!(
            sort(&empty, Origin::Server, GrpcStatusCode::Aborted, wait),
            Some(Class::Accept)
        );
        let transient = AdaptiveConfig {
            transient: vec![Cause::Pushback],
            overload: Vec::new(),
            ..AdaptiveConfig::default()
        };
        assert_eq!(
            sort(&transient, Origin::Server, GrpcStatusCode::Aborted, wait),
            Some(Class::Transient)
        );

        // Never counted, whatever the lists: the caller's deadline and cancel, a GOAWAY, an unsent
        // request, a broken stream and the engine's own.
        for origin in [Origin::GoAway, Origin::Unsent, Origin::Broke, Origin::Local] {
            assert_eq!(
                sort(&config, origin.clone(), GrpcStatusCode::Unavailable, wait),
                None,
                "{origin:?}"
            );
        }
        for code in [GrpcStatusCode::DeadlineExceeded, GrpcStatusCode::Cancelled] {
            assert_eq!(sort(&config, Origin::Server, code, wait), None, "{code:?}");
        }
    }

    /// Naming what is never counted is refused.
    #[test]
    fn a_list_that_names_what_is_never_counted_is_refused() {
        for cause in [
            Cause::Status(GrpcStatusCode::Ok),
            Cause::Status(GrpcStatusCode::Cancelled),
            Cause::Status(GrpcStatusCode::DeadlineExceeded),
            Cause::Http(99),
            Cause::Http(600),
        ] {
            for config in [
                AdaptiveConfig {
                    transient: vec![cause],
                    ..AdaptiveConfig::default()
                },
                AdaptiveConfig {
                    overload: vec![cause],
                    ..AdaptiveConfig::default()
                },
            ] {
                assert!(config.admissible().is_err(), "{cause:?}");
            }
        }
    }

    #[test]
    fn a_count_is_in_the_window_until_it_ages_out_between_eleven_twelfths_and_a_whole_window() {
        let estimate = adaptive();
        let slot = estimate.ring.interval;
        let window = slot * 12;
        // Recorded at the start of an interval, which is the longest it stays.
        let at = 5 * window;
        record(&estimate, Class::Accept, 3, at);

        assert_eq!(estimate.ring.read(at).accepted, 3);
        assert_eq!(
            estimate.ring.read(at + 11 * slot).accepted,
            3,
            "11 slots on"
        );
        assert_eq!(
            estimate.ring.read(at + 12 * slot - 1).accepted,
            3,
            "just under a window"
        );
        assert_eq!(
            estimate.ring.read(at + 12 * slot).accepted,
            0,
            "a whole window on"
        );

        // Recorded at the end of its interval, it leaves a slot earlier.
        let late = at + slot - 1;
        record(&estimate, Class::Overload, 2, late);
        assert_eq!(estimate.ring.read(late + 10 * slot).overloaded, 2);
        assert_eq!(estimate.ring.read(late + 11 * slot + 1).overloaded, 0);
    }

    #[test]
    fn an_idle_gap_leaves_nothing_in_the_window_and_a_record_after_it_is_counted() {
        let estimate = adaptive();
        let slot = estimate.ring.interval;
        record(&estimate, Class::Accept, 7, 3 * slot);

        // A window and more, and very long gaps.
        for gap in [12, 13, 100, 5_000] {
            assert_eq!(
                estimate.ring.read((3 + gap) * slot),
                Counts::default(),
                "{gap}"
            );
        }
        let later = (3 + 70_000) * slot;
        record(&estimate, Class::Transient, 1, later);
        assert_eq!(estimate.ring.read(later), counts(0, 1, 0));
    }

    #[test]
    fn a_slot_at_its_count_limit_halves_all_three_counts_and_keeps_the_interval() {
        let estimate = adaptive();
        let slot = estimate.ring.interval;
        let at = 9 * slot;
        record(&estimate, Class::Accept, LIMIT, at);
        record(&estimate, Class::Overload, 1000, at);
        assert_eq!(estimate.ring.read(at), counts(LIMIT, 0, 1000));

        estimate.record_at(Class::Accept, at);
        assert_eq!(
            estimate.ring.read(at),
            counts(LIMIT / 2 + 1, 0, 500),
            "halved, then counted"
        );
        let word = estimate.ring.slots.0[(9 % SLOTS as u64) as usize].load(Ordering::Relaxed);
        assert_eq!(tag_of(word), 9, "the interval bits are never disturbed");
    }

    /// The retry reading `R - K * A` and the rate reading `R - K_t * (R - T)` against hand-computed
    /// cases.
    #[test]
    fn the_readings_are_what_the_arithmetic_says() {
        let at = SECOND;
        let open = |accepted, transient, overloaded| {
            let estimate = adaptive();
            record(&estimate, Class::Accept, accepted, at);
            record(&estimate, Class::Transient, transient, at);
            record(&estimate, Class::Overload, overloaded, at);
            estimate.reading_at(at)
        };

        // 600 failures against 6,600 accepts leave the retry reading under the slack; 7,000
        // against 5,000 do not.
        assert!(open(6_600, 600, 0).retries_open);
        assert!(!open(5_000, 7_000, 0).retries_open);
        // A lone failure on an idle channel is under the slack of 10, and not under a slack of 0.
        assert!(open(0, 1, 0).retries_open);
        let strict = Adaptive::new(AdaptiveConfig {
            slack: 0,
            ..AdaptiveConfig::default()
        });
        strict.record_at(Class::Transient, at);
        assert!(!strict.reading_at(at).retries_open);

        // 600 transient failures and no throttling leave the rate uncapped; 600 overloaded, the
        // opposite.
        assert_eq!(open(0, 600, 0).cap, None);
        assert!(open(0, 0, 600).cap.is_some());
        assert!(!open(0, 0, 600).retries_open, "and the retries stop too");
    }

    /// When every failure is overload and the multipliers are equal, the two readings are one
    /// statistic, on every run.
    #[test]
    fn when_every_failure_is_overload_the_two_readings_are_one() {
        for (accepted, rejected) in [(0u64, 11u64), (100, 40), (50, 60), (1000, 480), (1000, 520)] {
            let estimate = adaptive();
            record(&estimate, Class::Accept, accepted, SECOND);
            record(&estimate, Class::Overload, rejected, SECOND);
            let reading = estimate.reading_at(SECOND);
            assert_eq!(
                reading.retries_open,
                reading.cap.is_none(),
                "{accepted}/{rejected}"
            );
        }
    }

    /// The cap: none while the rate reading is under the slack, and then within the slack of the
    /// window's average send rate; never under the floor.
    #[test]
    fn the_cap_follows_the_sends_and_never_goes_under_the_floor() {
        let estimate = adaptive();
        let window = estimate.config.window;
        let age = window.as_nanos() as u64;

        // A window's worth of attempts at 60 a second, 1,800 of them: just under the slack of
        // throttling, nothing capped.
        let accepted = 1_800 - 29;
        record(&estimate, Class::Accept, accepted, age);
        record(&estimate, Class::Overload, 29, age);
        assert_eq!(
            estimate.reading_at(age).cap,
            None,
            "29 overloaded of 1800 is under the slack"
        );

        // More throttling than half the attempts, past the slack: capped.
        let capped = adaptive();
        record(&capped, Class::Accept, 600, age);
        record(&capped, Class::Overload, 1_200, age);
        let cap = capped.reading_at(age).cap.expect("capped");
        // K_t * (R - T) / W' = 2 * 600 / 30.
        assert!((cap - 40.0).abs() < 1e-9, "{cap}");

        // Just past the slack, the cap is the window's average send rate less S / W', to within
        // one count.
        let edge = adaptive();
        // R - 2 * (R - T) > 10 with R = 1000: T > 505.
        record(&edge, Class::Accept, 494, age);
        record(&edge, Class::Overload, 506, age);
        let cap = edge.reading_at(age).cap.expect("just past the slack");
        let seconds = window.as_secs_f64();
        let bound = 1_000.0 / seconds - 10.0 / seconds;
        assert!(
            cap < bound && bound - cap <= 2.0 / seconds + 1e-9,
            "{cap} against {bound}"
        );

        // Everything overloaded: the floor.
        let floor = adaptive();
        record(&floor, Class::Overload, 500, age);
        assert_eq!(floor.reading_at(age).cap, Some(0.5));
    }

    /// With `K_t` of 1 the channel caps on overload beyond the slack, whatever it does not
    /// overload on; a transient failure never moves the cap.
    #[test]
    fn the_throttle_multiplier_moves_the_cap_and_transient_failures_never_do() {
        let age = 30 * SECOND;
        let one = Adaptive::new(AdaptiveConfig {
            throttle_multiplier: 1.0,
            ..AdaptiveConfig::default()
        });
        record(&one, Class::Accept, 10_000, age);
        record(&one, Class::Overload, 11, age);
        assert!(
            one.reading_at(age).cap.is_some(),
            "11 throttles over a slack of 10"
        );
        record(&one, Class::Accept, 0, age);

        let transient = adaptive();
        record(&transient, Class::Transient, 5_000, age);
        assert_eq!(transient.reading_at(age).cap, None);
        assert!(!transient.reading_at(age).retries_open);
    }

    /// The research's figures, a transient outage at 60 calls a second: nothing trips at 3 s, the retries stop
    /// between 5 and 9 s, and the rate is never capped.
    #[test]
    fn a_transient_outage_stops_the_retries_and_never_caps_the_rate() {
        let estimate = adaptive();
        let mut stopped_at = None;
        let lambda = 60u64;
        // A healthy window first.
        for second in 0..30 {
            record(&estimate, Class::Accept, lambda, second * SECOND);
        }
        // Then the server is down: each second, 60 first attempts fail and, while the retries
        // pass, five times as many retries after them (n = 5, spread over the next seconds).
        let down = 30 * SECOND;
        let mut retries_open_at_3s = true;
        for tenth in 0..200u64 {
            let now = down + tenth * SECOND / 10;
            let open = estimate.reading_at(now);
            assert_eq!(open.cap, None, "the rate is never capped, at {tenth}");
            if tenth == 30 {
                retries_open_at_3s = open.retries_open;
            }
            if !open.retries_open && stopped_at.is_none() {
                stopped_at = Some(tenth);
            }
            // Six attempts a tenth of a second: the first attempts, and the retries while open.
            let attempts = if open.retries_open {
                6 * lambda / 10
            } else {
                lambda / 10
            };
            record(&estimate, Class::Transient, attempts, now);
        }
        assert!(retries_open_at_3s, "a 3 s outage changes nothing");
        let stopped = stopped_at.expect("the retries stop") as f64 / 10.0;
        assert!((3.0..=9.0).contains(&stopped), "they stop at {stopped} s");
    }

    /// The research's figures, an overloaded server that the policy does not retry: the cap starts about half a window
    /// on, reaches the floor by a window, and lifts within a window of the end.
    #[test]
    fn a_throttling_episode_caps_the_rate_from_about_half_a_window_and_lifts_after_it() {
        let estimate = adaptive();
        let lambda = 60u64;
        for second in 0..30 {
            record(&estimate, Class::Accept, lambda, second * SECOND);
        }
        let start = 30 * SECOND;
        let mut capped_at = None;
        let mut floor_at = None;
        for tenth in 0..400u64 {
            let now = start + tenth * SECOND / 10;
            let reading = estimate.reading_at(now);
            if reading.cap.is_some() && capped_at.is_none() {
                capped_at = Some(tenth);
            }
            if reading.cap == Some(0.5) && floor_at.is_none() {
                floor_at = Some(tenth);
            }
            // First attempts are all overloaded; those over the cap wait, so the overloaded rate
            // falls with the cap.
            let sent = reading.cap.map_or(lambda as f64 / 10.0, |cap| cap / 10.0);
            let sent = sent.round() as u64;
            record(&estimate, Class::Overload, sent, now);
        }
        let capped = capped_at.expect("capped") as f64 / 10.0;
        assert!(
            (12.5..=17.5).contains(&capped),
            "the cap starts at {capped} s"
        );
        let floor = floor_at.expect("the floor") as f64 / 10.0;
        assert!(
            (25.0..=40.0).contains(&floor),
            "the floor is reached at {floor} s"
        );

        // The server recovers: probes at the floor succeed, and the cap lifts within a window.
        let recovered = start + 40 * SECOND;
        let mut lifted_at = None;
        for tenth in 0..400u64 {
            let now = recovered + tenth * SECOND / 10;
            if estimate.reading_at(now).cap.is_none() {
                lifted_at = Some(tenth);
                break;
            }
            record(
                &estimate,
                Class::Accept,
                if tenth % 20 == 0 { 1 } else { 0 },
                now,
            );
        }
        let lifted = lifted_at.expect("lifted") as f64 / 10.0;
        assert!(lifted <= 30.0, "lifted {lifted} s after the end");
    }

    /// The cap's cell: a depth of one second of turns, a turn every `1 / cap`, and no debt kept
    /// once the rate is not capped.
    #[test]
    fn the_cell_spaces_turns_by_the_cap_and_forgets_its_debt_when_uncapped() {
        let estimate = adaptive();
        let now = 100 * SECOND;
        // At 2 a second the depth is 2 and a turn comes every half second.
        assert_eq!(estimate.take(now, 2.0), Ok(()));
        assert_eq!(estimate.take(now, 2.0), Ok(()));
        assert_eq!(estimate.take(now, 2.0), Err(SECOND / 2));
        assert_eq!(estimate.take(now + SECOND / 2, 2.0), Ok(()));

        // Uncapped: the first decision resets the debt.
        assert!(estimate.tat.0.load(Ordering::Relaxed) > now);
        assert_eq!(estimate.turn(now), Ok(()));
        assert_eq!(estimate.tat.0.load(Ordering::Relaxed), 0);
    }

    /// Records from eight threads into one slot, each count below its limit, all add up: the
    /// sum of the window is the number of records in it, whatever the interleaving.
    #[test]
    fn the_counts_of_eight_threads_add_up_exactly() {
        let estimate = std::sync::Arc::new(adaptive());
        let at = 7 * estimate.ring.interval;
        let threads: Vec<_> = (0..8)
            .map(|which| {
                let estimate = std::sync::Arc::clone(&estimate);
                std::thread::spawn(move || {
                    let class = match which % 3 {
                        0 => Class::Accept,
                        1 => Class::Transient,
                        _ => Class::Overload,
                    };
                    for _ in 0..5_000u64 {
                        estimate.record_at(class, at);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("a thread");
        }
        assert_eq!(estimate.ring.read(at), counts(15_000, 15_000, 10_000));
    }

    /// Records racing across an interval boundary lose none that is still in the window.
    #[test]
    fn records_across_an_interval_boundary_are_all_kept() {
        let estimate = std::sync::Arc::new(adaptive());
        let slot = estimate.ring.interval;
        let threads: Vec<_> = (0..8u64)
            .map(|which| {
                let estimate = std::sync::Arc::clone(&estimate);
                std::thread::spawn(move || {
                    for n in 0..2_000u64 {
                        // Half of the records are of one interval and half of the next.
                        estimate.record_at(Class::Accept, 9 * slot + (n + which) % 2 * slot);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("a thread");
        }
        assert_eq!(estimate.ring.read(10 * slot).accepted, 16_000);
    }

    #[test]
    fn a_configuration_out_of_bounds_is_refused() {
        let ok = AdaptiveConfig::default();
        assert!(ok.admissible().is_ok());
        for config in [
            AdaptiveConfig {
                multiplier: 0.9,
                ..ok.clone()
            },
            AdaptiveConfig {
                multiplier: 101.0,
                ..ok.clone()
            },
            AdaptiveConfig {
                multiplier: f64::NAN,
                ..ok.clone()
            },
            AdaptiveConfig {
                throttle_multiplier: 0.0,
                ..ok.clone()
            },
            AdaptiveConfig {
                slack: 1_000_001,
                ..ok.clone()
            },
            AdaptiveConfig {
                window: Duration::from_millis(11),
                ..ok.clone()
            },
            AdaptiveConfig {
                window: Duration::from_secs(601),
                ..ok.clone()
            },
            AdaptiveConfig {
                floor_per_second: 0.0,
                ..ok.clone()
            },
            AdaptiveConfig {
                floor_per_second: f64::INFINITY,
                ..ok.clone()
            },
        ] {
            assert!(config.admissible().is_err(), "{config:?}");
        }
    }
}
