# The transport ABI

This is the protocol between the library behind `include/armonik_transport_ffi.h` and whoever calls
it. It is written for someone building a binding - Python, Java, C++, anything - who needs to know
how the two sides keep in step, and who never has to read the Rust the library is written in.

**The header preamble is normative; this document explains it.** Where the two disagree, the header
wins. Nothing here adds a rule: each section says what a rule in the header means for the side
implementing the other end, and why it is there. A rule whose reason is stated is a rule someone
keeps.

## What crosses this boundary

An HTTP/2 request, and nothing above it. The library opens a request from a set of headers, streams
bytes in both directions, and reports the response headers, body chunks and trailers. No gRPC
vocabulary crosses: there is no call kind, no message, no `grpc-status`, no gRPC deadline and no
retry policy. This is the transport underneath a gRPC stack, not the stack.

Four things are therefore the consumer's, and none of them is done here.

Framing. A read event carries whatever the connection delivered - part of a message, one message, or
several. A consumer speaking gRPC above this reassembles the length-prefixed messages itself, and
copes with any split.

Status. The HTTP status arrives in the response header blob under the `:status` key, as decimal
ASCII. A `grpc-status` arrives as an ordinary key in the header or the trailer blob, and the library
neither reads nor acts on it. A request that completes with `AK_OK` is one whose stream ended
cleanly; whether the call succeeded is a question about a header this library only carried.

Deadlines. There is a `Timeout` option, and it bounds the whole life of a request from this side
only. It is not a gRPC deadline: no `grpc-timeout` header goes out because of it, and the peer never
learns of it. A consumer that wants the server to know sends the header itself.

Retry. A replay is a new request. Which failures are worth one is keyed on a status this level never
sees, so the retry schedule in the configuration vocabulary is deliberately read and not applied
here.

## Configuration

A client is created from one UTF-8 JSON document: a flat object whose values are strings, naming
options in the transport's own vocabulary. An option the document does not name reads as its own
default, so a document says only what it changes. `include/http_config.schema.json` is that
vocabulary in full, and is what an options class in another language is generated from.

Creation is synchronous and opens no socket. The options are read, the certificates they name are
loaded, and the connector is assembled; the first connection is opened by the first request. So a
host application may create a client from a thread it cannot block, a mistyped option is reported
there rather than surfacing much later as a failed request, and a failure at creation is always a
configuration failure - never a connection failure, which would send its reader to check network
reachability over a mistake in a file.

Every option in the vocabulary except the retry schedule - `MaxAttempts`, `InitialBackOff`,
`MaxBackOff`, `BackOffMultiplier` - is applied by this library. That list is kept honest by a test
rather than by good intentions: every option the schema declares must appear on one of two lists,
applied or not applied, each with a note saying what it reaches or why this is the wrong layer to
read it, and an option added to the vocabulary fails the build until somebody decides which it is. A
binding author can therefore take the schema as complete, and take an option outside the retry
schedule as one that does something.

## Values that cross

A buffer handed *to* the library is borrowed for the duration of that call, and the library never
releases it. A null pointer or a zero length means absent.

A buffer handed *back* by the library is owned, comes only through a synchronous out-parameter, and
is given up by exactly one release call with the value passed back unchanged. Its pointer and length
are a read-only view; the third field names the real owner and is opaque. The two are kept apart so
that a buffer the library already holds can cross without being copied: what owns those bytes may be
a reference-counted view into a larger allocation, so the pointer on its own is not something that
can be freed. Copying the triple is harmless; releasing more than one copy of it is not. The zeroed
value means "no data" and is always safe to release.

Lists of pairs - request headers, response headers, trailers - travel as one blob whose layout the
header gives. Its integers are in native byte order, because this is an in-process ABI and not a
wire format. Keys may repeat and keep the order they were given. Values are opaque bytes: the base64
of a `-bin` header is the consumer's convention, and passes through untouched.

Handles are reference-counted and thread-safe. A release gives back one reference rather than
destroying anything, so a call already under way when another thread releases the same handle
finishes normally. What is not allowed is using a handle after your own release. The library
recognises most such uses and answers `AK_INVALID_HANDLE`, but that is a diagnostic and not a
guarantee: an address given back can eventually be handed to a new handle of the same kind.

Every entry point uses the C calling convention, on every platform and every architecture. On 32-bit
x86 that is not the default a managed runtime picks, and getting it wrong is a stack cleaned up
twice.

## The event model

A request is started with one callback, and the library is the only thing that ever invokes it.
Every entry point posts a command and returns; a single task per request drives it and is the sole
emitter of its events.

That is what makes delivery serialised per request: the callback is never entered twice at once for
the same request, so per-request state in a binding needs no lock of its own. It may be entered
concurrently for two different requests, so state shared between requests does need one.

