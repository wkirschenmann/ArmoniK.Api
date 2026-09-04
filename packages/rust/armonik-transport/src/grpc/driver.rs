//! The task that runs a call, from its request to its terminal.
//!
//! Separate from `call` because they are two readings of the same thing: `call` is what a caller
//! holds - halves it writes to and reads from - and this is what the spawned task holds. A reader
//! asking what can be done with a call has no business in the deframing loop, and one debugging
//! the loop has no business in the caller's API.
//!
//! Its one promise: whatever happens in here, the caller gets exactly one terminal. Every exit
//! goes through the same `end`.

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use http::Request;
use http_body_util::BodyExt;
use tokio::sync::{mpsc, oneshot, watch};

use super::call::{CallControl, OwnedMessage, RecvResult, RequestBody};
use super::channel::Inner;
use super::error::ChannelError;
use super::frame::Deframer;
use super::metadata::Metadata;
use super::status::{of_response_head, stated_status, GrpcStatus};

/// The driving task's half of a call.
pub(crate) struct Driving {
    stop: Stop,
    delivery: Delivery,
    control: CallControl,
}

impl Driving {
    /// Assembled by `call::create`, which holds the other end of each of these.
    ///
    /// A constructor rather than a literal built there: `Stop` and `Delivery` are how this task
    /// decides to give up and how it hands events over, and neither is any of the caller's
    /// business.
    pub(crate) fn new(
        over: watch::Receiver<bool>,
        channel_closed: watch::Receiver<bool>,
        head: oneshot::Sender<Metadata>,
        messages: mpsc::Sender<RecvResult>,
        control: CallControl,
    ) -> Self {
        Self {
            stop: Stop {
                over,
                channel_closed,
            },
            delivery: Delivery {
                head: Some(head),
                messages,
            },
            control,
        }
    }
}

/// Runs the call to its terminal, and delivers that terminal whatever happens.
pub(crate) async fn drive(inner: Arc<Inner>, request: Request<RequestBody>, driving: Driving) {
    let Driving {
        mut stop,
        mut delivery,
        control,
    } = driving;

    let status = run(inner, request, &mut stop, &mut delivery).await;
    // The call is over the moment its terminal is decided, whichever way it went; the
    // writing side is told before the reading side, so a reader holding the terminal knows
    // the writer is already refusing.
    control.cancel();
    delivery.end(&mut stop, status).await;
}

/// What ends a call from this side: the caller cancelled it, or the channel closed.
struct Stop {
    over: watch::Receiver<bool>,
    channel_closed: watch::Receiver<bool>,
}

impl Stop {
    /// Resolves once the channel closes, and not when the call itself ends.
    ///
    /// The terminal hand-over needs this one rather than `stopped`: `drive` raises `over` just
    /// before it, so a wait on the call's own end would abandon every terminal instead of
    /// delivering it. What must still be able to reap the task is the channel going away.
    async fn channel_closed(&mut self) {
        let _ = self.channel_closed.wait_for(|closed| *closed).await;
    }

    /// Resolves once the call should stop. A sender that is gone counts as stopped: nothing
    /// is left that could ask for the result.
    async fn stopped(&mut self) {
        // Destructured because `select!` puts both arms in one scope, where two `&mut self`
        // methods do not borrow-check as the disjoint fields they are.
        let Self {
            over,
            channel_closed,
        } = self;
        tokio::select! {
            _ = over.wait_for(|over| *over) => {}
            _ = channel_closed.wait_for(|closed| *closed) => {}
        }
    }
}

/// `work`'s result, unless the call stopped first.
async fn until_stopped<T>(stop: &mut Stop, work: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        biased;
        _ = stop.stopped() => None,
        value = work => Some(value),
    }
}

/// Where a call's events go, with the head resolved exactly once.
struct Delivery {
    head: Option<oneshot::Sender<Metadata>>,
    messages: mpsc::Sender<RecvResult>,
}

impl Delivery {
    fn head(&mut self, metadata: Metadata) {
        if let Some(head) = self.head.take() {
            let _ = head.send(metadata);
        }
    }

