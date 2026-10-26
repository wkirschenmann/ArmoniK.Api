//! A connection's writes, held while the work already ready on the runtime adds to them.

use std::future::Future;
use std::io;
use std::pin::{pin, Pin};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Wake, Waker};

use hyper::rt::{Read, ReadBuf, ReadBufCursor, Write};

use crate::metrics::{ConnCounters, Sniffer};

/// Holds each write for as long as a scheduler round adds bytes to it, up to `limit` bytes.
///
/// hyper's HTTP/2 writes what it has buffered each time its connection task is polled, so a frame
/// that a task polled after it makes ready goes in a write of its own: a request's message, handed
/// over from another thread while its headers wait, follows them in a second write. Holding the
/// write behind the work already ready lets such frames join it. A round that adds bytes earns
/// another; one that adds none, or a buffer at the limit, sends it. A limit of 0 holds nothing.
///
/// Under a [`Gate`], the connection's task is polled once at the end of a round its write waits,
/// rather than each time a task of the round hands it a frame. That poll takes in the frames the
/// round brought, and the write goes with them. A round in which no task woke the connection, and
/// whose end adds bytes to the write, earns another.
pub(crate) struct Coalescing<T> {
    io: T,
    limit: usize,
    state: State,
    round: Arc<Round>,
    /// Wakes the connection's task at the end of a round, through `round`. One waker for the
    /// connection's life, so that the runtime keeps one deferred wake-up for a round however many
    /// times the write is held in it.
    round_end: Waker,
    /// What the connection moves and reads, counted here because this is the one place that sees
    /// every byte of it.
    conn: ConnCounters,
    sniffer: Sniffer,
}

enum State {
    /// The next write is a new one.
    Open,
    /// A write of this length waits a round.
    Held(usize),
    /// A write goes, and what is left of it after a part or a full socket goes too, unheld.
    Sending,
}

/// The round a held write waits, shared by the write and the [`Gate`] over its connection's task.
#[derive(Default)]
pub(crate) struct Round {
    /// The gate is polling the connection: a write held now is its connection's.
    polling: AtomicBool,
    /// A write held under the gate waits for the end of the round.
    waiting: AtomicBool,
    /// The gate passed over a wake-up of the connection's task during the round.
    passed: AtomicBool,
    task: Mutex<Option<Waker>>,
}

impl Round {
    /// Defers the wake-up of the task `cx` polls to the end of the round.
    fn wait(&self, round_end: &Waker, cx: &Context<'_>) {
        {
            let mut task = self.task.lock().unwrap_or_else(PoisonError::into_inner);
            if !task.as_ref().is_some_and(|task| task.will_wake(cx.waker())) {
                *task = Some(cx.waker().clone());
            }
        }
        self.passed.store(false, Ordering::Relaxed);
        if self.polling.load(Ordering::Relaxed) {
            self.waiting.store(true, Ordering::Relaxed);
        }
        // Polled once and dropped: that queues the wake-up behind the tasks already ready,
        // including those woken from another thread, whose frames the next attempt brings.
        let _ = pin!(tokio::task::yield_now()).poll(&mut Context::from_waker(round_end));
    }

    /// Whether the poll that ends the round took in the frames the round brought: the gate passed
    /// over a wake-up during it.
    fn took_the_round_in(&self) -> bool {
        self.passed.swap(false, Ordering::Relaxed)
    }
}

/// The end of a round.
impl Wake for Round {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.waiting.store(false, Ordering::Relaxed);
        if let Some(task) = &*self.task.lock().unwrap_or_else(PoisonError::into_inner) {
            task.wake_by_ref();
        }
    }
}

/// A connection's task, not polled while its write waits for the end of a round.
///
/// Each task that hands the connection a frame wakes it, and a poll of hyper's connection costs
/// much the same whether it takes in one frame or many. Passing over the wake-ups of a round
/// leaves one poll, at its end, which [`Round`]'s deferred wake-up makes. A gate over any other
/// task sees no write held while it polls, and passes nothing over.
pub(crate) struct Gate<F> {
    task: F,
    round: Option<Arc<Round>>,
}

