//! A connection's writes, held while the work already ready on the runtime adds to them.

use std::future::Future;
use std::io;
use std::pin::{pin, Pin};
use std::task::{Context, Poll};

use hyper::rt::{Read, ReadBuf, ReadBufCursor, Write};

use crate::metrics::{ConnCounters, Sniffer};

/// Holds each write for as long as a scheduler round adds bytes to it, up to `limit` bytes.
///
/// hyper's HTTP/2 writes what it has buffered each time its connection task is polled, so a frame
/// that a task polled after it makes ready goes in a write of its own: a request's message, handed
/// over from another thread while its headers wait, follows them in a second write. Holding the
/// write behind the work already ready lets such frames join it. A round that adds bytes earns
/// another; one that adds none, or a buffer at the limit, sends it. A write that ends every
/// request it carries goes at once, as no round can add to those. A limit of 0 holds nothing.
pub(crate) struct Coalescing<T> {
    io: T,
    limit: usize,
    state: State,
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

impl<T> Coalescing<T> {
    pub(crate) fn new(io: T, limit: usize, conn: ConnCounters) -> Self {
        Self {
            io,
            limit,
            state: State::Open,
            conn,
            sniffer: Sniffer::new(),
        }
    }

    /// Whether the write of `bufs`, `len` bytes, waits a round, in which case this task is woken
    /// after it.
    fn hold(&mut self, bufs: &[io::IoSlice<'_>], len: usize, cx: &mut Context<'_>) -> bool {
        match self.state {
            State::Sending => return false,
            State::Held(before) if len <= before => {
                self.state = State::Sending;
                return false;
            }
            State::Open | State::Held(_) => {}
        }
        if len >= self.limit || ends_its_requests(bufs) {
            self.state = State::Sending;
            return false;
        }
        #[cfg(feature = "test-hooks")]
        crate::hooks::count_held_write();
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
        if let Poll::Ready(Ok(bytes)) = written {
            self.conn.bytes_written(bytes);
            #[cfg(feature = "test-hooks")]
            crate::hooks::count_write();
        }
        written
    }
}

const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const END_STREAM: u8 = 0x1;

/// How many open requests a write is read for; one that carries more is held.
const REQUESTS_READ: usize = 8;

/// Whether a write, which starts at a frame, ends at least one request and every request whose
/// frames it carries.
///
/// Read from the frames' headers, where END_STREAM is a flag of a request's last HEADERS or DATA
/// frame: the writer learns it from the bytes it writes, and shares no state with the calls. A
/// frame whose payload runs past the write is its last; a header the write cuts reads as a request
/// left open.
fn ends_its_requests(bufs: &[io::IoSlice<'_>]) -> bool {
    let mut slices = bufs.iter().map(|buf| &**buf);
    let mut rest: &[u8] = &[];
    let mut open = [0_u32; REQUESTS_READ];
    let mut opened = 0;
    let mut ended = false;
    'frames: loop {
        let mut header = [0_u8; 9];
        let mut filled = 0;
        while filled < header.len() {
            if rest.is_empty() {
                match slices.next() {
                    Some(next) => rest = next,
                    None if filled == 0 => break 'frames,
                    None => return false,
                }
                continue;
            }
            let taken = (header.len() - filled).min(rest.len());
            header[filled..filled + taken].copy_from_slice(&rest[..taken]);
            filled += taken;
            rest = &rest[taken..];
        }
        let [a, b, c, kind, flags, s0, s1, s2, s3] = header;
        if matches!(kind, DATA | HEADERS) {
            let stream = u32::from_be_bytes([s0 & 0x7f, s1, s2, s3]);
            let at = open[..opened].iter().position(|&open| open == stream);
            match (flags & END_STREAM != 0, at) {
                (true, Some(at)) => {
                    opened -= 1;
                    open.swap(at, opened);
                    ended = true;
                }
                (true, None) => ended = true,
                (false, Some(_)) => {}
                (false, None) if opened < open.len() => {
                    open[opened] = stream;
                    opened += 1;
                }
                (false, None) => return false,
            }
        }
        let mut payload = (usize::from(a) << 16) | (usize::from(b) << 8) | usize::from(c);
        while payload > 0 {
            if rest.is_empty() {
                match slices.next() {
                    Some(next) => rest = next,
                    None => break 'frames,
                }
                continue;
            }
            let taken = payload.min(rest.len());
            payload -= taken;
            rest = &rest[taken..];
        }
    }
    ended && opened == 0
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
        if this.hold(&[io::IoSlice::new(buf)], buf.len(), cx) {
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
        if this.hold(bufs, len, cx) {
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
        first: Vec<u8>,
        added: Vec<u8>,
        connection: Recorded,
        vectored: bool,
    ) -> Written {
        let pending = Arc::new(Mutex::new(first));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let polls = runtime.block_on(async {
            let mut coalescing =
                Coalescing::new(connection.clone(), limit, ConnCounters::default());
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
                    .extend(added);
            });
            adding.await.expect("the addition");
            writing.await.expect("the write").expect("written")
        });
        Written {
            writes: connection.writes(),
            polls,
        }
    }

    /// `len` bytes of zeros, which read as empty DATA frames that end no request.
    fn zeros(len: usize) -> Vec<u8> {
        vec![0; len]
    }

