use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError, TryLockError};
use std::task::{ready, Context, Poll};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use http::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, USER_AGENT};
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::client::conn::http2::SendRequest;
use tokio::sync::{broadcast, watch};
use tonic::codegen::tokio_stream::Stream;
use tonic::metadata::MetadataMap;
use tower_service::Service;

use super::error::GrpcChannelConfigError;
use crate::http2::{TransportConfig, TransportConnector};
use crate::options::LARGEST_WINDOW;

use super::call::{
    self, Answered, CallControl, CallStartOptions, Deadline, GrpcCall, ResponseSink, SendHalf,
};
use super::contained::contained;
use super::driver::{self, Outgoing, Sending};
use super::error::ChannelError;
use super::executor::Spawner;
use super::request::OneRequest;
use super::retry::{AttemptMessages, ChannelReplay, RequestBody, RetryConfig};
use super::status::{GrpcStatus, GrpcStatusCode, Unprocessed};
use crate::utils::safe_endpoint;

const DEFAULT_USER_AGENT: &str = concat!("armonik-transport/", env!("CARGO_PKG_VERSION"));

/// Advertised as the only encoding because this engine decompresses nothing: a peer that reads
/// `grpc-accept-encoding` then sends what can be read rather than a body that cannot.
const ACCEPTED_ENCODING: &str = "identity";

const DEFAULT_MAX_RECV_MESSAGE_SIZE: usize = 4 * 1024 * 1024;
const DEFAULT_DELIVERY_COALESCING: usize = 16 * 1024;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GrpcChannelConfig {
    pub transport: TransportConfig,
    pub user_agent: Option<String>,
    pub max_sends_in_flight: usize,
    /// The largest message a call sends; a larger one ends its call `RESOURCE_EXHAUSTED` and none
    /// of it is sent. None sends any message a four-byte length carries.
    pub max_send_message_size: Option<usize>,
    pub max_recv_message_size: usize,
    /// How many bytes of a response's messages a delivery to a sink that gathers may wait to
    /// gather. 0 delivers each read at once.
    pub delivery_coalescing: usize,
    /// The deadline of a call that states none, counted from its start.
    pub default_deadline: Option<Duration>,
    /// When a failed call is sent again. With none, a call keeps no copy, so only one its peer
    /// never processed, and that had sent nothing, goes again.
    pub retry: Option<RetryConfig>,
}

impl GrpcChannelConfig {
    pub fn new(transport: TransportConfig) -> Self {
        Self {
            transport,
            user_agent: None,
            max_sends_in_flight: 1,
            max_send_message_size: None,
            max_recv_message_size: DEFAULT_MAX_RECV_MESSAGE_SIZE,
            delivery_coalescing: DEFAULT_DELIVERY_COALESCING,
            default_deadline: None,
            retry: None,
        }
    }
}

#[derive(Clone)]
pub struct GrpcChannel {
    inner: Arc<Inner>,
}

impl GrpcChannel {
    pub fn new(
        config: GrpcChannelConfig,
        spawner: tokio::runtime::Handle,
    ) -> Result<Self, GrpcChannelConfigError> {
        if config.max_sends_in_flight == 0 {
            return Err(GrpcChannelConfigError::ZeroSendWindow);
        }

        // The schema's bound, checked at a door that takes a number rather than a document: the
        // window sizes a channel whose semaphore panics above its own limit instead of refusing,
        // and the first call is where that would land.
        if config.max_sends_in_flight > LARGEST_WINDOW as usize {
            return Err(GrpcChannelConfigError::SendWindowTooLarge {
                value: config.max_sends_in_flight,
            });
        }

        if config.max_send_message_size == Some(0) {
            return Err(GrpcChannelConfigError::ZeroMaxSendMessageSize);
        }

        if config.max_recv_message_size == 0 {
            return Err(GrpcChannelConfigError::ZeroMaxRecvMessageSize);
        }

        if let Some(retry) = &config.retry {
            retry.admissible()?;
        }
        let replay = Arc::new(ChannelReplay::new(
            config
                .retry
                .as_ref()
                .map_or(0, |retry| retry.channel_replay_bytes),
        ));

        let user_agent = match &config.user_agent {
            None => HeaderValue::from_static(DEFAULT_USER_AGENT),
            Some(text) => HeaderValue::from_str(text).map_err(|_| {
                GrpcChannelConfigError::InvalidUserAgent {
                    value: text.clone(),
                }
            })?,
        };

        let endpoint = config.transport.endpoint.clone();
        let idle_timeout = config.transport.http2.idle_timeout;
        let calls_per_session = config
            .transport
            .http2
            .simultaneous_calls_per_connection
            .unwrap_or(usize::MAX);
        let connector = TransportConnector::new(config.transport)?;

        Ok(Self {
            inner: Arc::new(Inner {
                endpoint,
                connector,
                spawner,
                user_agent,
                max_sends_in_flight: config.max_sends_in_flight,
                max_send_message_size: config.max_send_message_size,
                max_recv_message_size: config.max_recv_message_size,
                delivery_coalescing: config.delivery_coalescing,
                default_deadline: config.default_deadline,
                retry: config.retry,
                replay,
                idle_timeout,
                calls_per_session,
                sessions: std::sync::Mutex::new(Sessions::default()),
                closed: watch::channel(false).0,
            }),
        })
    }

    pub async fn connect(&self) -> Result<(), ChannelError> {
        self.inner.sender().await.map(|_| ())
    }

    pub fn start_call(&self, options: CallStartOptions) -> Result<GrpcCall, ChannelError> {
        let (path, metadata, deadline) = self.addressed(&options)?;
        let (grpc_call, messages, driving) = call::create(
            self.inner.max_sends_in_flight,
            self.inner.max_send_message_size,
            self.inner.closed.subscribe(),
        );
        let outgoing = Outgoing {
            path,
            metadata,
            messages: Sending::Stream(messages),
            deadline,
            read_gate: options.read_gate,
            one_response: options.one_response,
        };
        self.inner
            .spawner
            .spawn(driver::drive(self.inner.clone(), outgoing, driving));
        Ok(grpc_call)
    }

