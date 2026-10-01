use std::ffi::c_void;

/// Takes back a box this library lent, named by the pointer it handed the host.
///
/// The tag tells one kind of box from another, and either from a pointer that is neither. What it
/// cannot tell is a box already taken back: the read below happens before the check, so a second
/// return reads eight bytes out of a freed allocation, and whether the tag survived there is the
/// allocator's business. The header makes a second return of any owner undefined behaviour
/// rather than a no-op, because that is what it is - a token would be needed to make it
/// reportable, and the event path is where that token would be looked up.
///
/// Unaligned, because nothing promises the host's pointer is aligned for a `u64` - it is aligned
/// for whatever the host thinks `owner` points at, which is `void`.
///
/// # Safety
///
/// `owner` must be null, or a pointer this library handed out and the host has not given back,
/// to a `T` whose first field is the tag.
pub(crate) unsafe fn take_tagged<T>(owner: *mut c_void, tag: u64) -> Option<Box<T>> {
    if owner.is_null() {
        return None;
    }
    if unsafe { owner.cast::<u64>().read_unaligned() } != tag {
        return None;
    }
    Some(unsafe { Box::from_raw(owner as *mut T) })
}
