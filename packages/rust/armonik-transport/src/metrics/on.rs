//! The registry of a build with the `metrics` feature.

use std::cell::Cell;
use std::sync::atomic::{
    AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering::Relaxed,
};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use super::{
    reset_slot, CloseReason, GaugeSource, HostEvent, Stats, CLOSE_SLOTS, RESET_SLOTS, RETRY_SLOTS,
    STATUS_SLOTS,
};
use crate::grpc::GrpcStatusCode;

/// How many locks the registry's totals are spread over.
const SHARDS: usize = 8;

/// A value on a cache line of its own, so that the counters of two writers never share one.
#[derive(Default)]
#[repr(align(64))]
struct Padded<T>(T);

/// A counter that one task writes: a load and a store, never a read-modify-write.
#[derive(Default)]
struct Counter(AtomicU64);

impl Counter {
    fn add(&self, n: u64) {
        self.0.store(self.0.load(Relaxed).wrapping_add(n), Relaxed);
    }

    fn get(&self) -> u64 {
        self.0.load(Relaxed)
    }
}

/// What a call's driver writes.
#[derive(Default)]
struct Driver {
    messages_received: Counter,
    window_waits: Counter,
    waiting_for_stream: AtomicU32,
}

/// What a call's request body writes, as the connection's task polls it.
#[derive(Default)]
struct Body {
    messages_sent: Counter,
    raw: Counter,
    sent: Counter,
}

#[derive(Default)]
struct CallBlock {
    driver: Padded<Driver>,
    body: Padded<Body>,
    /// The status the call ended with, plus one, or zero while it runs. Set once, by its guard.
    ended: AtomicU8,
}

impl CallBlock {
    fn add_to(&self, totals: &mut Stats) {
        totals.messages_received += self.driver.0.messages_received.get();
        totals.host_window_waits += self.driver.0.window_waits.get();
        totals.messages_sent += self.body.0.messages_sent.get();
        totals.message_bytes_raw += self.body.0.raw.get();
        totals.message_bytes_sent += self.body.0.sent.get();
        if let Some(status) = self.ended.load(Relaxed).checked_sub(1) {
            totals.calls_ended[usize::from(status)] += 1;
        }
    }
}

/// What a connection's task writes: the bytes it moves, and the frames it reads.
#[derive(Default)]
struct Io {
    sent: Counter,
    received: Counter,
    resets: [Counter; RESET_SLOTS],
    goaway: AtomicBool,
    eof: AtomicBool,
}

#[derive(Default)]
struct ConnBlock {
    io: Padded<Io>,
    /// Why the engine ended the session, plus one, or zero: written by whoever ends it.
    mark: Padded<AtomicU8>,
}

impl ConnBlock {
    fn add_to(&self, totals: &mut Stats) {
        totals.wire_bytes_sent += self.io.0.sent.get();
        totals.wire_bytes_received += self.io.0.received.get();
        for (total, reset) in totals.streams_reset.iter_mut().zip(&self.io.0.resets) {
            *total += reset.get();
        }
    }
}

impl Stats {
    /// Adds the gauges of `other` to these.
    fn absorb_gauges(&mut self, other: &Stats) {
        self.throttle_cap_per_second += other.throttle_cap_per_second;
        self.channels_capped += other.channels_capped;
        self.channels_retries_closed += other.channels_retries_closed;
        self.calls_waiting_at_cap += other.calls_waiting_at_cap;
        self.calls_waiting_for_stream += other.calls_waiting_for_stream;
    }

    /// Adds the counters of `other` to these.
    fn absorb(&mut self, other: &Stats) {
        fn add<const N: usize>(into: &mut [u64; N], from: &[u64; N]) {
            for (into, from) in into.iter_mut().zip(from) {
                *into += from;
            }
        }
        self.calls_started += other.calls_started;
        add(&mut self.calls_ended, &other.calls_ended);
        self.messages_sent += other.messages_sent;
        self.messages_received += other.messages_received;
        add(&mut self.retries, &other.retries);
        self.retries_refused += other.retries_refused;
        self.calls_not_replayable += other.calls_not_replayable;
        self.resends += other.resends;
        self.dials_tried += other.dials_tried;
        self.dials_succeeded += other.dials_succeeded;
        self.dials_failed += other.dials_failed;
        add(&mut self.connections_closed, &other.connections_closed);
        add(&mut self.streams_reset, &other.streams_reset);
        self.wire_bytes_sent += other.wire_bytes_sent;
        self.wire_bytes_received += other.wire_bytes_received;
        self.message_bytes_raw += other.message_bytes_raw;
        self.message_bytes_sent += other.message_bytes_sent;
        self.host_window_waits += other.host_window_waits;
        self.host_memory_waits += other.host_memory_waits;
        self.host_memory_refusals += other.host_memory_refusals;
    }
}