    /// A call's send half and control, and what drives it, which the caller runs on the channel's
    /// runtime, joined with its own work on the call rather than as a task of its own. Nothing is
    /// sent until it is driven.
    pub fn prepare_call(
        &self,
        options: CallStartOptions,
    ) -> Result<(SendHalf, CallControl, CallDriver), ChannelError> {
        let (path, metadata, deadline) = self.addressed(&options)?;
        let (send, control, messages, driving) = call::create_with(
            self.inner.max_sends_in_flight,
            self.inner.max_send_message_size,
            self.inner.closed.subscribe(),
        );
        let outgoing = Outgoing {
            path,
            metadata,
            messages: Sending::Stream(messages),
            deadline,
            read_gate: options.read_gate,
            one_response: options.one_response,
        };
        Ok((
            send,
            control,
            CallDriver {
                inner: self.inner.clone(),
                outgoing,
                driving,
            },
        ))
    }

    /// A call that sends one request: where its request goes, its control, and what drives it,
    /// which the caller runs as `prepare_call`'s. The request is given once, and the call sends
    /// nothing, not even its head, until it is.
    pub fn prepare_one_request_call(
        &self,
        options: CallStartOptions,
    ) -> Result<(OneRequest, CallControl, CallDriver), ChannelError> {
        let (path, metadata, deadline) = self.addressed(&options)?;
        let (request, control, messages, driving) = call::create_one(self.inner.closed.subscribe());
        let outgoing = Outgoing {
            path,
            metadata,
            messages: Sending::One(messages),
            deadline,
            read_gate: options.read_gate,
            one_response: options.one_response,
        };
        Ok((
            request,
            control,
            CallDriver {
                inner: self.inner.clone(),
                outgoing,
                driving,
            },
        ))
    }

    /// Where a call goes, with what, and until when; refused on a closed channel.
    #[allow(clippy::type_complexity)]
    fn addressed(
        &self,
        options: &CallStartOptions,
    ) -> Result<(PathAndQuery, HeaderMap, Option<tokio::time::Instant>), ChannelError> {
        if *self.inner.closed.borrow() {
            return Err(ChannelError::Closed);
        }

        // An instant past what the clock holds is a deadline no call reaches, which is none.
        let now = tokio::time::Instant::now();
        let deadline = match options.deadline {
            Some(Deadline::Absolute(at)) => Some(tokio::time::Instant::from_std(at)),
            Some(Deadline::Timeout(after)) => now.checked_add(after),
            None => self
                .inner
                .default_deadline
                .and_then(|after| now.checked_add(after)),
        };

        let path = method_path(&options.method)?;
        let mut metadata = HeaderMap::new();
        options
            .metadata
            .write_into(&mut metadata)
            .map_err(|source| ChannelError::InvalidMetadata { source })?;
        Ok((path, metadata, deadline))
    }

    pub fn close(&self) {
        if self.inner.closed.send_replace(true) {
            return;
        }

        // Each call holds a sender of its own, so a session closes once its calls are done.
        self.inner.sessions().open.clear();
    }
}

/// What drives a call, from its request to its terminal: arguments rather than a future, so that
/// the future is built where it is polled and the caller's own stays small.
#[must_use = "the call makes no progress until its driver is polled"]
pub struct CallDriver {
    inner: Arc<Inner>,
    outgoing: Outgoing,
    driving: driver::Driving<()>,
}

impl CallDriver {
    /// The call, driven to its terminal, its response going to `sink`.
    pub fn drive<S: ResponseSink>(self, sink: S) -> impl Future<Output = ()> + Send + 'static {
        driver::drive(self.inner, self.outgoing, self.driving.with_sink(sink))
    }
}

impl std::fmt::Debug for CallDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallDriver").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for GrpcChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcChannel")
            .field("endpoint", &self.inner.endpoint)
            .field("closed", &*self.inner.closed.borrow())
            .finish()
    }
}

/// What the engine adds to every request, beside the `te` and `content-type` tonic's client writes.
fn engine_headers(user_agent: &HeaderValue) -> HeaderMap {
    let mut headers = HeaderMap::with_capacity(2);
    headers.insert(USER_AGENT, user_agent.clone());
    headers.insert(
        HeaderName::from_static("grpc-accept-encoding"),
        HeaderValue::from_static(ACCEPTED_ENCODING),
    );
    headers
}

pub(crate) struct Inner {
    endpoint: Uri,
    connector: TransportConnector,
    spawner: tokio::runtime::Handle,
    user_agent: HeaderValue,
    max_sends_in_flight: usize,
    pub(crate) max_send_message_size: Option<usize>,
    max_recv_message_size: usize,
    pub(crate) delivery_coalescing: usize,
    default_deadline: Option<Duration>,
    pub(crate) retry: Option<RetryConfig>,
    /// The replay bytes the channel's calls hold together.
    pub(crate) replay: Arc<ChannelReplay>,
    idle_timeout: Option<Duration>,
    /// How many calls one session carries at once, below what its server allows; `usize::MAX`
    /// when only the server bounds them.
    calls_per_session: usize,
    sessions: std::sync::Mutex<Sessions>,
    closed: watch::Sender<bool>,
}

/// The channel's HTTP/2 sessions, and the dials that open more.
#[derive(Default)]
struct Sessions {
    next: u64,
    open: Vec<Session>,
    dials: Vec<Dial>,
}

