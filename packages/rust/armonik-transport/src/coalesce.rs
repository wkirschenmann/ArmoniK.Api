//! A connection's writes, held while the work already ready on the runtime adds to them.

use std::future::Future;
use std::io;
use std::pin::{pin, Pin};
use std::task::{Context, Poll};

use hyper::rt::{Read, ReadBufCursor, Write};

/// Holds each write for as long as a scheduler round adds bytes to it, up to `limit` bytes.
///
/// hyper's HTTP/2 writes what it has buffered each time its connection task is polled, so a frame
/// that a task polled after it makes ready goes in a write of its own: a request's message, handed
/// over from another thread while its headers wait, follows them in a second write. Holding the
/// write behind the work already ready lets such frames join it. A round that adds bytes earns
/// another; one that adds none, or a buffer at the limit, sends it. A limit of 0 holds nothing.
pub(crate) struct Coalescing<T> {
    io: T,
    limit: usize,
    state: State,
}

enum State {
    /// The next write is a new one.
    Open,
    /// A write of this length waits a round.
    Held(usize),
    /// A write goes, and what is left of it after a part or a full socket goes too, unheld.
    Sending,
}

impl<T> Coalescing<T> {
    pub(crate) fn new(io: T, limit: usize) -> Self {
        Self {
            io,
            limit,
            state: State::Open,
        }
    }

    /// Whether a write of `len` bytes waits a round, in which case this task is woken after it.
    fn hold(&mut self, len: usize, cx: &mut Context<'_>) -> bool {
        match self.state {
            State::Sending => return false,
            State::Held(before) if len <= before => {
                self.state = State::Sending;
                return false;
            }
            State::Open | State::Held(_) => {}
        }
        if len >= self.limit {
            self.state = State::Sending;
            return false;
        }
        self.state = State::Held(len);
        // Polled once and dropped: that queues this task's wake-up behind the tasks already
        // ready, including those woken from another thread, whose frames the next attempt brings.
        let _ = pin!(tokio::task::yield_now()).poll(cx);
        true
    }

    /// Opens the next write once this one has gone whole, or failed.
    fn sent(&mut self, len: usize, written: Poll<io::Result<usize>>) -> Poll<io::Result<usize>> {
        match written {
            Poll::Ready(Ok(n)) if n < len => {}
            Poll::Ready(_) => self.state = State::Open,
            Poll::Pending => {}
        }
        #[cfg(feature = "test-hooks")]
        if let Poll::Ready(Ok(_)) = written {
            crate::hooks::count_write();
        }
        written
    }
}

impl<T: Read + Unpin> Read for Coalescing<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
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
    use std::sync::{Arc, Mutex, PoisonError};

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
            let mut coalescing = Coalescing::new(connection.clone(), limit);
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