/// Blocks by the place they were given, which a slot reuses once its block has left.
struct Slab<T> {
    slots: Vec<Option<Arc<T>>>,
    free: Vec<usize>,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Slab<T> {
    fn insert(&mut self, block: Arc<T>) -> usize {
        match self.free.pop() {
            Some(slot) => {
                self.slots[slot] = Some(block);
                slot
            }
            None => {
                self.slots.push(Some(block));
                self.slots.len() - 1
            }
        }
    }

    fn take(&mut self, slot: usize) -> Option<Arc<T>> {
        let block = self.slots.get_mut(slot)?.take()?;
        self.free.push(slot);
        Some(block)
    }

    fn live(&self) -> impl Iterator<Item = &Arc<T>> {
        self.slots.iter().flatten()
    }
}

#[derive(Default)]
struct Shard {
    live: Slab<CallBlock>,
    totals: Stats,
}

#[derive(Default)]
struct Connections {
    live: Slab<ConnBlock>,
    totals: Stats,
}

struct Registry {
    shards: [Padded<Mutex<Shard>>; SHARDS],
    connections: Mutex<Connections>,
    gauges: Mutex<Vec<Weak<dyn GaugeSource>>>,
    /// The registries this one reads besides its own, which live as long as it does.
    children: Mutex<Vec<Metrics>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The shard of the calling thread, drawn once on its first count.
fn shard_of_this_thread() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static SHARD: Cell<Option<usize>> = const { Cell::new(None) };
    }
    SHARD.with(|shard| match shard.get() {
        Some(shard) => shard,
        None => {
            let drawn = NEXT.fetch_add(1, Relaxed) % SHARDS;
            shard.set(Some(drawn));
            drawn
        }
    })
}