struct Session {
    id: u64,
    sender: SendRequest<tonic::body::Body>,
    /// How many streams its server lets it open at once, asked of the connection each time:
    /// hyper takes SETTINGS in on a task of its own and wakes nothing here. Without waiting,
    /// because the task that drives the connection holds it while it polls, and a poll can let go
    /// of a call's claim, which takes the lock the asker holds; a connection busy polling gives
    /// its last answer.
    streams: Box<dyn Fn() -> usize + Send + Sync>,
    /// The calls it carries.
    calls: usize,
    /// When it last carried no call.
    idle_since: tokio::time::Instant,
    /// Whether its idle timer is running.
    timing: bool,
}

/// A dial in flight, how many calls wait on it, and how its outcome reaches them.
///
/// A dial opens a connection of the channel's, so it belongs to the channel and not to whichever
/// call reached it first. Run inside that call's future it would be the call's: ending the call -
/// a deadline, a cancel, its channel closing - would drop the future and the dial with it, and the
/// calls waiting on it would start again from nothing. Under a stream of calls whose deadline is
/// shorter than a dial, none of them would ever complete one, though a single call left alone
/// would.
struct Dial {
    id: u64,
    waiting: usize,
    outcome: broadcast::Sender<Result<(), ChannelError>>,
}

/// A call's claim on its session: from the request's dispatch to the end of its response and of
/// its request.
///
/// The last claim on a session let go starts its idle timer, if the channel has one and it is not
/// already running; the session is closed once it has been idle for the timeout.
#[derive(Clone)]
pub(crate) struct Lease {
    _claim: Arc<Claim>,
}

struct Claim {
    inner: Arc<Inner>,
    session: u64,
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.inner.release(self.session);
    }
}

/// The idle timer of one session, sleeping until it has been idle for the timeout since its last
/// claim was let go, and closing it then unless a claim is taken.
async fn close_when_idle(
    weak: std::sync::Weak<Inner>,
    id: u64,
    since: tokio::time::Instant,
    idle_timeout: Duration,
) {
    let mut since = since;
    loop {
        let deadline = since.checked_add(idle_timeout);
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            // Past what the clock holds: a session never idle for that long.
            None => std::future::pending().await,
        }
        // Weak, so a timer does not keep a channel nobody holds, nor its sessions, alive.
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let mut sessions = inner.sessions();
        let Some(at) = sessions.open.iter().position(|session| session.id == id) else {
            return;
        };
        let session = &mut sessions.open[at];
        match session.idle_since {
            // Claimed again: the claim that is let go last starts the timer anew.
            _ if session.calls > 0 => {
                session.timing = false;
                return;
            }
            // Claimed and let go since: idle for less than the timeout yet.
            later if later > since => since = later,
            // A sleep tokio cut short of a deadline years away.
            _ if deadline.is_some_and(|deadline| tokio::time::Instant::now() < deadline) => {}
            _ => {
                sessions.open.swap_remove(at);
                return;
            }
        }
    }
}

impl Inner {
    /// A tonic client over this channel's sessions. One per call, and cheap: it holds the
    /// channel's handle and its configuration, not a connection of its own - and the call's
    /// marker, which it sets once the peer's response is in.
    pub(crate) fn client(
        self: &Arc<Self>,
        answered: Answered,
        one_response: bool,
        body: RequestBody,
    ) -> tonic::client::Grpc<Http2> {
        tonic::client::Grpc::with_origin(
            Http2 {
                inner: Arc::clone(self),
                answered,
                one_response,
                body: Some(body),
            },
            self.endpoint.clone(),
        )
        .max_decoding_message_size(addressable(self.max_recv_message_size))
    }