The `ctx` value given at the start is an opaque token. The library never dereferences it, only hands
it back with each event, and it is how a binding finds its own object for the request being
reported.

## Arming

Reads and writes are pulled, not pushed. One armed read produces exactly one event: a body chunk, or
the completion. One armed write produces exactly one event: the write acknowledgement, or the
completion. At most one read and at most one write are armed at a time on a request; a second is
refused synchronously with `AK_INVALID_STATE` rather than reported as an event, as is a write after
the request body has been ended.

This is where flow control comes from. The queue behind a request body holds one chunk, so the
peer's HTTP/2 window becomes back-pressure the consumer feels, and demand for response bytes is what
pulls them off the connection: no unbounded buffer sits between the two sides, in either direction.
A binding that arms the next read as soon as the last one is handled gets continuous streaming; one
that arms on demand gets flow control for nothing.

Two events are not armed by the consumer, because starting the request arms them: the response
headers, at most once and before any event carrying response data, and the completion. A write
acknowledgement says only that the connection took the chunk and one more may be armed - not that
the bytes reached the server.

## The completion, and who owns what

A completion event arrives exactly once per request and is always the last thing that request
reports. On every path: a clean end of the response, a connection that could not be made, one that
broke afterwards, a cancellation, an elapsed timeout, a failure inside the library. Anything still
armed is resolved by it rather than by an event of its own. After it, nothing fires again for that
request, ever.

That is where the consumer gives back whatever it rooted for `ctx` - the handle, the pinned
delegate, the global reference - and it is the only place. If starting the request returned anything
but `AK_OK`, no event ever arrives and nothing was rooted.

Releasing the request handle does not end this. A release cancels the request and gives back the
consumer's reference; the completion still arrives. A binding that frees its root at release is a
use-after-free: the library calls back into memory that is gone. There is one ownership rule and no
second one for any path.

A release cannot simply stop the work, and that is deliberate. The task is what resets the stream
and lets the pool have its connection back, so stopping it where it stands is how a peer is left
waiting on a call nobody is on the other end of - and how a consumer is left waiting for a
completion that never comes.

On a clean end the completion's code is `AK_OK` and its payload is the trailers as a key/value blob,
possibly with no pairs in it, which is what a stream that ended without trailers looks like.
Otherwise the code is a failure and the payload is that failure as a UTF-8 message.

## Borrowed payloads

An event payload is borrowed for the duration of the invocation and is invalid the moment the
callback returns. Copy what is needed before returning, and never release it: nothing on the event
path is owned by whoever receives it. The pointer is into a buffer the library still owns, which is
what lets a response chunk reach the consumer without being copied first.

## Threading and reentrancy

Events are emitted from the library's own threads. None is ever delivered during a call the consumer
made: every entry point posts a command and returns, so no entry point can call back into the
consumer on the consumer's own thread.

A callback must not block. It runs on a thread that is also driving other requests, and holding it
stalls them. Hand the event to your own queue and return.

Nothing may escape the callback back into the library: no exception, no unwind, no non-local jump. A
foreign exception crossing a C frame is not something any language defines, so a binding catches
everything at the boundary of its own callback. The library guards its side, and an unwind it
catches ends the request with `AK_INTERNAL_PANIC` rather than corrupting anything, but that is a
diagnosis of a bug and not a way to signal one.

Re-entry is limited, rather than forbidden. A `READ_DONE` callback may call `ak_request_read` to arm
the next read, and a `WRITE_DONE` callback may call `ak_request_write` to arm the next write. The
corresponding armed flag is cleared, and its internal lock released, before the callback starts;
the newly armed operation cannot emit until the current callback has returned. No other downcall
for that request is permitted from its callback in this ABI revision. In particular, do not wait,
release the request or its `ctx`, close the send side, or cancel reentrantly.

## Ordering

Two events with no causal relation between them have no promised order, and this contract never
promises one. A consumer that relies on an order has read into the contract something that is not
there.

Concretely, a peer that resets the stream while its trailers are in flight ends the same response
either way: `AK_OK` if the trailers arrived first and the stream ended cleanly, or `AK_TRANSPORT` if
the reset did and the stream is reported broken. The same holds for the two ways a request is given
up on: one cancelled at about the moment its configured `Timeout` elapses completes as
`AK_CANCELLED` or as `AK_TIMEOUT`, and which one is a race nobody can call. Downstream code that
filters on one of such a pair must accept the other. The library's own tests assert the set of
acceptable outcomes rather than a single one, which is the same discipline a binding's tests need.

## Errors

`AK_OK` is 0 and means success. Every failure is negative, and there is no positive space, because
no gRPC status is reported at this level. The value -7 is reserved and never returned: the gap costs
nothing and keeps a consumer that compares against it matching nothing rather than something else.

