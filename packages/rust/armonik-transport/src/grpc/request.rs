use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use tokio::sync::Notify;

/// The bytes ahead of a gRPC message on the wire: its compression flag and its length.
pub const FRAME_PREFIX: usize = 5;

/// A message framed in place: the gRPC prefix, then the message, in one allocation that goes on
/// the wire as it is - a call's one request, or a message of a stream.
#[derive(Debug)]
pub struct FramedMessage(Bytes);

/// Writes the prefix of the message after it; None when there is no room for it or the message
/// no four-byte length carries.
fn prefixed(buffer: &mut [u8]) -> Option<()> {
    let len = u32::try_from(buffer.len().checked_sub(FRAME_PREFIX)?).ok()?;
    buffer[0] = 0;
    buffer[1..FRAME_PREFIX].copy_from_slice(&len.to_be_bytes());
    Some(())
}

impl FramedMessage {
    /// `buffer` holds the message after [`FRAME_PREFIX`] bytes kept for the prefix, which this
    /// writes. None when the buffer has no room for the prefix or the message no four-byte length
    /// carries.
    pub fn in_place(mut buffer: Vec<u8>) -> Option<Self> {
        prefixed(&mut buffer)?;
        Some(Self(Bytes::from(buffer)))
    }

    /// As [`FramedMessage::in_place`], with `buffer` the owner of the message's bytes until the
    /// last of them is dropped: what it does then is its own, such as going back to a pool.
    pub fn in_place_owned<B>(mut buffer: B) -> Option<Self>
    where
        B: AsRef<[u8]> + AsMut<[u8]> + Send + 'static,
    {
        prefixed(buffer.as_mut())?;
        Some(Self(Bytes::from_owner(buffer)))
    }

    /// A message the caller holds elsewhere, framed by a copy.
    pub fn copy_of(message: &[u8]) -> Option<Self> {
        let mut buffer = vec![0; FRAME_PREFIX + message.len()];
        buffer[FRAME_PREFIX..].copy_from_slice(message);
        Self::in_place(buffer)
    }

    /// The empty message: the prefix alone.
    pub fn empty() -> Self {
        Self(Bytes::from_static(&[0; FRAME_PREFIX]))
    }

    /// The message's length, the prefix left out.
    pub fn len(&self) -> usize {
        self.0.len() - FRAME_PREFIX
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn body(&self) -> Bytes {
        self.0.clone()
    }

    pub(crate) fn into_body(self) -> Bytes {
        self.0
    }
}

/// What a call that sends one request has been given of it.
enum Given {
    Waiting,
    Request(FramedMessage),
    /// Taken by the driver, or never to be: the call ended first.
    Closed,
}

struct Slot {
    given: Mutex<Given>,
    arrived: Notify,
}

impl Slot {
    fn given(&self) -> MutexGuard<'_, Given> {
        self.given.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close(&self) {
        let mut given = self.given();
        if matches!(*given, Given::Waiting) {
            *given = Given::Closed;
            drop(given);
            self.arrived.notify_one();
        }
    }
}

/// Where the one request of a call that sends one goes, given once. The call sends nothing, not
/// even its head, until it is given; dropped instead, the call ends cancelled.
pub struct OneRequest(Arc<Slot>);

impl OneRequest {
    /// Gives the call its request, which `make` builds only if the call takes it: false, with
    /// `make` not called, once the call was given one or has ended.
    pub fn give(&self, make: impl FnOnce() -> FramedMessage) -> bool {
        let mut given = self.0.given();
        if !matches!(*given, Given::Waiting) {
            return false;
        }
        *given = Given::Request(make());
        drop(given);
        self.0.arrived.notify_one();
        true
    }
}

impl Drop for OneRequest {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl std::fmt::Debug for OneRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneRequest").finish_non_exhaustive()
    }
}

/// The driver's side of the slot. Dropped before a request came, it refuses any that comes later.
pub(crate) struct RequestSlot(Arc<Slot>);

impl RequestSlot {
    /// The request, once given; None when the call will never have one.
    pub(crate) async fn taken(&self) -> Option<FramedMessage> {
        loop {
            {
                let mut given = self.0.given();
                match std::mem::replace(&mut *given, Given::Closed) {
                    Given::Request(request) => return Some(request),
                    Given::Closed => return None,
                    Given::Waiting => *given = Given::Waiting,
                }
            }
            // A permit stored by a notify that came before this wait is what makes it safe to wait
            // after looking.
            self.0.arrived.notified().await;
        }
    }
}

impl Drop for RequestSlot {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub(crate) fn one_request() -> (OneRequest, RequestSlot) {
    let slot = Arc::new(Slot {
        given: Mutex::new(Given::Waiting),
        arrived: Notify::new(),
    });
    (OneRequest(Arc::clone(&slot)), RequestSlot(slot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefix_is_written_ahead_of_the_message() {
        let mut buffer = vec![0xff; FRAME_PREFIX];
        buffer.extend_from_slice(b"abc");
        let framed = FramedMessage::in_place(buffer).expect("room for the prefix");
        assert_eq!(&framed.body()[..], &[0, 0, 0, 0, 3, b'a', b'b', b'c']);

        assert_eq!(&FramedMessage::empty().body()[..], &[0; FRAME_PREFIX]);
        assert!(FramedMessage::in_place(vec![0; FRAME_PREFIX - 1]).is_none());
    }

    /// An owner holds the request's bytes until the last of them is dropped, and is dropped then.
    #[test]
    fn an_owned_request_is_dropped_with_its_last_bytes() {
        struct Owner(Vec<u8>, Arc<std::sync::atomic::AtomicBool>);
        impl AsRef<[u8]> for Owner {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }
        impl AsMut<[u8]> for Owner {
            fn as_mut(&mut self) -> &mut [u8] {
                &mut self.0
            }
        }
        impl Drop for Owner {
            fn drop(&mut self) {
                self.1.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut buffer = vec![0xff; FRAME_PREFIX];
        buffer.extend_from_slice(b"abc");
        let framed = FramedMessage::in_place_owned(Owner(buffer, Arc::clone(&dropped)))
            .expect("room for the prefix");
        let body = framed.body();
        assert_eq!(&body[..], &[0, 0, 0, 0, 3, b'a', b'b', b'c']);

        drop(framed);
        assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst));
        drop(body);
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// The buffer stays the giver's when the call no longer takes a request: `make` never runs.
    #[tokio::test]
    async fn a_request_is_built_only_for_a_call_that_takes_it() {
        let (request, slot) = one_request();
        drop(slot);
        assert!(!request.give(|| unreachable!("the call has ended")));

        let (request, slot) = one_request();
        assert!(request.give(FramedMessage::empty));
        assert!(!request.give(|| unreachable!("the call has one")));
        assert!(slot.taken().await.is_some());

        let (request, slot) = one_request();
        drop(request);
        assert!(slot.taken().await.is_none(), "never to come");
    }
}