    /// The sessions, locked. Never held across an await.
    fn sessions(&self) -> std::sync::MutexGuard<'_, Sessions> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A session with room for one more call, and the call's claim on it, dialling one if none
    /// has room.
    ///
    /// A session has room below both `calls_per_session` and the streams its server allows. The
    /// server tells the second once the session is open, h2 assuming 100 until then, so the calls
    /// placed on it before may go past it. The server refuses those it sees: a refused call whose
    /// request is still held whole goes again once, where there is room, and ends UNAVAILABLE
    /// otherwise. One that h2 has not sent when the limit arrives waits in h2 for a stream of the
    /// session to end.
    ///
    /// The session taken is the fullest that has room, then the one idle most recently, so that
    /// the others go idle and close. With none, a caller joins a dial in flight that has room for
    /// it, or starts one - and starting one means spawning it, not running it here. Going away
    /// then detaches this caller from the dial instead of cancelling it for everyone waiting. A
    /// dial that ends opens a session its callers then take like any other, so one that finds it
    /// full by then looks again.
    async fn sender(
        self: &Arc<Self>,
    ) -> Result<(SendRequest<tonic::body::Body>, Lease), ChannelError> {
        loop {
            let mut waiting = {
                let mut sessions = self.sessions();
                if *self.closed.borrow() {
                    return Err(ChannelError::Closed);
                }

                sessions.open.retain(|session| !session.sender.is_closed());
                let roomy = sessions
                    .open
                    .iter_mut()
                    // At least one: a server that allows none for now has a call wait in h2 rather
                    // than the channel dial for good.
                    .filter(|session| {
                        session.calls < self.calls_per_session
                            && session.calls < (session.streams)().max(1)
                    })
                    .max_by_key(|session| (session.calls, session.idle_since));
                if let Some(session) = roomy {
                    session.calls += 1;
                    let lease = Lease {
                        _claim: Arc::new(Claim {
                            inner: Arc::clone(self),
                            session: session.id,
                        }),
                    };
                    return Ok((session.sender.clone(), lease));
                }

                let calls_per_session = self.calls_per_session;
                match sessions
                    .dials
                    .iter_mut()
                    .find(|dial| dial.waiting < calls_per_session)
                {
                    Some(dial) => {
                        dial.waiting += 1;
                        dial.outcome.subscribe()
                    }
                    None => {
                        // One, because one outcome is sent and every waiter subscribed before it
                        // was.
                        let (outcome, waiting) = broadcast::channel(1);
                        let id = sessions.next;
                        sessions.next += 1;
                        sessions.dials.push(Dial {
                            id,
                            waiting: 1,
                            outcome,
                        });
                        let inner = Arc::clone(self);
                        self.spawner.spawn(async move { inner.dial(id).await });
                        waiting
                    }
                }
            };

            // The lock is released, so the dial is free to take it when it is done. A caller
            // dropped here drops only its receiver.
            match waiting.recv().await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(error),
                // The dial task went away without an outcome, which happens when the runtime it
                // was spawned on is shutting down.
                Err(_) => return Err(ChannelError::Closed),
            }
        }
    }

    /// Opens a session and tells whoever waited.
    ///
    /// Its own task, so no caller owns it. The order at the end matters: the session is added and
    /// the dial cleared before the outcome goes out, so a caller that arrives after the send finds
    /// the session rather than a dial that is no longer running. The session starts idle, its
    /// timer running, so that one whose callers all went away is closed too.
    async fn dial(self: Arc<Self>, id: u64) {
        // Contained, because a panic here would leave the dial listed with no task behind it, and
        // every caller waiting on it would wait for good.
        let dialled = contained(async {
            #[cfg(feature = "test-hooks")]
            crate::hooks::run_in_dial();

            crate::http2::open(
                &self.connector,
                &self.endpoint,
                Spawner(self.spawner.clone()),
            )
            .await
        })
        .await;

        let mut sessions = self.sessions();
        let Some(at) = sessions.dials.iter().position(|dial| dial.id == id) else {
            return;
        };
        let outcome = sessions.dials.swap_remove(at).outcome;
        let told = |result| {
            // Every waiter may have gone; the sessions are what the next caller reads.
            let _ = outcome.send(result);
        };

        let (sender, connection) = match dialled {
            Some(Ok(session)) => session,
            Some(Err(error)) => return told(Err(ChannelError::from(error))),
            None => {
                return told(Err(ChannelError::DialPanicked {
                    endpoint: safe_endpoint(&self.endpoint),
                }))
            }
        };

        if *self.closed.borrow() {
            return told(Err(ChannelError::Closed));
        }

        let endpoint = safe_endpoint(&self.endpoint);
        let last = AtomicUsize::new(connection.current_max_send_streams());
        let connection = Arc::new(std::sync::Mutex::new(Some(Box::pin(connection))));
        let driven = Driven(Arc::clone(&connection));
        self.spawner.spawn(async move {
            let ended = std::future::poll_fn(|cx| {
                match driven
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_mut()
                {
                    Some(connection) => connection.as_mut().poll(cx),
                    None => Poll::Ready(Ok(())),
                }
            })
            .await;
            if let Err(error) = ended {
                tracing::debug!(%endpoint, %error, "the HTTP/2 session ended");
            }
        });

        let since = tokio::time::Instant::now();
        sessions.open.push(Session {
            id,
            sender,
            streams: Box::new(move || {
                let connection = match connection.try_lock() {
                    Ok(connection) => connection,
                    Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                    Err(TryLockError::WouldBlock) => return last.load(Ordering::Relaxed),
                };
                let Some(connection) = connection.as_ref() else {
                    return last.load(Ordering::Relaxed);
                };
                let streams = connection.current_max_send_streams();
                last.store(streams, Ordering::Relaxed);
                streams
            }),
            calls: 0,
            idle_since: since,
            timing: self.idle_timeout.is_some(),
        });
        drop(sessions);
        if let Some(idle_timeout) = self.idle_timeout {
            self.spawner.spawn(close_when_idle(
                Arc::downgrade(&self),
                id,
                since,
                idle_timeout,
            ));
        }
        told(Ok(()));
    }

    /// Lets go of a call's claim on session `id`, starting its idle timer if that was the last.
    fn release(self: &Arc<Self>, id: u64) {
        let (since, idle_timeout) = {
            let mut sessions = self.sessions();
            let Some(session) = sessions.open.iter_mut().find(|session| session.id == id) else {
                return;
            };
            session.calls -= 1;
            if session.calls > 0 {
                return;
            }
            let since = tokio::time::Instant::now();
            session.idle_since = since;
            let Some(idle_timeout) = self.idle_timeout else {
                return;
            };
            if std::mem::replace(&mut session.timing, true) {
                return;
            }
            (since, idle_timeout)
        };
        self.spawner.spawn(close_when_idle(
            Arc::downgrade(self),
            id,
            since,
            idle_timeout,
        ));
    }
}

/// A session's connection as the task that drives it holds it, shared with the session that asks
/// it how many streams its server allows.
///
/// The connection goes with the task, however the task ends - done, panicking, or dropped by a
/// runtime shutting down: until hyper's half of it goes, the session's sender does not read as
/// closed, and a call would be placed on a connection nobody drives. Never with the session,
/// whose removal holds the sessions' lock that a call's claim let go there would take again.
struct Driven<C>(Arc<std::sync::Mutex<Option<C>>>);

