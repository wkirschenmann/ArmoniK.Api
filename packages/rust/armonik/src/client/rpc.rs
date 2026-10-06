//! The four shapes of a call, over the engine's channel: what the generated clients call.
//!
//! A request is encoded with prost straight into a buffer with the frame prefix's room ahead of
//! it, which the engine sends as it is; a response is decoded from the bytes the engine delivers.
//! A call that streams requests sends them from a task spawned on the current tokio runtime, so it
//! is made from inside one.

use std::sync::{Arc, OnceLock};

use armonik_transport::grpc::{
    CallControl, CallStartOptions, ChannelError, FramedMessage, GrpcChannel, GrpcStatus,
    GrpcStatusCode, HeadOrigin, RecvHalf, RecvResult, SendHalf, FRAME_PREFIX,
};
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};

/// What a call answers, as tonic's `Response` gives it: [`Response::into_inner`] is the message.
#[derive(Debug)]
pub struct Response<T>(T);

impl<T> Response<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// The messages of a response stream, each decoded, or the status that ended it.
pub type Streaming<T> = BoxStream<'static, Result<T, GrpcStatus>>;

/// A call that sends one message and answers one.
pub(crate) async fn unary<Req, Resp>(
    channel: &GrpcChannel,
    path: &str,
    request: Req,
) -> Result<Response<Resp>, GrpcStatus>
where
    Req: prost::Message,
    Resp: prost::Message + Default,
{
    let (send, mut recv, _) = start(channel, path, true)?;
    send_one(send, &request).await?;
    single(&mut recv).await.map(Response)
}

/// A call that sends one message and answers a stream.
pub(crate) async fn server_streaming<Req, Resp>(
    channel: &GrpcChannel,
    path: &str,
    request: Req,
) -> Result<Response<Streaming<Resp>>, GrpcStatus>
where
    Req: prost::Message,
    Resp: prost::Message + Default + Send + 'static,
{
    let (send, recv, _) = start(channel, path, false)?;
    send_one(send, &request).await?;
    answered(recv, None).await.map(Response)
}

/// A call that sends a stream and answers one message. A peer may answer, or fail the call, before
/// the stream ends: the answer is the call's end, whatever is left to send.
pub(crate) async fn client_streaming<Req, Resp, S>(
    channel: &GrpcChannel,
    path: &str,
    requests: S,
) -> Result<Response<Resp>, GrpcStatus>
where
    Req: prost::Message + 'static,
    Resp: prost::Message + Default,
    S: Stream<Item = Req> + Send + 'static,
{
    let (send, mut recv, control) = start(channel, path, true)?;
    let sender = Sender::spawn(send, control, requests);
    single(&mut recv)
        .await
        .map(Response)
        .map_err(|status| sender.status(status))
}

/// A call that sends a stream and answers a stream.
pub(crate) async fn bidi_streaming<Req, Resp, S>(
    channel: &GrpcChannel,
    path: &str,
    requests: S,
) -> Result<Response<Streaming<Resp>>, GrpcStatus>
where
    Req: prost::Message + 'static,
    Resp: prost::Message + Default + Send + 'static,
    S: Stream<Item = Req> + Send + 'static,
{
    let (send, recv, control) = start(channel, path, false)?;
    let sender = Sender::spawn(send, control, requests);
    answered(recv, Some(sender)).await.map(Response)
}

fn start(
    channel: &GrpcChannel,
    path: &str,
    one_response: bool,
) -> Result<(SendHalf, RecvHalf, CallControl), GrpcStatus> {
    let mut options = CallStartOptions::new(path);
    options.one_response = one_response;
    Ok(channel.start_call(options).map_err(refused)?.split())
}

/// What a call that never started ends with, in the codes a gRPC client gives the same causes.
fn refused(error: ChannelError) -> GrpcStatus {
    let code = match error {
        ChannelError::Closed => GrpcStatusCode::Cancelled,
        ChannelError::Transport { .. } => GrpcStatusCode::Unavailable,
        _ => GrpcStatusCode::Internal,
    };
    GrpcStatus::new(code, error.to_string())
}

/// `message`, encoded in place after the frame prefix's room.
fn framed(message: &impl prost::Message) -> Result<FramedMessage, GrpcStatus> {
    let mut buffer = Vec::with_capacity(FRAME_PREFIX + message.encoded_len());
    buffer.resize(FRAME_PREFIX, 0);
    message
        .encode(&mut buffer)
        .expect("a Vec grows to what the message needs");
    let len = buffer.len() - FRAME_PREFIX;
    FramedMessage::in_place(buffer).ok_or_else(|| {
        GrpcStatus::new(
            GrpcStatusCode::ResourceExhausted,
            format!("a message of {len} bytes does not fit the four-byte gRPC length prefix"),
        )
    })
}

/// Sends `message` and ends the sending. A refused send is the call's end, which the response
/// reports with its status, so it is not reported here twice.
async fn send_one(mut send: SendHalf, message: &impl prost::Message) -> Result<(), GrpcStatus> {
    let _ = send.send_framed(framed(message)?).await;
    Ok(())
}

/// A call's request stream, sent from a task of its own so that the response is read while the
/// requests are still coming. The task goes with the call: dropping this aborts it, so a request
/// stream that never yields again is not held for ever.
struct Sender {
    task: tokio::task::JoinHandle<()>,
    failed: Arc<OnceLock<GrpcStatus>>,
}