impl<F: Future> Future for Gate<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        // SAFETY: `task` is pinned with the gate, which never moves it and has no `Drop`.
        let (task, round) = unsafe {
            let this = self.get_unchecked_mut();
            (Pin::new_unchecked(&mut this.task), &this.round)
        };
        let Some(round) = round else {
            return task.poll(cx);
        };
        if round.waiting.load(Ordering::Relaxed) {
            round.passed.store(true, Ordering::Relaxed);
            return Poll::Pending;
        }
        round.polling.store(true, Ordering::Relaxed);
        let polled = task.poll(cx);
        round.polling.store(false, Ordering::Relaxed);
        polled
    }
}

/// The executor hyper spawns a connection's tasks on, which puts the first of them under the
/// gate of the connection's rounds: hyper's handshake spawns the task that drives the connection
/// before it hands back the sender any request goes through.
#[derive(Clone)]
pub(crate) struct Gating<E> {
    executor: E,
    round: Arc<Round>,
    gated: Arc<AtomicBool>,
}

impl<E> Gating<E> {
    pub(crate) fn new(executor: E, round: Arc<Round>) -> Self {
        Self {
            executor,
            round,
            gated: Arc::default(),
        }
    }
}

impl<E, F> hyper::rt::Executor<F> for Gating<E>
where
    E: hyper::rt::Executor<Gate<F>>,
{
    fn execute(&self, task: F) {
        let first = !self.gated.swap(true, Ordering::Relaxed);
        self.executor.execute(Gate {
            task,
            round: first.then(|| Arc::clone(&self.round)),
        });
    }
}

impl<T> Coalescing<T> {
    pub(crate) fn new(io: T, limit: usize, round: Arc<Round>, conn: ConnCounters) -> Self {
        Self {
            io,
            limit,
            state: State::Open,
            round_end: Waker::from(Arc::clone(&round)),
            round,
            conn,
            sniffer: Sniffer::new(),
        }
    }

    /// Whether a write of `len` bytes waits a round, in which case the task that holds it is woken
    /// at the round's end.
    fn hold(&mut self, len: usize, cx: &mut Context<'_>) -> bool {
        match self.state {
            State::Sending => return false,
            State::Held(before) if len <= before || self.round.took_the_round_in() => {
                self.state = State::Sending;
                return false;
            }
            State::Open | State::Held(_) => {}
        }
        if len >= self.limit {
            self.state = State::Sending;
            return false;
        }
        #[cfg(feature = "test-hooks")]
        crate::hooks::count_held_write();
        self.state = State::Held(len);
        self.round.wait(&self.round_end, cx);
        true
    }

    /// Opens the next write once this one has gone whole, or failed.
    fn sent(&mut self, len: usize, written: Poll<io::Result<usize>>) -> Poll<io::Result<usize>> {
        match written {
            Poll::Ready(Ok(n)) if n < len => {}
            Poll::Ready(_) => self.state = State::Open,
            Poll::Pending => {}
        }
        if let Poll::Ready(Ok(bytes)) = written {
            self.conn.bytes_written(bytes);
            #[cfg(feature = "test-hooks")]
            crate::hooks::count_write(bytes);
        }
        written
    }
}