impl<C> Drop for Driven<C> {
    fn drop(&mut self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
}

/// The channel's HTTP/2 session, in the shape tonic's client sends a call through.
///
/// Ready at once: the session is dialled or joined inside `call`, where a call that goes away
/// detaches from the dial instead of cancelling it for every call waiting on it.
pub(crate) struct Http2 {
    inner: Arc<Inner>,
    answered: Answered,
    one_response: bool,
    /// The request's body as it goes on the wire, framed already, in place of what tonic encoded
    /// from an empty stream of messages. Taken by the one request tonic's client sends.
    body: Option<RequestBody>,
}

/// Removed from every head and every trailer before tonic reads them. tonic decodes it with an
/// `expect` wherever it reads a status, so a peer that sent it malformed would panic the call's
/// task - and nothing on this side reads it: a `grpc-` key never reaches a caller's metadata.
const GRPC_STATUS_DETAILS: &str = "grpc-status-details-bin";

/// The limit tonic's decoder is given: the configured one, within what this target can address.
///
/// tonic reserves an announced length before reading it, and a reserve past what an address can
/// span panics instead of refusing. Held under half the address space, such a message meets the
/// limit first and is refused as RESOURCE_EXHAUSTED. Only a 32-bit target is ever near it: a
/// four-byte length does not come close on 64 bits.
fn addressable(limit: usize) -> usize {
    limit.min(isize::MAX as usize / 2)
}

impl Service<http::Request<tonic::body::Body>> for Http2 {
    type Response = http::Response<ResponseBody>;
    type Error = tonic::Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: http::Request<tonic::body::Body>) -> Self::Future {
        let inner = Arc::clone(&self.inner);
        let answered = self.answered.clone();
        let one_response = self.one_response;
        match self.body.take() {
            Some(RequestBody::Framed(body)) => {
                *request.body_mut() = tonic::body::Body::new(http_body_util::Full::new(body));
            }
            Some(RequestBody::Stream(messages)) => {
                *request.body_mut() = tonic::body::Body::new(FramedMessages::new(messages));
            }
            None => {}
        }
        Box::pin(async move {
            request
                .headers_mut()
                .extend(engine_headers(&inner.user_agent));

            let (mut sender, lease) = inner.sender().await.map_err(|error| match error {
                ChannelError::Closed => worded(GrpcStatus::cancelled()),
                // A fault on this side, as the driver's own panic is, and not a peer out of
                // reach: UNAVAILABLE would tell a caller the connection or the server failed.
                ChannelError::DialPanicked { .. } => {
                    worded(GrpcStatus::new(GrpcStatusCode::Internal, error.to_string()))
                }
                error => worded(GrpcStatus::unreachable(error)),
            })?;
            // A call counts on its session until hyper is done with its request too: a call whose
            // response ends first keeps its request open, and its stream, until the driver
            // half-closes it, and the next call would otherwise overlap it.
            let held = lease.clone();
            let request =
                request.map(|body| tonic::body::Body::new(LeasedBody { body, _lease: held }));
            let mut response = sender.send_request(request).await.map_err(|error| {
                let mut status = worded(GrpcStatus::request_lost(&error));
                if let Some(unprocessed) = Unprocessed::of(&error) {
                    status.set_source(Arc::new(unprocessed));
                }
                status
            })?;
            answered.mark();
            // A response whose head ended it: the Trailers-Only shape, or an HTTP error with no body.
            if response.body().is_end_stream() {
                answered.mark_ended();
            }
            response.headers_mut().remove(GRPC_STATUS_DETAILS);
            refuse_what_is_not_grpc(&response)?;
            let response = refuse_a_message_behind_a_stated_status(response, &answered).await?;

            Ok(response.map(|body| ResponseBody::new(body, lease, one_response, answered)))
        })
    }
}

/// Below this, a framed message ready together with another is gathered with it into one DATA
/// frame.
const GATHERED_BELOW: usize = 4 * 1024;

/// What a frame of gathered messages holds at most: tonic's encoder's threshold for handing a batch
/// on.
const GATHERED_UP_TO: usize = 32 * 1024;

/// A stream's messages as a request body, each a DATA frame as it was framed, with no copy - but
/// for framed messages under [`GATHERED_BELOW`] bytes that are ready together, which are gathered
/// by a copy into one frame, up to [`GATHERED_UP_TO`], so that a stream of small messages does not
/// cost a frame each.
struct FramedMessages {
    messages: AttemptMessages,
    /// A message that ended a gathering, sent next.
    held: Option<Bytes>,
}

impl FramedMessages {
    fn new(messages: AttemptMessages) -> Self {
        Self {
            messages,
            held: None,
        }
    }
}

impl Body for FramedMessages {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let first = match this.held.take() {
            Some(message) => message,
            None => match ready!(Pin::new(&mut this.messages).poll_next(cx)) {
                Some(message) => message,
                None if this.messages.cut() => return Poll::Ready(Some(Err(cut_short()))),
                None => return Poll::Ready(None),
            },
        };
        if first.len() >= GATHERED_BELOW {
            return Poll::Ready(Some(Ok(Frame::data(first))));
        }

        // A small message is copied only to share a frame with another that is ready behind it.
        let mut gathered: Option<BytesMut> = None;
        loop {
            let size = gathered.as_ref().map_or(first.len(), BytesMut::len);
            match Pin::new(&mut this.messages).poll_next(cx) {
                Poll::Ready(Some(next))
                    if next.len() < GATHERED_BELOW && size + next.len() <= GATHERED_UP_TO =>
                {
                    gathered
                        .get_or_insert_with(|| BytesMut::from(&first[..]))
                        .extend_from_slice(&next);
                }
                Poll::Ready(Some(next)) => {
                    this.held = Some(next);
                    break;
                }
                Poll::Ready(None) | Poll::Pending => break,
            }
        }
        let frame = gathered.map_or(first, BytesMut::freeze);
        Poll::Ready(Some(Ok(Frame::data(frame))))
    }
}

/// How a request the call's stop cut short ends: an error, which hyper sends as RST_STREAM with the
/// reason of the `h2::Error` among its sources, rather than the body's end, which it sends as
/// END_STREAM - a whole request to the peer.
fn cut_short() -> tonic::Status {
    let mut status = tonic::Status::cancelled("the call ended before its request did");
    status.set_source(Arc::new(h2::Error::from(h2::Reason::CANCEL)));
    status
}

/// A request's body, holding its call's claim on its session until hyper has sent it or let it
/// go.
struct LeasedBody {
    body: tonic::body::Body,
    _lease: Lease,
}

