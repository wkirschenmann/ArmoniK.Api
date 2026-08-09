//! The kinds of event a request reports through the callback it was started with.
//!
//! An enum rather than a list of constants, for the reason `status` gives: it is the one form both
//! generators carry across with its values intact, and it names the values a plain `int32_t` may
//! take without promising a width.
//!
//! The `AK_EVENT_` prefix is written here rather than acquired in a generator's rename table, so
//! that one name reads the same in the header, in the C# bindings and in this source.

/// What a request is reporting.
///
/// Zero is not one of them, so a zeroed `kind` is never a valid event. A caller ignores a kind it
/// does not know: the ABI is additive, and a library newer than its caller may report events that
/// caller has never heard of.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ak_event {
    /// The response headers arrived. The payload is a key/value blob, with the HTTP status in it
    /// under the `:status` key as decimal ASCII, and the code is `AK_OK`.
    ///
    /// At most once per request, and before any other event that carries response data.
    AK_EVENT_RESPONSE_HEADERS = 1,
    /// The read armed by `ak_request_read` produced body bytes. The payload is the chunk, borrowed
    /// for the duration of the call, and the code is `AK_OK`.
    ///
    /// A chunk is whatever the connection delivered, not a message: there is no framing at this
    /// level, so a reader copes with any split.
    AK_EVENT_READ_DONE = 2,
    /// The request is over, and no further callback is made for it.
    ///
    /// The code is `AK_OK` when the response ended cleanly, and the payload is then the trailers as
    /// a key/value blob - possibly empty, which is what a clean end of stream without trailers looks
    /// like. Otherwise the code is one of the failures and the payload is that failure as a UTF-8
    /// message, with its whole cause chain flattened into it.
    AK_EVENT_COMPLETED = 3,
}

impl ak_event {
    /// This kind as it crosses the ABI.
    pub(crate) const fn kind(self) -> i32 {
        self as i32
    }
}

#[cfg(test)]
mod tests {
    use super::ak_event::*;
    use super::*;

    /// Every kind, so that one added later is weighed against the others.
    const ALL: &[ak_event] = &[
        AK_EVENT_RESPONSE_HEADERS,
        AK_EVENT_READ_DONE,
        AK_EVENT_COMPLETED,
    ];

    #[test]
    fn no_kind_is_zero_and_no_two_share_a_value() {
        let mut kinds: Vec<i32> = ALL.iter().map(|event| event.kind()).collect();
        let total = kinds.len();
        kinds.sort_unstable();
        kinds.dedup();

        assert_eq!(kinds.len(), total, "two kinds share a value");
        assert!(
            !kinds.contains(&0),
            "zero is not a kind, so that a zeroed callback argument is never a valid event"
        );
    }
}
