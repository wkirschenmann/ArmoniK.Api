//! The arenas a channel's sends are done with, kept to be lent again.
//!
//! Where the allocator gives a large allocation back to the system when it is freed, as glibc's
//! does, the next one faults every page of it in again, which costs more than the copy into it.
//! So an arena past `POOLED_FROM` that a send is done with is kept by its channel, under the
//! runtime's ceiling with the charges and given up first when a charge needs the room. Spares are
//! kept until the channel closes, a charge needs their room, or newer ones take their place.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use armonik_transport::grpc::FramedRequest;
use bytes::Bytes;

use crate::held::Held;
use crate::ledger::Ledger;

/// The smallest arena worth keeping: below it the allocator serves a lend from memory it already
/// holds, and keeping one would cost a lock per message for nothing.
pub(crate) const POOLED_FROM: usize = 64 * 1024;

/// A channel's spare arenas.
pub(crate) struct Spares {
    kept: Mutex<Vec<Vec<u8>>>,
    /// At most this many are kept.
    most: usize,
    ledger: Arc<Ledger>,
}

impl Spares {
    /// Spares under `ledger`'s ceiling, at most `most` of them, which the ledger can give up.
    pub(crate) fn new(ledger: &Arc<Ledger>, most: usize) -> Arc<Self> {
        let spares = Arc::new(Self {
            kept: Mutex::new(Vec::new()),
            most,
            ledger: Arc::clone(ledger),
        });
        ledger.register(&spares);
        spares
    }