    /// A frame of `kind` on `stream`, with `flags` and a payload of `len` bytes, of which the
    /// first `written` are in the frame: a frame cut short is the start of one the write ends in.
    fn frame(kind: u8, flags: u8, stream: u32, len: usize, written: usize) -> Vec<u8> {
        let [_, a, b, c] = u32::try_from(len).expect("a frame's length").to_be_bytes();
        let mut frame = vec![a, b, c, kind, flags];
        frame.extend(stream.to_be_bytes());
        frame.resize(9 + written, 0);
        frame
    }

    /// The headers of a request, which say nothing of its end.
    fn headers(stream: u32) -> Vec<u8> {
        frame(HEADERS, 0x4, stream, 3, 3)
    }

    /// A message of a request, its last when `last`.
    fn data(stream: u32, last: bool) -> Vec<u8> {
        let flags = if last { END_STREAM } else { 0 };
        frame(DATA, flags, stream, 5, 5)
    }

    fn joined(frames: &[Vec<u8>]) -> Vec<u8> {
        frames.concat()
    }

    #[test]
    fn what_a_ready_task_adds_goes_in_the_same_write() {
        let written = write_with(
            16_384,
            zeros(100),
            zeros(50),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!(written.writes, [150]);
    }

    #[test]
    fn a_vectored_write_gathers_the_same_way() {
        let written = write_with(
            16_384,
            zeros(100),
            zeros(50),
            Recorded::taking(usize::MAX),
            true,
        );
        assert_eq!(written.writes, [150]);
    }

    #[test]
    fn what_is_left_of_a_write_taken_in_part_goes_unheld() {
        let written = write_with(16_384, zeros(100), zeros(50), Recorded::taking(100), false);
        assert_eq!(written.writes, [100, 50]);
        // Held at the first attempt and after the round that brought the 50; the third poll sends
        // both parts.
        assert_eq!(written.polls, 3);
    }

    #[test]
    fn a_write_at_the_limit_goes_at_once() {
        let written = write_with(
            100,
            zeros(100),
            zeros(50),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!((written.writes, written.polls), (vec![100], 1));
    }

    #[test]
    fn a_limit_of_zero_holds_nothing() {
        let written = write_with(
            0,
            zeros(100),
            zeros(50),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!((written.writes, written.polls), (vec![100], 1));
    }

    #[test]
    fn a_unary_request_goes_at_once() {
        let request = joined(&[headers(1), data(1, true)]);
        let len = request.len();
        for vectored in [false, true] {
            let written = write_with(
                16_384,
                request.clone(),
                headers(3),
                Recorded::taking(usize::MAX),
                vectored,
            );
            assert_eq!(
                (written.writes, written.polls),
                (vec![len], 1),
                "{vectored}"
            );
        }
    }

    #[test]
    fn a_header_split_between_two_parts_is_read_whole() {
        let request = joined(&[headers(1), data(1, true)]);
        // The vectored write splits the request inside the DATA frame's header.
        assert!((12..21).contains(&(request.len() / 2)));
        let written = write_with(
            16_384,
            request,
            Vec::new(),
            Recorded::taking(usize::MAX),
            true,
        );
        assert_eq!(written.polls, 1);
    }

    #[test]
    fn a_frame_whose_payload_is_not_in_the_write_still_ends_it() {
        let request = joined(&[headers(1), frame(DATA, END_STREAM, 1, 100, 0)]);
        let written = write_with(
            16_384,
            request,
            Vec::new(),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!((written.writes, written.polls), (vec![21], 1));
    }

    #[test]
    fn the_middle_messages_of_a_stream_still_gather() {
        let written = write_with(
            16_384,
            data(1, false),
            data(1, false),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!(written.writes, [28]);
        // Held at the first attempt and after the round that brought the second message.
        assert_eq!(written.polls, 3);
    }

    #[test]
    fn the_last_message_of_a_stream_goes_once_it_joins() {
        let written = write_with(
            16_384,
            data(1, false),
            data(1, true),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!((written.writes, written.polls), (vec![28], 2));
    }

    #[test]
    fn a_request_left_open_holds_the_write_that_ends_another() {
        let written = write_with(
            16_384,
            joined(&[headers(3), data(1, true)]),
            data(3, true),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!((written.writes, written.polls), (vec![40], 2));
    }

    #[test]
    fn a_write_of_no_request_is_held() {
        // A WINDOW_UPDATE of the connection.
        let window_update = frame(0x8, 0, 0, 4, 4);
        let written = write_with(
            16_384,
            window_update.clone(),
            window_update,
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!(written.writes, [26]);
    }

    #[test]
    fn more_requests_than_are_read_hold_the_write() {
        let mut first: Vec<u8> = (0..=REQUESTS_READ as u32)
            .flat_map(|n| headers(2 * n + 1))
            .collect();
        first.extend((0..=REQUESTS_READ as u32).flat_map(|n| data(2 * n + 1, true)));
        let len = first.len();
        let written = write_with(
            16_384,
            first,
            data(99, true),
            Recorded::taking(usize::MAX),
            false,
        );
        assert_eq!(written.writes, [len + 14]);
    }
}