impl Body for LeasedBody {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.get_mut().body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

/// A status this engine words, in the type tonic's client carries it in.
///
/// tonic hands back a `Status` it is given, which is what keeps these codes: its own reading of a
/// transport error answers UNKNOWN for a reset, its reset table being behind its `server`
/// feature.
fn worded(status: GrpcStatus) -> tonic::Status {
    tonic::Status::new(status.code, status.message)
}

fn broke(error: hyper::Error) -> tonic::Status {
    worded(GrpcStatus::stream_broke(&error))
}

/// The response body as tonic's decoder reads it, with the checks tonic does not make.
///
/// tonic takes trailers that arrive in the middle of a message for the end of the call and reports
/// their status, so a message the peer cut short is dropped under an OK. This follows the length
/// prefixes - each message's five-byte header, and how much of its body is still owed - without
/// holding any of the bytes, and answers such trailers with INTERNAL. On a call that declared one
/// response, it also passes up no byte of a second message: the frame that holds one is cut where
/// the first ends, and INTERNAL follows, before tonic decodes the second or anything charges it.
pub(crate) struct ResponseBody {
    inner: Incoming,
    framing: Framing,
    one_response: bool,
    /// The refusal of a second message, owed once the first's last bytes have gone up.
    refused: Option<tonic::Status>,
    /// Told when the peer's trailers, or its end of stream, come off the wire: tonic ends a call on
    /// those with the peer's status, which no failure of this side's reading of the messages
    /// before them does.
    answered: Answered,
    _lease: Lease,
}

impl ResponseBody {
    fn new(inner: Incoming, lease: Lease, one_response: bool, answered: Answered) -> Self {
        Self {
            inner,
            framing: Framing::default(),
            one_response,
            refused: None,
            answered,
            _lease: lease,
        }
    }

    fn second_message() -> tonic::Status {
        tonic::Status::internal("the server sent more than one message on a call that answers once")
    }
}

/// Where a stream of length-prefixed messages stands, from the bytes that went by.
#[derive(Default)]
struct Framing {
    header: [u8; 5],
    header_read: usize,
    owed: usize,
    /// Whole messages gone by.
    messages: usize,
}

impl Framing {
    fn follow(&mut self, data: &[u8]) {
        self.follow_up_to(data, usize::MAX);
    }

    /// Follows `data` until `limit` whole messages have gone by, and answers how many of its
    /// bytes it took: fewer than all only when a byte of the next message follows.
    fn follow_up_to(&mut self, data: &[u8], limit: usize) -> usize {
        let mut at = 0;
        while at < data.len() {
            if self.owed > 0 {
                let taken = self.owed.min(data.len() - at);
                self.owed -= taken;
                at += taken;
                if self.owed == 0 {
                    self.messages += 1;
                }
                continue;
            }
            if self.header_read == 0 && self.messages >= limit {
                return at;
            }

            let taken = (self.header.len() - self.header_read).min(data.len() - at);
            self.header[self.header_read..self.header_read + taken]
                .copy_from_slice(&data[at..at + taken]);
            self.header_read += taken;
            at += taken;
            if self.header_read == self.header.len() {
                let [_, length @ ..] = self.header;
                self.owed = u32::from_be_bytes(length) as usize;
                self.header_read = 0;
                if self.owed == 0 {
                    self.messages += 1;
                }
            }
        }
        at
    }

    fn between_messages(&self) -> bool {
        self.header_read == 0 && self.owed == 0
    }
}

impl Body for ResponseBody {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if let Some(refused) = this.refused.take() {
            return Poll::Ready(Some(Err(refused)));
        }
        let frame = match ready!(Pin::new(&mut this.inner).poll_frame(cx)) {
            None => {
                this.answered.mark_ended();
                return Poll::Ready(None);
            }
            Some(Err(error)) => return Poll::Ready(Some(Err(broke(error)))),
            Some(Ok(frame)) => frame,
        };

        let mut trailers = match frame.into_trailers() {
            Ok(trailers) => {
                this.answered.mark_ended();
                trailers
            }
            Err(frame) if !this.one_response => {
                if let Some(data) = frame.data_ref() {
                    this.framing.follow(data);
                }
                return Poll::Ready(Some(Ok(frame)));
            }
            Err(frame) => {
                let mut data = match frame.into_data() {
                    Ok(data) => data,
                    Err(frame) => return Poll::Ready(Some(Ok(frame))),
                };
                let taken = this.framing.follow_up_to(&data, 1);
                if taken == data.len() {
                    return Poll::Ready(Some(Ok(Frame::data(data))));
                }
                if taken == 0 {
                    return Poll::Ready(Some(Err(Self::second_message())));
                }
                data.truncate(taken);
                this.refused = Some(Self::second_message());
                return Poll::Ready(Some(Ok(Frame::data(data))));
            }
        };