    fn kept(&self) -> Held<MutexGuard<'_, Vec<Vec<u8>>>> {
        Held::new(self.kept.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The smallest spare of at least `total` bytes and at most an eighth more, if one is kept.
    pub(crate) fn take(&self, total: usize) -> Option<Vec<u8>> {
        if total < POOLED_FROM {
            return None;
        }
        let arena = {
            let mut kept = self.kept();
            let at = kept
                .iter()
                .enumerate()
                .filter(|(_, arena)| fits(arena.capacity(), total))
                .min_by_key(|(_, arena)| arena.capacity())
                .map(|(at, _)| at)?;
            // In order, so the front stays the oldest, which a full pool gives up first.
            kept.remove(at)
        };
        self.ledger.drop_spare(arena.capacity());
        Some(arena)
    }

    /// Keeps `arena` if it is worth keeping and the ceiling has room for it beside what is
    /// charged, in place of the oldest spare when `most` are kept; it is dropped otherwise.
    fn park(&self, mut arena: Vec<u8>) {
        if arena.capacity() < POOLED_FROM {
            return;
        }
        arena.clear();
        let oldest = {
            let mut kept = self.kept();
            let oldest = if kept.len() >= self.most && !kept.is_empty() {
                let oldest = kept.remove(0);
                self.ledger.drop_spare(oldest.capacity());
                Some(oldest)
            } else {
                None
            };
            if self.ledger.keep_spare(arena.capacity()) {
                kept.push(arena);
                #[cfg(feature = "test-hooks")]
                crate::hooks::count_spare_kept();
            }
            oldest
        };
        drop(oldest);
        // Outside the lock, which the trim takes: a charge made while this was being kept may
        // not have seen it.
        self.ledger.trim_spares();
    }

    /// Gives up every spare, for a charge that needs their room.
    pub(crate) fn clear(&self) {
        let gone = std::mem::take(&mut *self.kept());
        for arena in &gone {
            self.ledger.drop_spare(arena.capacity());
        }
    }

    /// `data`, as bytes whose arena comes back here once the last of them is dropped.
    pub(crate) fn returning(self: &Arc<Self>, data: Vec<u8>) -> Returning {
        Returning {
            data,
            spares: Arc::downgrade(self),
        }
    }

    /// `data` as a message, its arena kept here once the message is done with.
    pub(crate) fn message(self: &Arc<Self>, data: Vec<u8>) -> Bytes {
        if data.capacity() < POOLED_FROM {
            return Bytes::from(data);
        }
        Bytes::from_owner(self.returning(data))
    }

    /// `data` as a call's one request, framed in place, its arena kept here once the request is
    /// done with.
    pub(crate) fn request(self: &Arc<Self>, data: Vec<u8>) -> Option<FramedRequest> {
        if data.capacity() < POOLED_FROM {
            return FramedRequest::in_place(data);
        }
        FramedRequest::in_place_owned(self.returning(data))
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.kept().len()
    }
}

impl Drop for Spares {
    fn drop(&mut self) {
        self.clear();
    }
}

fn fits(capacity: usize, total: usize) -> bool {
    capacity >= total && capacity - total <= total / 8
}

/// An arena lent out as a message's bytes, which goes back to its channel's spares when dropped,
/// unless the channel is gone.
pub(crate) struct Returning {
    data: Vec<u8>,
    spares: Weak<Spares>,
}

impl AsRef<[u8]> for Returning {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl AsMut<[u8]> for Returning {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

impl Drop for Returning {
    fn drop(&mut self) {
        if let Some(spares) = self.spares.upgrade() {
            spares.park(std::mem::take(&mut self.data));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena(capacity: usize) -> Vec<u8> {
        Vec::with_capacity(capacity)
    }

    #[test]
    fn a_spare_is_taken_only_if_it_fits_within_an_eighth() {
        let ledger = Arc::new(Ledger::new(0, 0).expect("a valid ledger"));
        let spares = Spares::new(&ledger, 4);
        drop(spares.returning(arena(POOLED_FROM * 2)));
        drop(spares.returning(arena(POOLED_FROM * 4)));

        assert!(spares.take(POOLED_FROM * 3).is_none(), "neither fits");
        let taken = spares
            .take(POOLED_FROM * 2 - 1)
            .expect("the smaller one fits");
        assert!(taken.capacity() >= POOLED_FROM * 2);
        assert!(
            spares.take(POOLED_FROM / 2).is_none(),
            "too small to be kept or taken"
        );
        assert_eq!(spares.len(), 1);
    }

    /// A small arena is dropped, and a full pool gives its oldest spare up for the newest, so a
    /// workload whose sizes change is served by what it sends now.
    #[test]
    fn a_full_pool_keeps_its_newest_spares() {
        let ledger = Arc::new(Ledger::new(0, 0).expect("a valid ledger"));
        let spares = Spares::new(&ledger, 3);
        let keep = |size: usize| drop(spares.returning(arena(size * POOLED_FROM)));
        keep(0);
        assert_eq!(spares.len(), 0, "too small to keep");
        for size in [1, 2, 4, 8] {
            keep(size);
        }
        assert_eq!(spares.len(), 3);
        assert!(
            spares.take(POOLED_FROM).is_none(),
            "the oldest was given up"
        );

        // A spare taken leaves the others in their order, so the oldest is still the first given
        // up: of 4, 8, 16 and 32, the pool keeps 8, 16 and 32.
        assert!(spares.take(2 * POOLED_FROM).is_some());
        keep(16);
        keep(32);
        assert!(
            spares.take(4 * POOLED_FROM).is_none(),
            "the oldest was given up"
        );
        for size in [8, 16, 32] {
            assert!(spares.take(size * POOLED_FROM).is_some(), "{size}");
        }
    }

    /// Spares stay under the ceiling beside what is charged, and a charge that needs their room
    /// takes it from them.
    #[test]
    fn spares_give_their_room_to_a_charge() {
        let ceiling = 4 * POOLED_FROM as u64;
        let ledger = Arc::new(Ledger::new(ceiling, 0).expect("a valid ledger"));
        let spares = Spares::new(&ledger, 8);
        for _ in 0..3 {
            drop(spares.returning(arena(POOLED_FROM)));
        }
        assert!(spares.len() >= 3);

        assert_eq!(ledger.hold_bytes(2 * POOLED_FROM), Ok(()));
        assert_eq!(
            spares.len(),
            0,
            "the charge and the spares passed the ceiling"
        );

        drop(spares.returning(arena(3 * POOLED_FROM)));
        assert_eq!(spares.len(), 0, "no room beside the charge");
        ledger.release_bytes(2 * POOLED_FROM);
    }

    /// A message's arena comes back once its last bytes are dropped, and not before.
    #[test]
    fn a_message_comes_back_with_its_last_bytes() {
        let ledger = Arc::new(Ledger::new(0, 0).expect("a valid ledger"));
        let spares = Spares::new(&ledger, 4);
        let mut data = arena(POOLED_FROM);
        data.resize(10, 7);
        let message = spares.message(data);
        let clone = message.clone();
        assert_eq!(&message[..], &[7; 10]);

        drop(message);
        assert_eq!(spares.len(), 0);
        drop(clone);
        assert_eq!(spares.len(), 1);
    }
}
