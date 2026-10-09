use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::{Buf, Bytes};
use tokio::sync::Notify;

use super::compression::Charge;

/// The bytes ahead of a gRPC message on the wire: its compression flag and its length.
pub const FRAME_PREFIX: usize = 5;

/// A message framed in place: the gRPC prefix, then the message, in one allocation that goes on
/// the wire as it is - a call's one request, or a message of a stream.
#[derive(Debug)]
pub struct FramedMessage(Bytes);

/// The compression flag of a message its peer inflates.
const COMPRESSED: u8 = 1;

/// A message's bytes and what counts them, which goes with the last reference to them.
struct Charged {
    buffer: Vec<u8>,
    _charge: Charge,
}

impl AsRef<[u8]> for Charged {
    fn as_ref(&self) -> &[u8] {
        &self.buffer
    }
}

/// Writes the prefix of the message that starts at `headroom` into the bytes just before it, and
/// says where it starts; None when the headroom has no room for it, the buffer no message, or the
/// message no four-byte length carries.
fn prefixed(buffer: &mut [u8], headroom: usize) -> Option<usize> {
    prefixed_flagged(buffer, headroom, 0)
}

fn prefixed_flagged(buffer: &mut [u8], headroom: usize, flag: u8) -> Option<usize> {
    let at = headroom.checked_sub(FRAME_PREFIX)?;
    let len = u32::try_from(buffer.len().checked_sub(headroom)?).ok()?;
    buffer[at] = flag;
    buffer[at + 1..headroom].copy_from_slice(&len.to_be_bytes());
    Some(at)
}

impl FramedMessage {
    /// `buffer` holds the message after [`FRAME_PREFIX`] bytes kept for the prefix, which this
    /// writes. None when the buffer has no room for the prefix or the message no four-byte length
    /// carries.
    pub fn in_place(buffer: Vec<u8>) -> Option<Self> {
        Self::in_place_after(buffer, FRAME_PREFIX)
    }

    /// `buffer` holds the message after `headroom` bytes, the prefix written into the last
    /// [`FRAME_PREFIX`] of them and the message going out from there. A headroom larger than the
    /// prefix lets the message start at an offset of the caller's choosing, such as an aligned one.
    pub fn in_place_after(mut buffer: Vec<u8>, headroom: usize) -> Option<Self> {
        let at = prefixed(&mut buffer, headroom)?;
        let mut bytes = Bytes::from(buffer);
        bytes.advance(at);
        Some(Self(bytes))
    }

    /// As [`FramedMessage::in_place_after`], with `buffer` the owner of the message's bytes until
    /// the last of them is dropped: what it does then is its own, such as going back to a pool.
    pub fn in_place_owned_after<B>(mut buffer: B, headroom: usize) -> Option<Self>
    where
        B: AsRef<[u8]> + AsMut<[u8]> + Send + 'static,
    {
        let at = prefixed(buffer.as_mut(), headroom)?;
        let mut bytes = Bytes::from_owner(buffer);
        bytes.advance(at);
        Some(Self(bytes))
    }

    /// `buffer` holds a message already compressed after [`FRAME_PREFIX`] bytes kept for the
    /// prefix, which this writes with the compressed flag set. The message owns `charge`, which is
    /// dropped with the last of its bytes.
    pub(crate) fn compressed_in_place(mut buffer: Vec<u8>, charge: Option<Charge>) -> Option<Self> {
        prefixed_flagged(&mut buffer, FRAME_PREFIX, COMPRESSED)?;
        Some(Self(match charge {
            Some(charge) => Bytes::from_owner(Charged {
                buffer,
                _charge: charge,
            }),
            None => Bytes::from(buffer),
        }))
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

    /// The message's bytes, the prefix left out.
    pub(crate) fn payload(&self) -> &[u8] {
        &self.0[FRAME_PREFIX..]
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

    /// After a larger headroom, the prefix takes its last bytes and the message goes out from
    /// there, the headroom's first bytes left behind.
    #[test]
    fn the_prefix_takes_the_end_of_a_larger_headroom() {
        let mut buffer = vec![0xff; 8];
        buffer.extend_from_slice(b"abc");
        let framed = FramedMessage::in_place_after(buffer, 8).expect("room for the prefix");
        assert_eq!(&framed.body()[..], &[0, 0, 0, 0, 3, b'a', b'b', b'c']);
        assert_eq!(framed.len(), 3);

        assert!(FramedMessage::in_place_after(vec![0; 8], FRAME_PREFIX - 1).is_none());
        assert!(FramedMessage::in_place_after(vec![0; 7], 8).is_none());
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
        let mut buffer = vec![0xff; 8];
        buffer.extend_from_slice(b"abc");
        let framed = FramedMessage::in_place_owned_after(Owner(buffer, Arc::clone(&dropped)), 8)
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
