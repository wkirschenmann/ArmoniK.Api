/* C ABI over the ArmoniK Rust gRPC engine.
 *
 * Written by hand and committed, so an ABI change shows up in review rather than in a generated
 * file nobody reads. `spec/armonik_grpc_ffi/design.md` carries the reasoning; this carries the
 * contract.
 *
 * Two rules run through all of it:
 *
 *   - A handle is a token, not a pointer. It names a slot and a generation, so a downcall on
 *     something the runtime has reclaimed reports AK_STATUS_HANDLE_STALE instead of faulting.
 *   - Memory crosses in one direction at a time. What the library hands over, the host gives
 *     back exactly once - a payload through ak_event_consumed, a lent buffer through
 *     ak_call_send_message or ak_return_call_buffer - and the runtime cannot finish until it has.
 *
 * Integers in the key/value blobs are in native byte order. This ABI runs in-process between this
 * library and its host; it is not a wire format.
 */

#ifndef ARMONIK_TRANSPORT_FFI_H
#define ARMONIK_TRANSPORT_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AK_ABI_VERSION 1

/* === Status ===
 *
 * Returned by every entry point that can fail. ak_event_consumed, ak_return_call_buffer and
 * ak_channel_release are void because a wrong token is a host bug the ABI cannot report anywhere
 * useful; ak_runtime_status and ak_abi_version return their answer.
 */
typedef enum {
    AK_STATUS_OK                = 0,
    AK_STATUS_HANDLE_STALE      = 1, /* the object is gone; the token names nothing */
    AK_STATUS_SLOT_BUSY         = 2, /* this call's send window is full - backpressure, not an
                                        error; retry when a WRITE_DONE arrives */
    AK_STATUS_INVALID_ARG       = 3, /* a null pointer, or a struct whose size prefix does not
                                        match any known version */
    AK_STATUS_INTERNAL          = 4, /* a fault the ABI cannot attribute */
    AK_STATUS_BUDGET_BUSY       = 5, /* the runtime-wide byte ceiling is reached - poll
                                        ak_runtime_memory_usage and retry */
    AK_STATUS_INVALID_STATE     = 6, /* a valid handle at the wrong moment: a send after the
                                        terminal, a start while stopping, a destroy before
                                        quiescence. A guard refused, which is not a fault */
    AK_STATUS_MESSAGE_TOO_LARGE = 7, /* len exceeds the ceiling itself, so no return by anyone
                                        will ever make room. Permanent; do not retry */
} ak_status;

/* === Handles === */

typedef uint64_t ak_handle;
#define AK_HANDLE_NONE ((uint64_t)0)

/* Chosen by the host, returned in each callback for its call. Opaque: never dereferenced here.
 * Must stay valid until the call's terminal (AK_EVENT_STATUS). */
typedef void *ak_call_ctx;

/* === Buffers === */

/* Bytes the host lends this library for the duration of one downcall. */
typedef struct {
    const uint8_t *ptr;
    size_t len;
} ak_bytes_in;

/* Owned by the host after reception, until ak_event_consumed.
 *
 * owner, not ptr, identifies the allocation: the view may be a slice of a larger one. owner ==
 * NULL means unowned, never "empty" - a zero-length payload that took a delivery credit carries a
 * real owner and MUST be consumed, because the credit comes back with the acquittal and not with
 * the bytes. */
typedef struct {
    const uint8_t *ptr;
    size_t len;
    void *owner;
} ak_bytes;

/* Lent by ak_get_call_buffer out of the call's arena. The host writes len bytes and gives it back
 * exactly once, by ak_call_send_message or ak_return_call_buffer. This library never reclaims a
 * lent buffer on its own - not on cancellation, not on channel close - which is what removes the
 * race between a writing thread and a cancelling one. */
typedef struct {
    uint8_t *ptr;
    size_t len;
    void *owner;
} ak_buffer;

/* === Runtime state === */

typedef enum {
    AK_RUNTIME_RUNNING           = 1, /* operational, accepts channels and calls */
    AK_RUNTIME_GRPC_STOPPING     = 2, /* start gate closed, channels closing */
    AK_RUNTIME_GRPC_STOPPED      = 3, /* the gRPC side is done; the host may still hold memory */
    AK_RUNTIME_QUIESCENT         = 4, /* and nothing of it is outstanding either */
    AK_RUNTIME_FAILED_UNQUIESCED = 5, /* quiescence impossible, destroy refused */
} ak_runtime_state;

/* Only QUIESCENT permits ak_runtime_destroy or unloading the library. The host reaches it by
 * acting, not by waiting: while it still holds something the status stays STOPPED, and the
 * AK_EVENT_SHUTDOWN_COMPLETE callback says so through its host_debt field. Polling for QUIESCENT
 * before returning what it holds is therefore a deadlock. */

/* === Events === */