        if !this.framing.between_messages() {
            return Poll::Ready(Some(Err(tonic::Status::internal(
                "the peer's trailers arrived in the middle of a message",
            ))));
        }
        trailers.remove(GRPC_STATUS_DETAILS);
        Poll::Ready(Some(Ok(Frame::trailers(trailers))))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// An HTTP answer that is not gRPC, refused before tonic's client reads its body as messages.
///
/// A proxy's error page is the usual one, and tonic would take the first byte of its HTML for a
/// message flag. The gRPC table maps the HTTP status instead, and a status the peer states stands
/// even behind an HTTP error.
fn refuse_what_is_not_grpc(response: &http::Response<Incoming>) -> Result<(), tonic::Status> {
    let headers = response.headers();
    let status = response.status();
    if status != StatusCode::OK {
        return Err(tonic::Status::from_header_map(headers).unwrap_or_else(|| {
            tonic::Status::with_metadata(
                http_code(status),
                format!(
                    "the peer answered HTTP {} rather than gRPC",
                    status.as_u16()
                ),
                MetadataMap::from_headers(headers.clone()),
            )
        }));
    }
    if !speaks_grpc(headers) {
        return Err(tonic::Status::internal(
            "the peer answered HTTP 200 without a gRPC content type",
        ));
    }
    Ok(())
}

/// A status in the head, held to the Trailers-Only shape: nothing may follow it.
///
/// tonic takes any status in the head for Trailers-Only and ends the call on it without reading
/// the body, so a message behind it would be dropped under whatever that status said. The body is
/// read to its end here first. An empty DATA frame is still nothing: hyper ends a body it did not
/// end on the head with one, which is how a Trailers-Only answer from a hyper server arrives.
/// UNKNOWN, because the call's status has not been said where the protocol puts it.
async fn refuse_a_message_behind_a_stated_status(
    mut response: http::Response<Incoming>,
    answered: &Answered,
) -> Result<http::Response<Incoming>, tonic::Status> {
    if !response.headers().contains_key("grpc-status") {
        return Ok(response);
    }
    while let Some(frame) = response.body_mut().frame().await {
        let frame = frame.map_err(broke)?;
        if frame.data_ref().is_some_and(|data| !data.is_empty()) {
            return Err(tonic::Status::unknown(
                "the peer stated a grpc-status in the response head and then sent a message",
            ));
        }
    }
    answered.mark_ended();
    Ok(response)
}

/// PROTOCOL-HTTP2's table from an HTTP status to the gRPC code a client reports for it.
fn http_code(status: StatusCode) -> GrpcStatusCode {
    match status.as_u16() {
        400 => GrpcStatusCode::Internal,
        401 => GrpcStatusCode::Unauthenticated,
        403 => GrpcStatusCode::PermissionDenied,
        404 => GrpcStatusCode::Unimplemented,
        429 | 502 | 503 | 504 => GrpcStatusCode::Unavailable,
        _ => GrpcStatusCode::Unknown,
    }
}

fn speaks_grpc(headers: &HeaderMap) -> bool {
    const GRPC: &str = "application/grpc";

    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let value = value.trim();
            value.len() >= GRPC.len()
                && value[..GRPC.len()].eq_ignore_ascii_case(GRPC)
                && value[GRPC.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| next == '+' || next == ';')
        })
        .unwrap_or(false)
}