/// The registry of calls, connections and channels, and the totals the finished ones left.
///
/// Shared by every channel built with the same handle, which is what reads them as one.
#[derive(Clone)]
pub struct Metrics(Arc<Registry>);

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self(Arc::new(Registry {
            shards: std::array::from_fn(|_| Padded(Mutex::new(Shard::default()))),
            connections: Mutex::new(Connections::default()),
            gauges: Mutex::new(Vec::new()),
            children: Mutex::new(Vec::new()),
        }))
    }

    /// A registry of its own that this one reads as well: `stats()` of this is the sum of its own
    /// counts and its children's, and `stats()` of the child is the child's alone. A child counts
    /// for as long as its parent exists, so that what a closed channel counted stays in the sum.
    pub fn child(&self) -> Self {
        let child = Self::new();
        locked(&self.0.children).push(child.clone());
        child
    }

    /// The counters and gauges, now: the totals the finished calls and connections left, and the
    /// counters of those still live, a shard at a time under its lock, so that a call is in the
    /// sum once.
    pub fn stats(&self) -> Stats {
        let mut stats = Stats {
            counting: true,
            ..Stats::default()
        };
        for shard in &self.0.shards {
            let shard = locked(&shard.0);
            stats.absorb(&shard.totals);
            for block in shard.live.live() {
                block.add_to(&mut stats);
                stats.calls_waiting_for_stream +=
                    u64::from(block.driver.0.waiting_for_stream.load(Relaxed));
            }
        }
        {
            let connections = locked(&self.0.connections);
            stats.absorb(&connections.totals);
            for block in connections.live.live() {
                block.add_to(&mut stats);
            }
        }
        // The sources are read outside the registry's own locks: a channel takes its own.
        let sources: Vec<_> = {
            let mut sources = locked(&self.0.gauges);
            sources.retain(|source| source.strong_count() > 0);
            sources.iter().filter_map(Weak::upgrade).collect()
        };
        for source in sources {
            let gauges = source.gauges();
            if let Some(cap) = gauges.cap {
                stats.throttle_cap_per_second += cap;
                stats.channels_capped += 1;
            }
            stats.channels_retries_closed += u64::from(!gauges.retries_open);
            stats.calls_waiting_at_cap += gauges.waiting_at_cap;
        }
        // Outside the registry's own locks, as the sources are: a child takes its own.
        let children = locked(&self.0.children).clone();
        for child in &children {
            let read = child.stats();
            stats.absorb(&read);
            stats.absorb_gauges(&read);
        }
        stats
    }

    /// Counts what the embedding crate reports.
    pub fn count_host(&self, event: HostEvent) {
        self.event(|totals| match event {
            HostEvent::WindowWait => totals.host_window_waits += 1,
            HostEvent::MemoryWait => totals.host_memory_waits += 1,
            HostEvent::MemoryRefusal => totals.host_memory_refusals += 1,
        });
    }

    /// Adds to the totals of the calling thread's shard.
    fn event(&self, add: impl FnOnce(&mut Stats)) {
        add(&mut locked(&self.0.shards[shard_of_this_thread()].0).totals);
    }

    pub(crate) fn watch(&self, source: Weak<dyn GaugeSource>) {
        let mut sources = locked(&self.0.gauges);
        sources.retain(|source| source.strong_count() > 0);
        sources.push(source);
    }

    /// Puts a call in the registry, where it stays until the last holder of its counters lets go.
    pub(crate) fn start_call(&self, counters: &CallCounters) -> CallGuard {
        // A call is registered once: counters already in the registry are not entered again by a
        // repeat that follows the first. Starting one call's counters from two threads at once is
        // not supported.
        if counters.fold.home.get().is_some() {
            return CallGuard(counters.clone());
        }
        let shard = shard_of_this_thread();
        {
            let mut guarded = locked(&self.0.shards[shard].0);
            guarded.totals.calls_started += 1;
            let slot = guarded.live.insert(Arc::clone(&counters.block));
            let home = Home {
                registry: Arc::clone(&self.0),
                shard,
                slot,
            };
            let _ = counters.fold.home.set(home);
        }
        CallGuard(counters.clone())
    }

    pub(crate) fn retry(&self, slot: impl FnOnce() -> usize) {
        self.event(|totals| totals.retries[slot().min(RETRY_SLOTS - 1)] += 1);
    }

    pub(crate) fn retry_refused(&self) {
        self.event(|totals| totals.retries_refused += 1);
    }

    pub(crate) fn not_replayable(&self) {
        self.event(|totals| totals.calls_not_replayable += 1);
    }

    pub(crate) fn resend(&self) {
        self.event(|totals| totals.resends += 1);
    }

    pub(crate) fn dial_tried(&self) {
        locked(&self.0.connections).totals.dials_tried += 1;
    }

    /// A dial that ended with no connection, or with one it dropped unused: what its handshake
    /// moved is counted.
    pub(crate) fn dial_failed(&self, conn: &ConnCounters) {
        let mut connections = locked(&self.0.connections);
        connections.totals.dials_failed += 1;
        conn.0.add_to(&mut connections.totals);
    }

    /// A dial that succeeded: the connection is in the registry until its guard ends it.
    pub(crate) fn connection_open(&self, conn: &ConnCounters) -> ConnGuard {
        let slot = {
            let mut connections = locked(&self.0.connections);
            connections.totals.dials_succeeded += 1;
            connections.live.insert(Arc::clone(&conn.0))
        };
        ConnGuard {
            registry: Some(Arc::clone(&self.0)),
            slot,
        }
    }
}

/// A call's counters, shared by the tasks that write them.
///
/// A task may count after the call has ended, as the tail of a request body or the delivery of
/// the last messages to a host do, so the block stays in the registry, summed live, until the last
/// task that holds the counters lets go; only then are they added to the shard's totals.
#[derive(Clone, Debug)]
pub(crate) struct CallCounters {
    block: Arc<CallBlock>,
    fold: Arc<Fold>,
}

impl std::fmt::Debug for CallBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallBlock").finish_non_exhaustive()
    }
}

impl Default for CallCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a started call's block lives in the registry.
struct Home {
    registry: Arc<Registry>,
    shard: usize,
    slot: usize,
}

/// Moves a call's block from the live ones to the totals of its shard, when the last holder of its
/// counters is gone.
struct Fold {
    home: OnceLock<Home>,
}