typedef enum {
    AK_EVENT_INITIAL_METADATA   = 1, /* payload = metadata blob (owned) */
    AK_EVENT_MESSAGE            = 2, /* payload = message bytes (owned) */
    AK_EVENT_STATUS             = 3, /* terminal - payload = reason + trailers (owned) */
    AK_EVENT_WRITE_DONE         = 4, /* an accepted send is settled; its slot is already free */
    AK_EVENT_SHUTDOWN_COMPLETE  = 5, /* the runtime has stopped running */
    AK_EVENT_RESOURCES_RELEASED = 6, /* and now nothing of it is outstanding */
} ak_event_kind;

/* Carried by AK_EVENT_SHUTDOWN_COMPLETE and by nothing else. */
typedef enum {
    AK_HOST_NOTHING_TO_RETURN = 0, /* nothing of ours is in your hands; no second event */
    AK_HOST_MUST_RETURN       = 1, /* consume the payloads, return the buffers;
                                      AK_EVENT_RESOURCES_RELEASED follows */
} ak_host_debt;

/* On the callback's stack; no lifecycle of its own. The payload is owned when owner is not NULL.
 *
 * WRITE_DONE, SHUTDOWN_COMPLETE and RESOURCES_RELEASED carry no payload: ptr == NULL, len == 0,
 * owner == NULL. ak_event_consumed on such a payload is a no-op, so a host may route every event
 * through one path. */
typedef struct {
    ak_event_kind kind;
    ak_bytes      payload;
    int32_t       status_code; /* gRPC status, AK_EVENT_STATUS only */
    ak_host_debt  host_debt;   /* AK_EVENT_SHUTDOWN_COMPLETE only */
} ak_event;

/* The function pointer must stay valid for the runtime's lifetime, and runtime_ctx until the
 * runtime's last event.
 *
 * Data callbacks (INITIAL_METADATA, MESSAGE, STATUS) are serialized per call and concurrent
 * between calls. WRITE_DONE may arrive in parallel with any of them, including for the same call:
 * a per-call lock in the handler would hold the slot release hostage behind a slow message
 * handler. */
typedef void (*ak_callback)(void *runtime_ctx, void *call_ctx, const ak_event *event);

/* === Options ===
 *
 * Every options struct starts with struct_size, which the host sets to sizeof of its own
 * definition. A size this library does not know is refused with AK_STATUS_INVALID_ARG rather than
 * read: it names a version whose fields are not the ones read here.
 */

typedef struct {
    uint32_t struct_size;
    uint32_t worker_threads;  /* zero leaves the choice to the runtime */
    uint64_t memory_ceiling;  /* bytes lent buffers may occupy at once; zero is no ceiling */
} ak_runtime_config;

typedef struct {
    uint32_t    struct_size;
    ak_bytes_in method;    /* "/Service/Method", not NUL-terminated */
    ak_bytes_in metadata;  /* the key/value blob below; may be empty */
} ak_call_start_options;

/* === Blobs ===
 *
 * Metadata travels as:
 *
 *     uint32_t count
 *     repeated count times {
 *         uint32_t key_len;   key_len bytes
 *         uint32_t value_len; value_len bytes
 *     }
 *
 * A key ending in "-bin" carries raw bytes; every other value is printable ASCII. An empty blob
 * and a blob of count zero mean the same thing.
 *
 * The AK_EVENT_STATUS payload is:
 *
 *     uint32_t message_len; message_len bytes    the reason, UTF-8
 *     the trailing metadata, as a blob
 *
 * The status code itself is ak_event.status_code, not part of the payload.
 */

/* === Observation === */

typedef struct {
    uint32_t payloads_owed;       /* delivered, not yet ak_event_consumed */
    uint32_t buffers_lent;        /* out of the arena, not yet given back */
    uint32_t callbacks_in_flight; /* the runtime's own, informational */
    int32_t  terminal_delivered;  /* 0 or 1 */
} ak_call_debt;

typedef struct {
    uint64_t bytes_used; /* occupied against the ceiling, atomic snapshot */
    uint64_t ceiling;    /* the configured limit; zero is no ceiling */
} ak_memory_usage;

/* === Runtime lifecycle === */

ak_status ak_runtime_create(const ak_runtime_config *config,
                            ak_callback callback,
                            void *runtime_ctx,
                            ak_handle *out);

ak_runtime_state ak_runtime_status(ak_handle runtime);

/* Closes the start gate and drains. AK_EVENT_SHUTDOWN_COMPLETE follows. Idempotent. */
ak_status ak_runtime_begin_shutdown(ak_handle runtime);

/* Frees the runtime. Refused before AK_RUNTIME_QUIESCENT, and that is the only reason.
 *
 * Live call and channel handles do not block it: destroy stales every handle of this runtime at
 * once, and a later downcall on one returns AK_STATUS_HANDLE_STALE. A handle names runtime-owned
 * state, so the runtime may reclaim it; a payload or a lent buffer is memory the host may still
 * be reading or writing, so only the host can end it. */