A failure reaches a consumer two ways. A synchronous call returns the code and, through an optional
out-parameter, an owned message. An asynchronous failure arrives as the completion, whose code is
the failure and whose payload is the message.

The message is one flat UTF-8 string with the whole cause chain flattened into it. A C ABI has no
inner exception to follow, and there is no error chain left to walk on the other side, so a message
that stopped at the outermost error would drop the only part worth reading: "could not establish TLS
connection to the remote" says nothing to act on, and the cause naming the unreadable file or the
key that does not match its certificate is the entire diagnosis.

The codes divide by who has to do something. A null argument, a buffer that is not the UTF-8 it had
to be, an invalid handle and an invalid state are the caller's doing and are always synchronous. An
invalid configuration is the document or something it names. A connection failure and a transport
failure are the same distinction drawn at one moment: before any response header arrived, nothing
was answered; after them, the stream broke under a response that had started. Cancellation and
timeout are the two ways a request is given up on. An internal failure and a caught panic are always
bugs in the library, and the message is what to report.

A delivered build says what went wrong and never whereabouts in its own source it happened: source
locations and positions inside the configuration document are removed. A build that keeps them
exists for whoever is debugging the library. The messages are otherwise the same and the ABI is
identical, which is what makes the two interchangeable - a consumer picks one by which library it
ships, not by how it calls.

Treat an unknown result code as a failure, and an unknown event kind as one to ignore. Zero is never
a valid event kind, so a zeroed callback argument is never a valid event.

## Failure-containment limits in this revision

This revision contains failures at an entry point or at one request; it does not implement a
runtime-wide quarantine protocol. A rejected downcall returns a status. A panic caught while the
request task is being driven reaches that request as a terminal `AK_INTERNAL_PANIC`. The panic
guards keep an unwind on the side where it originated.

The callback ABI has no central `operation_id`, owned event queue, runtime start gate, or shutdown
barrier. If a binding can no longer resolve `ctx`, loses an event while translating it, or detects a
shared invariant violation, there is no call in ABI version 1 that can atomically reject all new
requests, fail every known request, and prove that the runtime is quiescent. Ignoring an unknown
event kind is safe only under the additive-versioning rules above; it is not a general recovery
rule for an unknown request, stale context, duplicate terminal event, or ownership contradiction.

A binding must therefore keep every callback root valid until its request's real terminal event.
It must not claim that a damaged runtime is safe to unload or recreate. A host that suspects a
process-wide or memory-integrity failure needs a policy outside this ABI, up to process termination
or process isolation. Malformed network input remains a request or connection failure; it must not
be allowed to select callback contexts or native handle identities.

The replacement design's quarantine, join and `FailedUnquiesced` protocol is specified for review
in [`DESIGN.md`](DESIGN.md#410-protocole-de-sortie-sur-rupture-dinvariant); its insertion into the
current PR stack is tracked in [`PR_REMEDIATION.md`](PR_REMEDIATION.md). Neither document turns it
into a promise made by ABI version 1.

## The committed artefacts

Three files under `include/` are generated and committed, so that a change to this contract shows up
in a diff rather than only in a compiled library. A binding author needs that directory and nothing
else.

`armonik_transport_ffi.h` is the whole contract in one file: the entry points, the enumerations, the
structures, and in its preamble everything a signature cannot carry. `NativeMethods.g.cs` is the
same surface as C# declarations, targeted at netstandard2.0. `http_config.schema.json` is the option
vocabulary the configuration document is written in.

The first two are rewritten by the build on every build; the third is regenerated by hand, because
what it describes lives in a component the build script cannot reach. All three are pinned by tests:
one compares the schema against the vocabulary as it stands now, and others check that every entry
point, every constant and every value is present and unchanged. They are also pinned to LF line
endings on every platform, since they are compared byte for byte.

## Versioning

`ak_abi_version` reports the revision the loaded library speaks; the header carries the revision it
was generated from as a constant. Compare the two before trusting anything else. Asking is what
turns a mismatch into a diagnosis: a host process loads one native module and every add-in in it
shares whichever was loaded first, so an add-in that did not bring its own has no other way to find
out what it got, and reaching for an entry point that is not there surfaces as an obscure failure
from somewhere unrelated.

Within one revision the ABI is additive only. What may be added: entry points, result codes, event
kinds, keys in a blob. What may never change: the name, the signature or the calling convention of
an existing entry point; the layout of an existing structure; the numeric value or the meaning of an
existing constant; the ownership and lifetime rules above. That is what lets a binding keep working
against a library newer than itself, and it is why treating an unknown code as a failure and an
unknown kind as one to ignore is not defensive habit but the other half of the same promise.