impl std::fmt::Debug for Fold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fold").finish_non_exhaustive()
    }
}

impl Drop for Fold {
    fn drop(&mut self) {
        let Some(home) = self.home.get() else {
            return;
        };
        let mut shard = locked(&home.registry.shards[home.shard].0);
        if let Some(block) = shard.live.take(home.slot) {
            block.add_to(&mut shard.totals);
        }
    }
}

impl CallCounters {
    pub(crate) fn new() -> Self {
        Self {
            block: Arc::default(),
            fold: Arc::new(Fold {
                home: OnceLock::new(),
            }),
        }
    }

    pub(crate) fn message_received(&self) {
        self.block.driver.0.messages_received.add(1);
    }

    /// A message taken from the call's request stream, `raw` bytes as the caller wrote it and
    /// `sent` as the engine sends it.
    pub(crate) fn message_sent(&self, raw: usize, sent: usize) {
        let body = &self.block.body.0;
        body.messages_sent.add(1);
        body.raw.add(raw as u64);
        body.sent.add(sent as u64);
    }

    pub(crate) fn window_wait(&self) {
        self.block.driver.0.window_waits.add(1);
    }

    /// The call waits for a session to open or to have room, until the guard is dropped.
    pub(crate) fn wait_for_stream(&self) -> StreamWait<'_> {
        self.block.driver.0.waiting_for_stream.store(1, Relaxed);
        StreamWait(self)
    }
}

pub(crate) struct StreamWait<'a>(&'a CallCounters);

impl Drop for StreamWait<'_> {
    fn drop(&mut self) {
        self.0.block.driver.0.waiting_for_stream.store(0, Relaxed);
    }
}

/// A call's place in the registry. Ended with the status the call ends with, or CANCELLED when it
/// is dropped first, which is what a call whose driver never ran, or was dropped, is.
pub(crate) struct CallGuard(CallCounters);

impl CallGuard {
    pub(crate) fn end(self, code: GrpcStatusCode) {
        self.set(code);
    }

    /// The first status stands.
    fn set(&self, code: GrpcStatusCode) {
        let status = (code as usize).min(STATUS_SLOTS - 1) as u8 + 1;
        let _ = self
            .0
            .block
            .ended
            .compare_exchange(0, status, Relaxed, Relaxed);
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.set(GrpcStatusCode::Cancelled);
    }
}

/// A connection's counters, shared by its connection task and whoever ends its session.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConnCounters(Arc<ConnBlock>);

impl std::fmt::Debug for ConnBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnBlock").finish_non_exhaustive()
    }
}

impl ConnCounters {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether this build reads what a connection carries.
    pub(crate) const OBSERVES: bool = true;

    /// Records why the engine is ending the session, before it lets go of it. The first stands.
    pub(crate) fn mark(&self, reason: CloseReason) {
        let _ = self
            .0
            .mark
            .0
            .compare_exchange(0, reason as u8 + 1, Relaxed, Relaxed);
    }

    pub(crate) fn bytes_written(&self, bytes: usize) {
        self.0.io.0.sent.add(bytes as u64);
    }

    pub(crate) fn bytes_read(&self, bytes: usize) {
        self.0.io.0.received.add(bytes as u64);
    }

    /// The read of the peer's end of the stream.
    pub(crate) fn eof(&self) {
        self.0.io.0.eof.store(true, Relaxed);
    }

    fn reset(&self, code: u32) {
        self.0.io.0.resets[reset_slot(code)].add(1);
    }

    fn goaway(&self) {
        self.0.io.0.goaway.store(true, Relaxed);
    }

    fn marked(&self) -> Option<CloseReason> {
        match self.0.mark.0.load(Relaxed) {
            0 => None,
            marked => [
                CloseReason::GoAway,
                CloseReason::KeepaliveTimeout,
                CloseReason::IdleTimeout,
                CloseReason::IoError,
                CloseReason::LocalClose,
                CloseReason::PeerClosed,
                CloseReason::ProtocolError,
                CloseReason::Other,
            ]
            .get(usize::from(marked - 1))
            .copied(),
        }
    }