impl Sender {
    fn spawn<Req, S>(send: SendHalf, control: CallControl, requests: S) -> Self
    where
        Req: prost::Message + 'static,
        S: Stream<Item = Req> + Send + 'static,
    {
        let failed = Arc::new(OnceLock::new());
        let task = tokio::spawn(send_all(send, control, requests, failed.clone()));
        Self { task, failed }
    }

    /// The call's status: the one the call `ended` with, unless a request could not be sent, which
    /// ended the call first and says why.
    fn status(&self, ended: GrpcStatus) -> GrpcStatus {
        self.failed.get().cloned().unwrap_or(ended)
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Sends every message of `requests`, then ends the sending; stops at the first the call refuses,
/// whose status the response reports. Sending that stops short of the stream's end - a request
/// that cannot be framed, a panic, the task aborted - ends the call with `failed`, rather than
/// leaving the peer a shorter stream that ends as if it were whole.
async fn send_all<Req, S>(
    mut send: SendHalf,
    control: CallControl,
    requests: S,
    failed: Arc<OnceLock<GrpcStatus>>,
) where
    Req: prost::Message,
    S: Stream<Item = Req>,
{
    let mut unfinished = Unfinished {
        control,
        failed,
        status: Some(GrpcStatus::new(
            GrpcStatusCode::Internal,
            "the request stream stopped before its end",
        )),
    };
    let mut requests = std::pin::pin!(requests);
    while let Some(request) = requests.next().await {
        let message = match framed(&request) {
            Ok(message) => message,
            Err(status) => {
                unfinished.status = Some(status);
                return;
            }
        };
        if send.send_framed(message).await.is_err() {
            break;
        }
    }
    unfinished.status = None;
}

/// Ends the call with `status` when dropped holding one: the sending stopped short of its end.
struct Unfinished {
    control: CallControl,
    failed: Arc<OnceLock<GrpcStatus>>,
    status: Option<GrpcStatus>,
}

impl Drop for Unfinished {
    fn drop(&mut self) {
        if let Some(status) = self.status.take() {
            let _ = self.failed.set(status);
            self.control.cancel();
        }
    }
}

/// The one message of a call that answers one, read to the call's status.
async fn single<Resp>(recv: &mut RecvHalf) -> Result<Resp, GrpcStatus>
where
    Resp: prost::Message + Default,
{
    let Some(answer) = receive(recv).await? else {
        return Err(GrpcStatus::new(
            GrpcStatusCode::Internal,
            "a call that answers one message ended OK with none",
        ));
    };
    // A second message is the engine's to refuse, as `one_response` asks of it.
    match receive::<Resp>(recv).await? {
        None => Ok(answer),
        Some(_) => Err(GrpcStatus::new(
            GrpcStatusCode::Internal,
            "a call that answers one message answered more",
        )),
    }
}

/// The response stream, once its head is in, as tonic answers a streaming call: a call that ends
/// with no head - its status alone, or refused before an answer - fails here, not at the stream's
/// first read. The stream holds `sender`, so the requests go on as long as it is read.
async fn answered<Resp>(
    mut recv: RecvHalf,
    sender: Option<Sender>,
) -> Result<Streaming<Resp>, GrpcStatus>
where
    Resp: prost::Message + Default + Send + 'static,
{
    let status = |ended: GrpcStatus| match &sender {
        Some(sender) => sender.status(ended),
        None => ended,
    };
    let origin = recv
        .recv_head()
        .await
        .map_err(|_| status(dropped()))?
        .origin;
    if origin == HeadOrigin::Wire {
        return Ok(answers(recv, sender));
    }
    match receive::<Resp>(&mut recv).await.map_err(status)? {
        None => Ok(futures::stream::empty().boxed()),
        Some(_) => Err(GrpcStatus::new(
            GrpcStatusCode::Internal,
            "a call answered a message with no head",
        )),
    }
}

/// The next message of a response, decoded; None once the call ended OK.
async fn receive<Resp>(recv: &mut RecvHalf) -> Result<Option<Resp>, GrpcStatus>
where
    Resp: prost::Message + Default,
{
    match recv.next_message().await {
        Ok(RecvResult::Message(message)) => Resp::decode(message.data)
            .map(Some)
            .map_err(|error| GrpcStatus::new(GrpcStatusCode::Internal, error.to_string())),
        Ok(RecvResult::End(status)) if status.code == GrpcStatusCode::Ok => Ok(None),
        Ok(RecvResult::End(status)) => Err(status),
        Err(_) => Err(dropped()),
    }
}

/// What a call ends with when it has no status to read: the runtime driving it went away.
fn dropped() -> GrpcStatus {
    GrpcStatus::new(
        GrpcStatusCode::Internal,
        "the call ended with no status: its runtime is shutting down",
    )
}

/// The response's messages as a stream, ended by the call's status.
fn answers<Resp>(recv: RecvHalf, sender: Option<Sender>) -> Streaming<Resp>
where
    Resp: prost::Message + Default + Send + 'static,
{
    futures::stream::unfold(Some((recv, sender)), |call| async move {
        let (mut recv, sender) = call?;
        match receive::<Resp>(&mut recv).await {
            Ok(Some(message)) => Some((Ok(message), Some((recv, sender)))),
            Ok(None) => None,
            Err(status) => {
                let status = match &sender {
                    Some(sender) => sender.status(status),
                    None => status,
                };
                Some((Err(status), None))
            }
        }
    })
    .boxed()
}