impl<T: Read + Unpin> Read for Coalescing<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !ConnCounters::OBSERVES {
            return Pin::new(&mut this.io).poll_read(cx, buf);
        }

        // The cursor lends no view of what a read filled, so the read goes into a buffer over
        // the cursor's own memory, whose filled part is read back and then advanced past.
        let room = buf.remaining() > 0;
        // SAFETY: the slice goes only into a `ReadBuf`, which tracks what of it is initialized.
        let mut read = ReadBuf::uninit(unsafe { buf.as_mut() });
        match Pin::new(&mut this.io).poll_read(cx, read.unfilled()) {
            Poll::Ready(Ok(())) => {
                let filled = read.filled();
                let bytes = filled.len();
                if bytes > 0 {
                    this.conn.bytes_read(bytes);
                    this.sniffer.feed(filled, &this.conn);
                } else if room {
                    this.conn.eof();
                }
                // SAFETY: the read initialized the first `bytes` bytes of the cursor's memory.
                unsafe { buf.advance(bytes) };
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<T: Write + Unpin> Write for Coalescing<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.hold(buf.len(), cx) {
            return Poll::Pending;
        }
        let written = Pin::new(&mut this.io).poll_write(cx, buf);
        this.sent(buf.len(), written)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let len = bufs.iter().map(|buf| buf.len()).sum();
        if this.hold(len, cx) {
            return Poll::Pending;
        }
        let written = Pin::new(&mut this.io).poll_write_vectored(cx, bufs);
        this.sent(len, written)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A connection that takes up to `most` bytes of each write and records how many it took.
    #[derive(Clone)]
    struct Recorded {
        most: usize,
        writes: Arc<Mutex<Vec<usize>>>,
    }

    impl Recorded {
        fn taking(most: usize) -> Self {
            Self {
                most,
                writes: Arc::default(),
            }
        }

        fn take(&self, len: usize) -> Poll<io::Result<usize>> {
            let taken = len.min(self.most);
            self.writes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(taken);
            Poll::Ready(Ok(taken))
        }

        fn writes(&self) -> Vec<usize> {
            self.writes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl Write for Recorded {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.take(buf.len())
        }

        fn poll_write_vectored(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bufs: &[io::IoSlice<'_>],
        ) -> Poll<io::Result<usize>> {
            self.take(bufs.iter().map(|buf| buf.len()).sum())
        }

        fn is_write_vectored(&self) -> bool {
            true
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct Written {
        writes: Vec<usize>,
        /// How many times the writing task was polled before its buffer was gone.
        polls: usize,
    }

    /// Writes what `pending` holds until all of it is gone, as hyper writes its buffer, while a
    /// task ready before the first attempt appends `added` to it. A vectored write offers the
    /// buffer in two halves.
    fn write_with(
        limit: usize,
        first: usize,
        added: usize,
        connection: Recorded,
        vectored: bool,
    ) -> Written {
        let pending = Arc::new(Mutex::new(vec![0_u8; first]));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let polls = runtime.block_on(async {
            let mut coalescing = Coalescing::new(
                connection.clone(),
                limit,
                Arc::default(),
                ConnCounters::default(),
            );
            let buffer = Arc::clone(&pending);
            let writing = tokio::spawn(async move {
                let mut polls = 0;
                std::future::poll_fn(|cx| {
                    polls += 1;
                    let mut bytes = buffer.lock().unwrap_or_else(PoisonError::into_inner);
                    while !bytes.is_empty() {
                        let (front, back) = bytes.split_at(bytes.len() / 2);
                        let attempt = if vectored {
                            let halves = [io::IoSlice::new(front), io::IoSlice::new(back)];
                            Pin::new(&mut coalescing).poll_write_vectored(cx, &halves)
                        } else {
                            Pin::new(&mut coalescing).poll_write(cx, &bytes)
                        };
                        match attempt {
                            Poll::Ready(Ok(n)) => drop(bytes.drain(..n)),
                            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                            Poll::Pending => return Poll::Pending,
                        }
                    }
                    Poll::Ready(Ok(()))
                })
                .await
                .map(|()| polls)
            });
            let adding = tokio::spawn(async move {
                pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend(std::iter::repeat_n(0_u8, added));
            });
            adding.await.expect("the addition");
            writing.await.expect("the write").expect("written")
        });
        Written {
            writes: connection.writes(),
            polls,
        }
    }

    #[test]
    fn what_a_ready_task_adds_goes_in_the_same_write() {
        let written = write_with(16_384, 100, 50, Recorded::taking(usize::MAX), false);
        assert_eq!(written.writes, [150]);
    }

    #[test]
    fn a_vectored_write_gathers_the_same_way() {
        let written = write_with(16_384, 100, 50, Recorded::taking(usize::MAX), true);
        assert_eq!(written.writes, [150]);
    }

    #[test]
    fn what_is_left_of_a_write_taken_in_part_goes_unheld() {
        let written = write_with(16_384, 100, 50, Recorded::taking(100), false);
        assert_eq!(written.writes, [100, 50]);
        // Held at the first attempt and after the round that brought the 50; the third poll sends
        // both parts.
        assert_eq!(written.polls, 3);
    }

    #[test]
    fn a_write_at_the_limit_goes_at_once() {
        let written = write_with(100, 100, 50, Recorded::taking(usize::MAX), false);
        assert_eq!((written.writes, written.polls), (vec![100], 1));
    }

    #[test]
    fn a_limit_of_zero_holds_nothing() {
        let written = write_with(0, 100, 50, Recorded::taking(usize::MAX), false);
        assert_eq!((written.writes, written.polls), (vec![100], 1));
    }
}