    /// Why a session ended, in the order the engine's design gives: what the engine did, a GOAWAY
    /// read, hyper's keepalive timeout, the peer's end of the stream, an I/O error, any other
    /// error, and a clean end that none of those explains.
    pub(crate) fn classify(&self, ended: &Result<(), hyper::Error>) -> CloseReason {
        if let Some(reason) = self.marked() {
            return reason;
        }
        if self.0.io.0.goaway.load(Relaxed) {
            return CloseReason::GoAway;
        }
        if ended.as_ref().is_err_and(hyper::Error::is_timeout) {
            return CloseReason::KeepaliveTimeout;
        }
        if self.0.io.0.eof.load(Relaxed) {
            return CloseReason::PeerClosed;
        }
        match ended {
            Err(error) if has_io_cause(error) => CloseReason::IoError,
            Err(_) => CloseReason::ProtocolError,
            Ok(()) => CloseReason::Other,
        }
    }
}

fn has_io_cause(error: &(dyn std::error::Error + 'static)) -> bool {
    std::iter::successors(Some(error), |error| error.source())
        .any(|error| error.is::<std::io::Error>())
}

/// A connection's place in the registry, ended with the reason its session did, or as the engine's
/// own close when it is dropped first.
pub(crate) struct ConnGuard {
    registry: Option<Arc<Registry>>,
    slot: usize,
}

impl ConnGuard {
    pub(crate) fn end(mut self, reason: CloseReason) {
        self.finish(reason);
    }

    fn finish(&mut self, reason: CloseReason) {
        let Some(registry) = self.registry.take() else {
            return;
        };
        let mut connections = locked(&registry.connections);
        if let Some(block) = connections.live.take(self.slot) {
            block.add_to(&mut connections.totals);
        }
        connections.totals.connections_closed[(reason as usize).min(CLOSE_SLOTS - 1)] += 1;
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.finish(CloseReason::LocalClose);
    }
}

/// Follows the HTTP/2 frames in the bytes a connection reads, which arrive in chunks that cut
/// them anywhere: a frame is a header of nine bytes, its length in the first three and its type in
/// the fourth, and a payload.
#[derive(Debug, Default)]
pub(crate) struct Sniffer {
    header: [u8; 9],
    header_len: usize,
    /// The bytes of the current frame's payload still to come.
    remaining: usize,
    kind: u8,
    /// The first bytes of the payload of a frame whose error code is wanted.
    code: [u8; 8],
    code_len: usize,
    code_wanted: usize,
}

const RST_STREAM: u8 = 3;
const GOAWAY: u8 = 7;

impl Sniffer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn feed(&mut self, mut data: &[u8], conn: &ConnCounters) {
        while !data.is_empty() {
            if self.header_len < self.header.len() {
                let taken = (self.header.len() - self.header_len).min(data.len());
                self.header[self.header_len..self.header_len + taken]
                    .copy_from_slice(&data[..taken]);
                self.header_len += taken;
                data = &data[taken..];
                if self.header_len == self.header.len() {
                    let [a, b, c, kind, ..] = self.header;
                    self.remaining =
                        (usize::from(a) << 16) | (usize::from(b) << 8) | usize::from(c);
                    self.kind = kind;
                    self.code_len = 0;
                    self.code_wanted = match kind {
                        RST_STREAM => 4.min(self.remaining),
                        GOAWAY => 8.min(self.remaining),
                        _ => 0,
                    };
                    if self.remaining == 0 {
                        self.end_frame(conn);
                    }
                }
                continue;
            }
            let taken = self.remaining.min(data.len());
            if self.code_len < self.code_wanted {
                let wanted = (self.code_wanted - self.code_len).min(taken);
                self.code[self.code_len..self.code_len + wanted].copy_from_slice(&data[..wanted]);
                self.code_len += wanted;
            }
            self.remaining -= taken;
            data = &data[taken..];
            if self.remaining == 0 {
                self.end_frame(conn);
            }
        }
    }

    fn end_frame(&mut self, conn: &ConnCounters) {
        self.header_len = 0;
        match self.kind {
            RST_STREAM if self.code_len == 4 => {
                let [a, b, c, d, ..] = self.code;
                conn.reset(u32::from_be_bytes([a, b, c, d]));
            }
            GOAWAY if self.code_len == 8 => conn.goaway(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
        let len = payload.len() as u32;
        let mut frame = vec![
            (len >> 16) as u8,
            (len >> 8) as u8,
            len as u8,
            kind,
            0,
            0,
            0,
            0,
            1,
        ];
        frame.extend_from_slice(payload);
        frame
    }

    fn resets(conn: &ConnCounters) -> [u64; RESET_SLOTS] {
        let mut stats = Stats::default();
        conn.0.add_to(&mut stats);
        stats.streams_reset
    }

    /// Wherever the chunks cut the stream, the same frames are read.
    #[test]
    fn the_frames_are_read_whatever_the_chunks_cut() {
        let mut stream = Vec::new();
        stream.extend(frame(4, &[0; 12]));
        stream.extend(frame(0, &[7; 300]));
        stream.extend(frame(RST_STREAM, &11u32.to_be_bytes()));
        stream.extend(frame(RST_STREAM, &8u32.to_be_bytes()));
        stream.extend(frame(RST_STREAM, &99u32.to_be_bytes()));
        stream.extend(frame(6, &[0; 8]));

        for chunk in [1, 2, 3, 7, 9, 10, 64, stream.len()] {
            let conn = ConnCounters::default();
            let mut sniffer = Sniffer::default();
            for part in stream.chunks(chunk) {
                sniffer.feed(part, &conn);
            }
            let mut expected = [0; RESET_SLOTS];
            expected[11] = 1;
            expected[8] = 1;
            expected[RESET_SLOTS - 1] = 1;
            assert_eq!(resets(&conn), expected, "chunks of {chunk}");
            assert!(!conn.0.io.0.goaway.load(Relaxed), "chunks of {chunk}");
        }
    }

    #[test]
    fn a_goaway_is_read_by_its_error_code_being_there() {
        let mut payload = 5u32.to_be_bytes().to_vec();
        payload.extend(2u32.to_be_bytes());
        payload.extend(b"debug data");
        for chunk in [1, 4, 100] {
            let conn = ConnCounters::default();
            let mut sniffer = Sniffer::default();
            for part in frame(GOAWAY, &payload).chunks(chunk) {
                sniffer.feed(part, &conn);
            }
            assert!(conn.0.io.0.goaway.load(Relaxed), "chunks of {chunk}");
        }
    }

    #[test]
    fn a_frame_with_no_payload_ends_at_its_header() {
        let conn = ConnCounters::default();
        let mut sniffer = Sniffer::default();
        sniffer.feed(&frame(4, &[]), &conn);
        sniffer.feed(&frame(RST_STREAM, &1u32.to_be_bytes()), &conn);
        assert_eq!(resets(&conn)[1], 1);
    }

    #[test]
    fn a_session_ends_for_the_first_reason_that_holds() {
        let conn = ConnCounters::default();
        conn.eof();
        assert_eq!(conn.classify(&Ok(())), CloseReason::PeerClosed);
        conn.goaway();
        assert_eq!(conn.classify(&Ok(())), CloseReason::GoAway);
        conn.mark(CloseReason::IdleTimeout);
        conn.mark(CloseReason::LocalClose);
        assert_eq!(conn.classify(&Ok(())), CloseReason::IdleTimeout);

        assert_eq!(
            ConnCounters::default().classify(&Ok(())),
            CloseReason::Other
        );
    }

    #[test]
    fn a_call_is_counted_once_live_or_finished() {
        let metrics = Metrics::new();
        let counters = CallCounters::default();
        let guard = metrics.start_call(&counters);
        counters.message_sent(10, 4);
        counters.message_received();

        let live = metrics.stats();
        assert_eq!(live.calls_started, 1);
        assert_eq!(live.calls_ended, [0; STATUS_SLOTS]);
        assert_eq!((live.messages_sent, live.message_bytes_raw), (1, 10));
        assert_eq!((live.messages_received, live.message_bytes_sent), (1, 4));

        guard.end(GrpcStatusCode::Unavailable);
        let done = metrics.stats();
        assert_eq!(done.calls_started, 1);
        assert_eq!(done.ended_with(GrpcStatusCode::Unavailable), 1);
        assert_eq!((done.messages_sent, done.messages_received), (1, 1));
    }

    #[test]
    fn a_call_dropped_before_it_ends_ends_cancelled() {
        let metrics = Metrics::new();
        drop(metrics.start_call(&CallCounters::default()));
        assert_eq!(metrics.stats().ended_with(GrpcStatusCode::Cancelled), 1);
    }

    fn live_calls(metrics: &Metrics) -> usize {
        let live = |shard: &Padded<Mutex<Shard>>| locked(&shard.0).live.live().count();
        metrics.0.shards.iter().map(live).sum()
    }

    /// What a call counts after its driver has ended it, as the tail of its request body and the
    /// delivery of its last messages do, is in the sum while the call has holders and in the
    /// totals after.
    #[test]
    fn a_count_after_the_call_ended_is_not_lost() {
        let metrics = Metrics::new();
        let counters = CallCounters::default();
        let guard = metrics.start_call(&counters);
        counters.message_sent(10, 4);
        guard.end(GrpcStatusCode::Ok);

        counters.message_sent(6, 6);
        counters.message_received();
        counters.window_wait();
        counters.window_wait();

        let tally = |stats: &Stats| {
            (
                stats.ended_with(GrpcStatusCode::Ok),
                stats.messages_sent,
                (stats.message_bytes_raw, stats.message_bytes_sent),
                stats.messages_received,
                stats.host_window_waits,
            )
        };
        let after_the_end = metrics.stats();
        assert_eq!(tally(&after_the_end), (1, 2, (16, 10), 1, 2));
        assert_eq!(after_the_end.calls_started, 1);
        assert_eq!(live_calls(&metrics), 1, "its counters still have a holder");

        drop(counters);
        let folded = metrics.stats();
        assert_eq!(tally(&folded), (1, 2, (16, 10), 1, 2));
        assert_eq!(folded.calls_ended.iter().sum::<u64>(), 1);
        assert_eq!(live_calls(&metrics), 0, "the last holder folded it");
    }

    #[test]
    fn counters_already_in_the_registry_are_not_entered_again() {
        let metrics = Metrics::new();
        let counters = CallCounters::default();
        let first = metrics.start_call(&counters);
        let second = metrics.start_call(&counters);
        assert_eq!(metrics.stats().calls_started, 1);
        assert_eq!(live_calls(&metrics), 1);

        first.end(GrpcStatusCode::Ok);
        drop(second);
        let stats = metrics.stats();
        assert_eq!(
            stats.ended_with(GrpcStatusCode::Ok),
            1,
            "the first status stands"
        );
        assert_eq!(stats.calls_ended.iter().sum::<u64>(), 1);
    }

    /// Whichever of the guard and the holders of the counters goes first, each count is in the
    /// totals once.
    #[test]
    fn a_call_is_folded_once_whatever_the_order_of_its_holders() {
        let metrics = Metrics::new();

        let counters = CallCounters::default();
        let guard = metrics.start_call(&counters);
        counters.message_sent(1, 1);
        drop(counters);
        guard.end(GrpcStatusCode::Unavailable);

        let counters = CallCounters::default();
        let held = counters.clone();
        drop(metrics.start_call(&counters));
        held.message_sent(2, 2);
        drop(counters);
        held.window_wait();
        drop(held);

        let stats = metrics.stats();
        assert_eq!(stats.calls_started, 2);
        assert_eq!(stats.ended_with(GrpcStatusCode::Unavailable), 1);
        assert_eq!(stats.ended_with(GrpcStatusCode::Cancelled), 1);
        assert_eq!((stats.messages_sent, stats.message_bytes_raw), (2, 3));
        assert_eq!(stats.host_window_waits, 1);
        assert_eq!(live_calls(&metrics), 0);
    }

    /// A task that goes on counting while its call is ended and read is counted exactly, and a
    /// counter is never seen to go back.
    #[test]
    fn a_writer_that_outlives_the_end_of_its_call_is_counted_exactly() {
        const BEFORE: u64 = 1000;
        const AFTER: u64 = 50_000;
        let metrics = Metrics::new();
        let counters = CallCounters::default();
        let guard = metrics.start_call(&counters);
        let (half_done, half) = std::sync::mpsc::channel();
        let (ended, ending) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            for _ in 0..BEFORE {
                counters.message_sent(1, 1);
            }
            half_done.send(()).expect("the call is ended");
            ending.recv().expect("the call ended");
            for _ in 0..AFTER {
                counters.message_sent(1, 1);
            }
        });
        half.recv().expect("half is written");
        guard.end(GrpcStatusCode::Ok);
        ended.send(()).expect("the writer waits");

        let mut seen = 0;
        while !writer.is_finished() {
            let sent = metrics.stats().messages_sent;
            assert!(sent >= seen, "a counter went back from {seen} to {sent}");
            seen = sent;
        }
        writer.join().expect("the writer finished");

        let stats = metrics.stats();
        assert_eq!(stats.messages_sent, BEFORE + AFTER);
        assert_eq!(stats.message_bytes_sent, BEFORE + AFTER);
        assert_eq!(stats.ended_with(GrpcStatusCode::Ok), 1);
        assert_eq!(live_calls(&metrics), 0);
    }

    #[test]
    fn a_parent_reads_its_children_and_a_child_reads_itself() {
        let parent = Metrics::new();
        let (one, two) = (parent.child(), parent.child());
        parent.count_host(HostEvent::MemoryWait);
        one.dial_tried();
        one.dial_tried();
        two.dial_tried();

        assert_eq!(one.stats().dials_tried, 2);
        assert_eq!(two.stats().dials_tried, 1);
        assert_eq!(one.stats().host_memory_waits, 0, "the parent's own");
        let all = parent.stats();
        assert_eq!((all.dials_tried, all.host_memory_waits), (3, 1));

        drop((one, two));
        assert_eq!(parent.stats().dials_tried, 3, "a child stays counted");
    }

    #[test]
    fn a_connection_that_opens_closes_once() {
        let metrics = Metrics::new();
        let conn = ConnCounters::default();
        metrics.dial_tried();
        let guard = metrics.connection_open(&conn);
        conn.bytes_written(5);
        conn.bytes_read(7);
        let open = metrics.stats();
        assert_eq!((open.dials_tried, open.dials_succeeded), (1, 1));
        assert_eq!((open.wire_bytes_sent, open.wire_bytes_received), (5, 7));
        assert_eq!(open.connections_closed, [0; CLOSE_SLOTS]);

        guard.end(CloseReason::IdleTimeout);
        let closed = metrics.stats();
        assert_eq!(
            closed.connections_closed[CloseReason::IdleTimeout as usize],
            1
        );
        assert_eq!((closed.wire_bytes_sent, closed.wire_bytes_received), (5, 7));

        drop(metrics.connection_open(&ConnCounters::default()));
        assert_eq!(
            metrics.stats().connections_closed[CloseReason::LocalClose as usize],
            1
        );
    }

    #[test]
    fn a_wait_for_a_stream_counts_while_it_is_held() {
        let metrics = Metrics::new();
        let counters = CallCounters::default();
        let _guard = metrics.start_call(&counters);
        assert_eq!(metrics.stats().calls_waiting_for_stream, 0);
        {
            let _wait = counters.wait_for_stream();
            assert_eq!(metrics.stats().calls_waiting_for_stream, 1);
        }
        assert_eq!(metrics.stats().calls_waiting_for_stream, 0);
    }

    /// Threads that start, count and end calls together leave sums that add up exactly.
    #[test]
    fn the_sums_of_many_threads_add_up() {
        const THREADS: usize = 16;
        const CALLS: u64 = 2000;
        let metrics = Metrics::new();
        let reader = metrics.clone();
        let reading = std::thread::spawn(move || {
            // Read while the others write: a call is never in the sum twice or not at all.
            for _ in 0..200 {
                let stats = reader.stats();
                let ended: u64 = stats.calls_ended.iter().sum();
                assert!(ended <= stats.calls_started);
                assert!(stats.messages_sent <= stats.calls_started * 3);
            }
        });
        let writers: Vec<_> = (0..THREADS)
            .map(|_| {
                let metrics = metrics.clone();
                std::thread::spawn(move || {
                    for _ in 0..CALLS {
                        let counters = CallCounters::default();
                        let guard = metrics.start_call(&counters);
                        for _ in 0..3 {
                            counters.message_sent(8, 8);
                        }
                        metrics.resend();
                        guard.end(GrpcStatusCode::Ok);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("a writer finished");
        }
        reading.join().expect("the reader finished");

        let total = THREADS as u64 * CALLS;
        let stats = metrics.stats();
        assert_eq!(stats.calls_started, total);
        assert_eq!(stats.ended_with(GrpcStatusCode::Ok), total);
        assert_eq!(stats.messages_sent, 3 * total);
        assert_eq!(stats.message_bytes_raw, 24 * total);
        assert_eq!(stats.resends, total);
    }
}