fn method_path(method: &str) -> Result<PathAndQuery, ChannelError> {
    let invalid = || ChannelError::InvalidMethod {
        method: method.to_owned(),
    };

    if method.contains(['?', '#']) {
        return Err(invalid());
    }
    let mut segments = method.split('/');
    match (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) {
        (Some(""), Some(service), Some(name), None) if !service.is_empty() && !name.is_empty() => {}
        _ => return Err(invalid()),
    }

    method.parse().map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::super::driver::Delivery;
    use super::super::request::FramedMessage;
    use super::*;

    /// A small message alone goes as it was framed, with no copy.
    #[tokio::test]
    async fn a_small_message_alone_goes_as_it_was_framed() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = call::create(8, None, closed);
        let (mut send, _recv, _control) = call.split();
        let replay = super::super::retry::Replay::new(live, None, Arc::new(ChannelReplay::new(0)));

        let message = FramedMessage::copy_of(b"alone").expect("a message");
        let framed = message.body().as_ptr();
        send.send_framed(message).await.expect("queued");

        let mut body = FramedMessages::new(replay.attempt());
        let frame = body
            .frame()
            .await
            .expect("a frame")
            .expect("no error")
            .into_data()
            .expect("a DATA frame");
        assert_eq!(frame.as_ptr(), framed);
    }

    /// Small messages ready together go as one DATA frame, a large one as a frame of its own, and
    /// the order holds.
    #[tokio::test]
    async fn small_messages_ready_together_are_gathered_into_one_frame() {
        let (_closed, closed) = watch::channel(false);
        let (call, live, _driving) = call::create(8, None, closed);
        let (mut send, _recv, _control) = call.split();
        let replay = super::super::retry::Replay::new(live, None, Arc::new(ChannelReplay::new(0)));

        let large = vec![7; GATHERED_BELOW];
        for message in [&b"one"[..], b"two", &large, b"three"] {
            send.send_message(Bytes::copy_from_slice(message))
                .await
                .expect("queued");
        }
        drop(send);

        let mut body = FramedMessages::new(replay.attempt());
        let mut frames = Vec::new();
        while let Some(frame) = body.frame().await {
            frames.push(frame.expect("a frame").into_data().expect("a DATA frame"));
        }

        let framed = |message: &[u8]| {
            let mut frame = vec![0];
            frame.extend_from_slice(&u32::try_from(message.len()).expect("small").to_be_bytes());
            frame.extend_from_slice(message);
            frame
        };
        assert_eq!(
            frames,
            vec![
                Bytes::from([framed(b"one"), framed(b"two")].concat()),
                Bytes::from(framed(&large)),
                Bytes::from(framed(b"three")),
            ]
        );
    }

    /// Both edges of the window, at the door that takes a number.
    ///
    /// The FFI refuses this range out of a document; nothing refused it here, and a window past
    /// the semaphore's limit panics at the call that sizes its channel rather than at the
    /// configuration that named it.
    #[tokio::test]
    async fn a_send_window_outside_what_the_options_admit_is_refused() {
        let refused = |max_sends_in_flight| {
            let mut config = GrpcChannelConfig::new(TransportConfig::new(Uri::from_static(
                "http://127.0.0.1:1234",
            )));
            config.max_sends_in_flight = max_sends_in_flight;
            GrpcChannel::new(config, tokio::runtime::Handle::current()).err()
        };

        assert!(matches!(
            refused(0),
            Some(GrpcChannelConfigError::ZeroSendWindow)
        ));
        assert!(matches!(
            refused(LARGEST_WINDOW as usize + 1),
            Some(GrpcChannelConfigError::SendWindowTooLarge { .. })
        ));
        assert!(
            refused(LARGEST_WINDOW as usize).is_none(),
            "the deepest window the options admit is one this door takes"
        );
    }

    /// Zero is refused, as the receive limit's zero is: it admits only empty messages.
    #[tokio::test]
    async fn a_send_limit_of_zero_is_refused() {
        let refused = |max_send_message_size| {
            let mut config = GrpcChannelConfig::new(TransportConfig::new(Uri::from_static(
                "http://127.0.0.1:1234",
            )));
            config.max_send_message_size = max_send_message_size;
            GrpcChannel::new(config, tokio::runtime::Handle::current()).err()
        };

        assert!(matches!(
            refused(Some(0)),
            Some(GrpcChannelConfigError::ZeroMaxSendMessageSize)
        ));
        assert!(refused(Some(1)).is_none());
        assert!(refused(None).is_none());
    }

    /// Nothing is dialed: the driver is dropped before anything polls it, and the sink with it.
    #[tokio::test]
    async fn a_call_whose_driver_is_dropped_unpolled_never_ends() {
        let channel = GrpcChannel::new(
            GrpcChannelConfig::new(TransportConfig::new(Uri::from_static(
                "http://127.0.0.1:1234",
            ))),
            tokio::runtime::Handle::current(),
        )
        .expect("a valid configuration");
        let (head, head_rx) = tokio::sync::oneshot::channel();
        let (messages, _messages_rx) = tokio::sync::mpsc::channel(1);
        let (terminal, terminal_rx) = tokio::sync::oneshot::channel();
        let (_send, _control, driver) = channel
            .prepare_call(CallStartOptions::new("/echo.Echo/Say"))
            .expect("an open channel");
        drop(driver.drive(Delivery::new(head, messages, terminal)));

        assert!(head_rx.await.is_err(), "no head was given");
        assert!(terminal_rx.await.is_err(), "and no end");
    }

    #[test]
    fn every_header_on_the_wire_that_the_caller_did_not_write_is_one_it_may_not_set() {
        use http::header::{CONTENT_TYPE, TE};

        let engine = engine_headers(&HeaderValue::from_static("test"));
        assert_eq!(engine.len(), 2, "the set grew or shrank: {engine:?}");

        for name in engine.keys().chain([&TE, &CONTENT_TYPE]) {
            let refused = super::super::Metadata::new().append_ascii(name.as_str(), "mine");
            assert!(
                refused.is_err(),
                "`{name}` is set by the channel and a caller may set it too, so both would travel"
            );
        }
    }

    #[test]
    fn the_content_type_has_to_say_grpc() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_str(value).expect("valid"));
            headers
        };

        for value in [
            "application/grpc",
            "application/grpc+proto",
            "application/grpc; charset=utf-8",
            "Application/gRPC",
        ] {
            assert!(speaks_grpc(&with(value)), "{value}");
        }

        assert!(!speaks_grpc(&with("text/html")));
        assert!(!speaks_grpc(&with("application/grpcweb")));
        assert!(!speaks_grpc(&HeaderMap::new()));
    }

    #[test]
    fn framing_is_followed_across_the_chunks_a_message_arrives_in() {
        let mut framing = Framing::default();
        assert!(framing.between_messages(), "before anything");

        // A header split over two chunks, then a body of three.
        framing.follow(&[0, 0]);
        assert!(!framing.between_messages(), "inside a header");
        framing.follow(&[0, 0, 3, b'a']);
        assert!(!framing.between_messages(), "two bytes owed");
        framing.follow(b"bc");
        assert!(framing.between_messages(), "the message is whole");

        // An empty message, then a whole one, in a single chunk.
        framing.follow(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 1, b'x']);
        assert!(framing.between_messages(), "both are whole");

        framing.follow(&[0, 0, 0, 0, 4, b'y']);
        assert!(!framing.between_messages(), "three bytes owed");
    }

    /// One message allowed: the bytes up to its end are taken, the next header's first byte is not,
    /// wherever the frames cut them.
    #[test]
    fn framing_stops_where_the_one_message_allowed_ends() {
        let mut framing = Framing::default();
        assert_eq!(framing.follow_up_to(&[0, 0, 0, 0, 2, b'a'], 1), 6);
        assert_eq!(
            framing.follow_up_to(&[b'b', 0, 0], 1),
            1,
            "the second's header"
        );
        assert_eq!(framing.follow_up_to(&[0], 1), 0, "nothing more is taken");

        let mut empty = Framing::default();
        assert_eq!(
            empty.follow_up_to(&[0, 0, 0, 0, 0, 0], 1),
            5,
            "an empty message is whole at its header"
        );
    }

    #[test]
    fn a_limit_is_kept_within_what_the_target_can_address() {
        assert_eq!(addressable(4 * 1024 * 1024), 4 * 1024 * 1024);
        assert_eq!(addressable(usize::MAX), isize::MAX as usize / 2);
    }

    #[test]
    fn an_http_failure_maps_to_the_code_grpc_gives_it() {
        assert_eq!(
            http_code(StatusCode::NOT_FOUND),
            GrpcStatusCode::Unimplemented
        );
        assert_eq!(
            http_code(StatusCode::SERVICE_UNAVAILABLE),
            GrpcStatusCode::Unavailable
        );
        assert_eq!(http_code(StatusCode::IM_A_TEAPOT), GrpcStatusCode::Unknown);
    }

    #[test]
    fn anything_that_is_not_service_slash_method_is_refused() {
        assert!(method_path("/armonik.api.grpc.v1.Sessions/CreateSession").is_ok());
        for method in [
            "",
            "/",
            "Service/Method",
            "/Service",
            "/Service/",
            "//Method",
            "/Service/Method/Extra",
            "/Service/Method?query",
        ] {
            assert!(
                method_path(method).is_err(),
                "`{method}` should not be a method path"
            );
        }
    }
}