ak_status ak_runtime_destroy(ak_handle runtime);

ak_status ak_runtime_memory_usage(ak_handle runtime, ak_memory_usage *out);

/* === Channel === */

/* Creates a channel from a config JSON. Synchronous and performs no I/O, so it fails only on a
 * bad config. The JSON carries at least {"endpoint": "http://host:port"}, and optionally
 * connect_timeout_ms, user_agent, max_recv_message_size, delivery_credits and
 * max_sends_in_flight. An option spelled wrong is refused, not ignored.
 *
 * The last two are the two windows, and they mirror each other. delivery_credits bounds the
 * payloads of one call outstanding at once, and the host chooses it because the host is what
 * has to hold them; max_sends_in_flight bounds the buffers one call may have out, counting
 * those being filled and those awaiting their WRITE_DONE. Zero is refused for either; both
 * default to 1. */
ak_status ak_channel_create(ak_handle runtime, ak_bytes_in config_json, ak_handle *out);

/* Frees the channel. Calls under way are cancelled. */
void ak_channel_release(ak_handle channel);

/* === Call ===
 *
 * There is no ak_call_release, on purpose. Every resource a call lends out is given back through
 * something the runtime already observes, so it knows when a terminal call owes nothing and takes
 * the handle back itself. Two consequences: the handle goes stale at a moment the host does not
 * choose, and abandoning a call is still ak_call_cancel followed by consuming through to the
 * terminal - dropping a payload on the floor keeps the runtime alive.
 */

ak_status ak_call_start(ak_handle channel,
                        const ak_call_start_options *options,
                        void *call_ctx,
                        ak_handle *out);

/* Lends a buffer out of the call's arena to serialize into. The exact length is known before the
 * first byte is written, so no growable writer is needed.
 *
 * One unfilled buffer at a time, whatever max_sends_in_flight says: asking for a second while
 * still holding one is AK_STATUS_INVALID_STATE, a host bug rather than backpressure. The window
 * counts those being filled and those committed and awaiting their WRITE_DONE; when it is full
 * the refusal is AK_STATUS_SLOT_BUSY, whose wake-up is this call's next WRITE_DONE. That wake-up
 * is only meaningful because a host eligible to ask holds nothing. AK_STATUS_BUDGET_BUSY is the runtime-wide ceiling, and
 * has no single event announcing room: poll ak_runtime_memory_usage. AK_STATUS_MESSAGE_TOO_LARGE
 * is permanent. On every refusal no buffer is lent and *out is untouched. */
ak_status ak_get_call_buffer(ak_handle call, size_t len, ak_buffer *out);

/* Commits a lent buffer as the next message. Ownership passes back to this library.
 *
 * AK_EVENT_WRITE_DONE settles an accepted send and frees its slot from the moment the event is
 * emitted, not when the callback returns - so a host woken by it may ask for a buffer from inside
 * the callback. It says nothing about the network: the message may have been written, or
 * abandoned because the call was cancelled. It arrives exactly once per accepted send, in send
 * order, and always before the terminal. */
ak_status ak_call_send_message(ak_handle call, ak_buffer buffer);

/* Gives a lent buffer back unused. Legal on a cancelled or terminal call: it is the only exit for
 * a buffer whose send is refused, and the call is not reclaimed until it happens.
 *
 * Takes no call handle: the buffer determines its call. A refused ak_call_send_message therefore
 * leaves the buffer with the host, exactly as it was lent. */
void ak_return_call_buffer(ak_buffer buffer);

/* Signals end of sending. No ak_call_send_message after this. */
ak_status ak_call_end_send(ak_handle call);

/* Cancels the call, which then reaches a terminal carrying CANCELLED.
 *
 * Asynchronous: the request takes effect when the call's task observes it, so callbacks already
 * committed may still arrive after this returns. INITIAL_METADATA is never skipped. */
ak_status ak_call_cancel(ak_handle call);

/* What the call still owes. Purely observational; it is legal never to call it. It exists because
 * without a release downcall nothing reports a forgotten ak_return_call_buffer synchronously. */
ak_status ak_call_debt_of(ak_handle call, ak_call_debt *out);

/* === Utilities === */

int ak_abi_version(void);

/* Signals that the host has consumed an event's payload. Two things at once:
 *
 *   1. frees the native memory;
 *   2. arms reception of the next event for that call.
 *
 * At most one non-consumed payload per call by default: while the host owes it, the runtime
 * withholds the next data callback. Only a terminal still goes out with the credit spent.
 *
 * Remains legal, and required, after the terminal: the call is not reclaimed until it happens. */
void ak_event_consumed(ak_bytes payload);

#ifdef __cplusplus
}
#endif

#endif /* ARMONIK_TRANSPORT_FFI_H */