    /// Hands over one message, or reports that nobody is reading any more.
    async fn message(&self, data: Bytes) -> bool {
        self.messages
            .send(RecvResult::Message(OwnedMessage { data }))
            .await
            .is_ok()
    }

    /// Hands the terminal over, or gives up if the call is stopped while trying.
    ///
    /// The status is already decided - it came from the network - and getting it to the boundary
    /// is this crate's obligation, not the caller's. But a consumer that holds its half without
    /// reading would otherwise park this task for ever, and `close` could not reap it: this was
    /// the one send not guarded, so it was also the one a closing channel could not reach.
    ///
    /// Guarded on the channel alone, deliberately. The call's own end is raised immediately
    /// before this runs, so waiting on that would abandon every terminal.
    ///
    /// Untested, and not for want of trying: whether the task was reaped is not observable
    /// through this crate's API - `close` answers nothing and a spawn returns nothing - and a
    /// test that reads in order to look relieves the very block it is checking for. The two
    /// outcomes differ only in whether a consumer that stopped reading later sees the status or
    /// `Aborted`, which no caller of a closing channel is entitled to rely on either way.
    async fn end(mut self, stop: &mut Stop, status: GrpcStatus) {
        // A call that never saw a response head still answers the question, with nothing in it.
        self.head(Metadata::new());
        tokio::select! {
            biased;
            _ = self.messages.send(RecvResult::End(status)) => {}
            _ = stop.channel_closed() => {}
        }
    }
}

async fn run(
    inner: Arc<Inner>,
    request: Request<RequestBody>,
    stop: &mut Stop,
    delivery: &mut Delivery,
) -> GrpcStatus {
    let mut sender = match until_stopped(stop, inner.sender()).await {
        None | Some(Err(ChannelError::Closed)) => return GrpcStatus::cancelled(),
        Some(Err(error)) => return GrpcStatus::unreachable(error),
        Some(Ok(sender)) => sender,
    };

    let response = match until_stopped(stop, sender.send_request(request)).await {
        None => return GrpcStatus::cancelled(),
        Some(Err(error)) => return GrpcStatus::request_lost(error),
        Some(Ok(response)) => response,
    };

    let (head, mut body) = response.into_parts();
    let metadata = match of_response_head(head.status, &head.headers) {
        Err(status) => return status,
        Ok(metadata) => metadata,
    };

    delivery.head(metadata);

    // Nothing below needs the channel, and a streaming call may outlive its release.
    let mut deframer = Deframer::new(inner.max_recv_message_size());
    drop(inner);
    loop {
        if let Err(status) = deliver_ready(&mut deframer, stop, delivery).await {
            return status;
        }

        let frame = match until_stopped(stop, body.frame()).await {
            None => return GrpcStatus::cancelled(),
            Some(None) => return GrpcStatus::no_status(),
            Some(Some(Err(error))) => return GrpcStatus::stream_broke(error),
            Some(Some(Ok(frame))) => frame,
        };

        let trailers = match frame.into_data() {
            Ok(data) => {
                deframer.push(data);
                continue;
            }
            Err(frame) => match frame.into_trailers() {
                Ok(trailers) => trailers,
                // Neither data nor trailers: a frame kind this engine has no use for.
                Err(_) => continue,
            },
        };

        if !deframer.is_at_message_boundary() {
            return GrpcStatus::ended_mid_message();
        }
        return stated_status(&trailers).unwrap_or_else(GrpcStatus::no_trailing_status);
    }
}

/// Hands over every message the deframer already holds.
///
/// The error is the terminal to end the call with: either the peer's framing is unreadable,
/// or nobody is reading any more.
async fn deliver_ready(
    deframer: &mut Deframer,
    stop: &mut Stop,
    delivery: &Delivery,
) -> Result<(), GrpcStatus> {
    loop {
        match deframer.next_message() {
            Ok(None) => return Ok(()),
            Err(error) => return Err(error.status()),
            Ok(Some(message)) => match until_stopped(stop, delivery.message(message)).await {
                Some(true) => {}
                _ => return Err(GrpcStatus::cancelled()),
            },
        }
    }
}
