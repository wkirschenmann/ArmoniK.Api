# Retry admission: one health estimate for the rate limit, the retries and the backoffs

Status: proposal, 2026-10-07; the answers to section 8 of the same day are recorded there. Research
only; nothing here binds the code until the open questions are decided. The rate limiter it builds
on is merged on `wk/feat/phase1` (`90249e09b`, from `wk/feat/option-rate-limit`, tip `2ae62df47`),
and "the branch" below means that code.

## 1. The question

A channel decides three things about a request, and the engine has three mechanisms for them:

- **when a call's first attempt may start**: `Grpc.RateLimit`, a fixed window like tower's, built on
  the branch above;
- **whether a failed call is tried again, and when**: the retry policy (gRFC A6's `retryPolicy`),
  built on `main`, with A6's `retryThrottling` still to build (T6.15);
- **how long a call waits for a connection**: wait-for-ready, whose waits are the connection
  backoff, built on `main`.

They should be one mechanism. The budget for retries follows the server's health, as Google's
client-side adaptive throttling does, and takes into account what the rate limit leaves between the
rate at which callers ask to start calls and the rate the channel may start them. The same health
estimate caps the rate of starts, by its own measurements and under any configured limit, while the
server fails, and lets it climb back as the server accepts again. The backoffs belong to the same
account rather than beside it.

### 1.1 The baseline, and where it falls short

Built on the branch: every attempt takes a turn (a call's first, each retry, each transparent
resend; a stream takes one at its start), `Calls` turns per fixed window of `PerSeconds`. A first
attempt and a transparent resend wait for a turn in arrival order and end `DEADLINE_EXCEEDED` at
their deadline or `CANCELLED`. A retry whose backoff has elapsed takes a turn only if one is free at
once and no request is waiting; otherwise it is skipped into its next backoff, the skip counting
toward `MaxAttempts` and not in `grpc-previous-rpc-attempts` (the driver keeps `previous` and
`skipped`). The branch's decision names this as interim, until a global retry budget makes a refused
retry end the call. A6's `retryPolicy` is built; A6's throttle is not.

Shortcomings, each answered in section 4:

1. **A skip spends attempts that sent nothing**, and the next backoff is longer for a reason that is
   not the server's.
2. **Retries are first-come, not second.** A retry that finds a free turn takes it ahead of a first
   attempt that arrives a microsecond later, so a burst of retries can use a window and leave first
   attempts waiting. Bronson et al. name the remedy: a lower priority for retries (section 2.8).
3. **The retry rule reads a lock, not the capacity.** That no one is waiting is the state of a
   mutex; it says nothing about how much capacity first attempts left over.
4. **The fixed window has a boundary burst**: the last `Calls` of one window and the first `Calls`
   of the next can start together, `2 * Calls` at one instant.
5. **Nothing follows the server's health.** A channel retries a dead server as hard as a healthy
   one, and the rate limit is the same in both. A per-call cap does not bound a channel (Huang et
   al., section 2.8), and a channel budget does not bound a call; every stack surveyed keeps both
   (section 2.10).
6. **A budget that is a fixed ratio of the first-attempt rate is the wrong shape.** With ample room
   under the rate limit it still caps retries at a share of first attempts, and at one call a second
   a tenth of a retry is never a whole one (section 4.9).
7. **A herd after an outage is not paced.** Calls that wait for a connection leave together when it
   opens, and so do the transparent resends after a GOAWAY, unless the turn is taken when the
   request is about to leave; with no `Grpc.Rate.Limit` there is no rate to pace them by (4.8).

## 2. Survey

Each entry says what the mechanism bounds, its parameters, how it fails, and where it is described.
Numbers are the cited page's unless marked derived.

### 2.1 gRFC A6 retry throttling, and what the stacks build

Source: gRFC A6, <https://github.com/grpc/proposal/blob/master/A6-client-retries.md>.

- **What it bounds.** Retries of a channel (per server name) against the failure fraction. Optional;
  the service config's `retryThrottling: {maxTokens, tokenRatio}`.
- **Rule.** `token_count` starts at `maxTokens`, in `[0, maxTokens]`. Each failed RPC takes 1 from
  it; each successful RPC adds `tokenRatio`. Retries are throttled while `token_count <= maxTokens /
  2`. Only failures with a retryable (for hedging, non-fatal) status code count. A throttled retry
  is cancelled and the failure goes back to the application. Transparent retries do not count as
  failures for the throttle, nor toward `maxAttempts`. `tokenRatio` is truncated to three decimals.
  The grpc.io retry guide shows `maxTokens` 10 and `tokenRatio` 0.1 and says retries pause when the
  count falls below half; it says retries are enabled by default but there is no default policy
  (<https://grpc.io/docs/guides/retry/>).
- **Implementations.** grpc-java `RetriableStream.Throttle`: an `AtomicInteger` in thousandths of a
  token, compare-and-set
  (<https://github.com/grpc/grpc-java/blob/master/core/src/main/java/io/grpc/internal/RetriableStream.java>).
  grpc-go `retryThrottler`: a float under a mutex, `throttle()` decrements and returns `tokens <=
  thresh` (<https://github.com/grpc/grpc-go/blob/master/clientconn.go>). grpc core: atomic
  milli-tokens, compare-exchange, one instance per server configuration
  (<https://github.com/grpc/grpc/blob/master/src/core/client_channel/retry_throttle.cc>).
  grpc-dotnet `ChannelRetryThrottling`: a lock
  (<https://github.com/grpc/grpc-dotnet/blob/master/src/Grpc.Net.Client/Internal/Retry/ChannelRetryThrottling.cs>).
  The ArmoniK `GrpcClient` sets no `retryThrottling`
  (`packages/csharp/ArmoniK.Api.Client/Submitter/GrpcChannelFactory.cs`).
- **Behaviour (derived from the code).** A failure always decrements, whether or not a retry
  follows, and the retry is allowed only if the count after the decrement is still above half. The
  count falls while failures exceed `tokenRatio` times successes, that is when the failure share of
  attempts passes about `r / (1 + r)`; past that the count sits near zero and retries stop until
  about `(maxTokens / 2 + 1) / r` successes have refilled it (about 60 at 10 and 0.1). Below it,
  retries are unrestricted. So A6 is a deterministic, clamped, retries-only form of the SRE adaptive
  throttle (section 2.2), with `K = 1 + tokenRatio`.
- **Failure modes.** No notion of rate or of time: a client with little traffic recovers only as
  fast as it succeeds; it is all or nothing; it cannot see failures that are not retryable codes (a
  timeout, a slow server).
- **Other facts used below.** The deadline spans all attempts. A pick the client's own balancer
  drops fails the RPC at once with no retry, which is a precedent for failing on a client-side
  refusal. `maxAttempts` is required, and a value above 5 is treated as 5. Pushback
  (`grpc-retry-pushback-ms`) replaces the backoff, and a negative value means do not retry.

### 2.2 Google SRE: adaptive throttling and retry budgets

Sources: <https://sre.google/sre-book/handling-overload/>,
<https://sre.google/sre-book/addressing-cascading-failures/>.

- **Adaptive throttling.** Each client keeps, over the last two minutes, `requests`, the attempts
  made at the application layer, and `accepts`, those the backend accepted, and rejects a new
  request locally with probability `max(0, (requests - K * accepts) / (requests + 1))`, `K = 2`. The
  backend sees at most about `K` times what it accepts. The book's reason for 2 and not 1.1: letting
  more requests through than the backend will accept wastes some of its work, and in return the
  client notices sooner that the backend has recovered. It acts on first attempts too; its signal is
  a rejection by the backend, which the engine has to read from status codes.
- **Retry budgets.** Two, together: a request that has failed three times is returned to the caller
  (per request), and a per-client budget lets a request be retried only while retries are below 10%
  of requests. Without the ratio, retry chains could triple the traffic; with it, amplification
  stays near 1.1. The cascading-failures chapter adds randomized exponential backoff, a process-wide
  budget (60 retries a minute as an example) past which the request just fails, and retrying at one
  layer only: retries at three layers above the database, three each, make 64 attempts (4 cubed) on
  it.

### 2.3 Retry budgets in Finagle, Linkerd, tower and Envoy

- **Finagle `RetryBudget`.** `ttl` 10 s, `minRetriesPerSec` 10, `percentCanRetry` 0.2: about a fifth
  of requests may be retried, on top of 10 retries a second, the second term for clients that are
  new or have low traffic. A leaky bucket: every request deposits, a retry withdraws `1 /
  percentCanRetry` times as much, deposits expire after `ttl`, a reserve of `minRetriesPerSec * ttl`
  is always there. The budget is per client and the `RetryPolicy` per request; the documentation
  recommends one budget shared by the retry and requeue filters
  (<https://twitter.github.io/finagle/guide/Clients.html#retries>,
  <https://github.com/twitter/finagle/blob/develop/finagle-core/src/main/scala/com/twitter/finagle/service/RetryBudget.scala>).
- **Linkerd.** `retryRatio`, `minRetriesPerSecond`, `ttl` in the service profile, and a per-route
  retry limit that defaults to 1 (<https://linkerd.io/2/reference/service-profiles/>,
  <https://linkerd.io/2/reference/retries/>). Defaults of the budget were not on the pages read.
- **tower `TpsBudget`.** `new(ttl, min_per_sec, retry_percent)`, ten slots over `ttl` (1 s to 60 s),
  a reserve of `min_per_sec * ttl`, deposit on requests, withdraw before a retry
  (<https://docs.rs/tower/latest/tower/retry/budget/index.html>). Slots behind a mutex: not O(1)
  lock-free.
- **Envoy.** `RetryBudget { budget_percent = 20, min_retry_concurrency = 3 }`: concurrent retries as
  a share of active plus pending requests, with a floor, superseding the static `max_retries`
  circuit breaker (default 3) when set
  (<https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/cluster/v3/circuit_breaker.proto>). It
  bounds concurrency, so the allowance grows with latency.

### 2.4 AWS: retry quota and adaptive mode

Source: <https://docs.aws.amazon.com/sdkref/latest/guide/feature-retry-behavior.html>. The page says
its behaviour needs an opt-in until it becomes the default, and that earlier behaviour differs.

- **Standard mode.** A bucket of 500 tokens; a retry withdraws 14 for a transient error and 5 for a
  throttling error; a success returns what its retry took, or 1 if it needed none. Empty, the error
  is returned without a retry; the initial request is never delayed. Three attempts by default (four
  for DynamoDB), backoff `random(0, 1) * min(20 s, base * 2^retry)`.
- **Adaptive mode.** Adds a client-side rate limiter, a token bucket whose fill rate follows CUBIC
  on throttling responses, and can delay the initial request. In the Java SDK: a throttling error
  multiplies the rate by 0.7 and records it; success grows it as `0.4 * (t - w)^3 + last_max`, `w`
  the time to regain the recorded rate; the sending rate is held to twice the measured rate, with a
  floor of 0.5 tokens a second
  (<https://github.com/aws/aws-sdk-java-v2/blob/master/core/sdk-core/src/main/java/software/amazon/awssdk/core/internal/retry/RateLimitingTokenBucket.java>).
  The page does not recommend it as a default: one throttled resource slows every request of the
  client.
- **Builders' Library.** Retries are selfish, layers multiply (the page's figure: 243 times the load
  across five layers retrying three times), retry at one point, use a token bucket
  (<https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/>).
- **Brooker.** A token bucket acts like N retries when failures are rare and like a percentage of
  retries when they are common; a circuit breaker is N retries or none, and trips early because each
  client samples alone (<https://brooker.co.za/blog/2022/02/28/retries.html>).

### 2.5 Concurrency limits and Little's law

Netflix `concurrency-limits` (<https://github.com/Netflix/concurrency-limits>) limits requests in
flight, not per second, because fixed rates go stale as a service scales. That is Little's law,
concurrency equals rate times latency (Little, Operations Research 9(3), 1961): a rate limit `R` and
a concurrency limit `K` agree only while latency is `K / R`. When a server slows, a rate limit lets
concurrency grow and a concurrency limit lets the rate fall. Its AIMD limit (initial 20, minimum 20,
maximum 200) multiplies by 0.9 on a drop or a timeout and adds 1 when in flight is at least half the
limit
(<https://github.com/Netflix/concurrency-limits/blob/master/concurrency-limits-core/src/main/java/com/netflix/concurrency/limits/limit/AIMDLimit.java>);
Vegas and Gradient2 react to latency instead. Envoy's adaptive concurrency filter is the same family
and needs complete control of the cluster's concurrency
(<https://www.envoyproxy.io/docs/envoy/latest/configuration/http/http_filters/adaptive_concurrency_filter>).

### 2.6 Shapes of a rate limiter

- **Fixed window** (tower's `RateLimit`; the branch): one counter; a spike of up to twice the rate
  passes at a window boundary
  (<https://blog.cloudflare.com/counting-things-a-lot-of-different-things/>).
- **Sliding window log**: a timestamp per request; exact, with memory proportional to the count.
  **Sliding window counter**: two weighted counters; approximate (same source).
- **Token bucket** (depth `b`, refill `r`): the rate is `r` over any long interval, the burst at
  most `b` (Go, <https://pkg.go.dev/golang.org/x/time/rate>); `System.Threading.RateLimiting` offers
  all four shapes (<https://learn.microsoft.com/en-us/dotnet/core/extensions/http-ratelimiter>).
- **GCRA**: the same bucket kept as one timestamp, the theoretical arrival time `TAT`. A request at
  `t` conforms if `t > TAT - tau`; the next `TAT` is `max(TAT, t) + T`, `T` the emission interval.
  No periodic refill: the state is read against the clock on arrival
  (<https://en.wikipedia.org/wiki/Generic_cell_rate_algorithm>). Rust's `governor` crate keeps it in
  one atomic `u64`, lock-free, with a fake clock for tests
  (<https://docs.rs/governor/latest/governor/>).
- **A bucket does not make a window.** With depth `B` and rate `Calls / PerSeconds`, up to `B +
  Calls` requests can start in `PerSeconds`. With `B = Calls` that is the fixed window's `2 * Calls`
  worst case, but spread over the window; the burst that reaches a server at one instant falls from
  `2 * Calls` to `Calls`.

### 2.7 Backoff, jitter, wait-for-ready

AWS's jitter study finds full, equal and decorrelated jitter all cut the work substantially against
none, full jitter doing less work than decorrelated for slightly more time
(<https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/>). The engine's contract
draws uniformly below the bound, which is full jitter; that choice is made. gRPC's connection
backoff is 1 s, times 1.6, to 120 s, jitter 0.2, reset on SETTINGS
(<https://github.com/grpc/grpc/blob/master/doc/connection-backoff.md>). A wait-for-ready RPC queues
until the channel is ready and still fails at its deadline
(<https://github.com/grpc/grpc/blob/master/doc/wait-for-ready.md>).

### 2.8 Why it matters: retry storms and metastable failures

- Bronson, Aghayev, Charapko and Zhu, HotOS 2021
  (<https://sigops.org/s/conferences/hotos/2021/papers/hotos21-s11-bronson.pdf>): a trigger puts the
  system into overload and a sustaining effect keeps it there; request retries are among the most
  common. Their example: a database fast below 300 QPS, a web tier at 280 QPS retrying after 1 s; a
  10 s outage leaves 560 QPS and no goodput, ending only when load drops under 150 QPS or retries
  are held under 20 QPS. A lower priority for retried queries would break the loop, since the user
  queries that follow would succeed. A retry budget is named too; the difficulty is that each client
  decides alone.
- Huang, Magnusson et al., OSDI 2022, "Metastable Failures in the Wild"
  (<https://www.usenix.org/conference/osdi22/presentation/huang-lexiang>): the retry policy is the
  most common sustaining effect; its table lists it for 12 of the 22 incidents, and the text says
  more than half. At most two retries cannot amplify work more than three times; with no cap the
  system has no stable region. One incident (AWS3) shows the pressure against capping: servers were
  told to retry without limit after a botched recovery was blamed on their giving up.

### 2.9 Polly

Polly's rate limiter throws a rejection with a `RetryAfter` when known, and its guidance is to put
the retry outside it so the retry catches the rejection
(<https://www.pollydocs.org/strategies/rate-limiter.html>). That is the baseline: a refusal is a
failed attempt. The limiter outside the retry counts a call once however many attempts it makes,
which the engine's decision rejected because retries would multiply the rate the option names.

### 2.10 Two layers, per call and per channel, and which codes are retried

Every stack surveyed keeps both a bound on one request and a bound on the client, and none relies on
one alone:

| Stack | Per request | Per client or channel |
|---|---|---|
| SRE book | give up after about 3 failures | retries under 10% of requests; adaptive throttle |
| gRFC A6 | `maxAttempts`, required, clamped to 5 | `retryThrottling`, optional |
| AWS SDK | max attempts, 3 by default | quota of 500 tokens |
| Finagle | `RetryPolicy` (`tries`, `backoff`) | `RetryBudget` |
| Linkerd | retry limit, default 1 | retry budget |
| Envoy | per-route retry count | `max_retries` breaker or `RetryBudget` |

The per-request layer is what bounds a poison call, one that fails the same way every time. A
channel budget bounds the sum, so it can spend itself on one call, and does not bound the call.

Which codes are retried is the owner's choice and the stacks differ. A6 has no default (the list is
required) and the grpc.io example retries `UNAVAILABLE` alone. Envoy's gRPC retry conditions are
named (`unavailable`, `resource-exhausted`, `cancelled`, `deadline-exceeded`, `internal`). Finagle's
default requeue retries only failures known to be safe, before bytes reach the wire, never
application errors. `GrpcClient` and requirement 3 of this design retry `UNAVAILABLE`, `ABORTED` and
`UNKNOWN`. In gRPC `UNKNOWN` is what an unhandled server exception becomes, so retrying it retries
application bugs, which is how a poison call is made (Q5).

### 2.11 What the sources agree on

1. A per-call cap alone does not bound a channel, and a channel budget alone does not bound a call.
2. The channel bound is a ratio kept in tokens, with a depth that absorbs a cold start.
3. On exhaustion the call fails; none of them parks the retry.
4. First attempts are not throttled by the retry budget; adaptive throttling that does so is a
   separate, feedback-driven mechanism.
5. Retries should be the lower priority (Bronson et al.).
6. Backoff is jittered and is a per-call spacing, not a limiter.
7. A rate limit bounds the rate and a concurrency limit bounds latency-driven growth; Little's law
   relates them and neither replaces the other.

## 3. Comparison with the engine's constraints

Constraints: per channel; an admission check is O(1) and lock-free on its fast path, valid under any
number of threads; deterministic enough to test; explainable to a user in a few sentences;
compatible with A6's service-config parameters.

| Mechanism | Bounds | Hot path | Deterministic | Explainable | A6 parameters | Verdict |
|---|---|---|---|---|---|---|
| Fixed window (baseline) | starts per window; boundary burst | a counter and a lock | yes | yes | n/a | replace by GCRA |
| Sliding window log | exact starts per window | O(Calls) memory | yes | yes | n/a | no |
| Sliding window counter | starts, approximate | two counters | yes | fair | n/a | no |
| Token bucket / GCRA | rate `Calls / PerSeconds`, burst `Calls` | one word | yes (integers) | yes | n/a | **use**, the cells |
| SRE adaptive throttle, as a threshold | sends against accepts, over a window | a few counters, no random draw | yes | yes (send at most K times what is accepted) | maps (5) | **use**, the health estimate |
| A6 throttle | failure share, retries only, clamped integrator | one word | yes | fair | is the parameter | compared (4.9); mapped |
| Fixed-ratio ledger (Finagle, tower, Linkerd, AWS quota) | retries as a share of starts, plus a floor | slots, mutex (tower) | needs a window | yes | no equivalent | compared (4.9) |
| Envoy retry budget | concurrent retries | one counter | yes | yes | no | not now (2.3, 2.5) |
| AWS adaptive (CUBIC), Netflix AIMD, Vegas | rate or concurrency by feedback | controller state, samples | with a clock | hard | no | compared (4.9); later if needed |
| Per-call `maxAttempts` | one call | none | yes | yes | is the parameter | **keep, default 5** |

## 4. The proposal

### 4.1 The principle

**One estimate of the server's health, kept per channel and on by default, decides whether a retry
is worth sending and how fast calls may start; a retry is a loan against what first attempts leave
unused.** Three gates, each answering one question:

| Gate | Question | Bounds | Present when |
|---|---|---|---|
| per call: `MaxAttempts`, retryable codes, deadline, pushback | has this call tried enough? | one call's attempts | a retry policy |
| health: sends against accepts over a window | is the server accepting what is sent? | retries, and the rate of starts | `Grpc.Rate.Adaptive` not `Off` (the default) |
| ceiling: a GCRA cell | is there room under the configured rate? | starts per second, retries by what first attempts leave | `Grpc.Rate.Limit` |

The configured rate limit is an upper bound on top of the estimate: while the server accepts, the
estimate imposes nothing and the ceiling is the only limit; when it does not, the estimate's cap is
the lower of the two. With nothing configured but the per-call gate and a healthy server, behaviour
is today's.

### 4.2 The health estimate

This is the Google SRE book's client-side adaptive throttling (section 2.2), kept as counts over a
window and read as a threshold instead of a random draw.

**What it keeps.** Over the last `W` seconds, `R`, the attempts that ended and counted, and `A`,
those the server accepted. Each attempt counts when it ends; retries and transparent resends are
attempts, since each is a request the server sees. Attempts the engine refused locally never reached
the server and count for nothing, so `R` is what was sent.

**What counts.** The estimate is about the server accepting load, so it counts refusals of load, not
application failures. It classifies by where the end of the attempt came from, not by the gRPC code
alone, because the engine maps several origins onto one code:

| What ended the attempt | Counts as | Because |
|---|---|---|
| a status in the server's trailers: `OK`, or an application status (`NOT_FOUND`, `INVALID_ARGUMENT`, `ALREADY_EXISTS`, `FAILED_PRECONDITION`, `PERMISSION_DENIED`, `UNAUTHENTICATED`, `OUT_OF_RANGE`, `UNIMPLEMENTED`, `ABORTED`, `UNKNOWN`, `INTERNAL`, `DATA_LOSS`) | accept | the server took the request and answered; an application failure is not a refusal of load |
| trailers carrying `UNAVAILABLE` or `RESOURCE_EXHAUSTED` | reject | the server's own refusal or overload signal |
| trailers carrying `DEADLINE_EXCEEDED` or `CANCELLED` | neither | the server's own clock or the caller's; the estimate cannot tell them from a slow server |
| a failed attempt whose trailers carry `grpc-retry-pushback-ms` with a non-negative value | reject, whatever the code | the server asked the client to back off; a negative or unreadable value means do not retry, and the code decides |
| a response with no gRPC content type, from a proxy: HTTP 408, 429, 500, 502, 503, 504 | reject | the path could not serve it |
| the same, HTTP 400, 401, 403, 404 | accept | something answered, and not for want of capacity |
| the same, any other HTTP status, or HTTP 200 without a gRPC content type | neither | nothing says whether capacity was the cause |
| a dial, TLS or connection failure; a connection that dies or is reset by the peer before the response head with no GOAWAY, for any reason but `CANCEL` and `INADEQUATE_SECURITY`; `REFUSED_STREAM`; `ENHANCE_YOUR_CALM` | reject | the request could not be served |
| a reset `CANCEL` or `INADEQUATE_SECURITY`; a stream that breaks after the response head arrived | neither | not a refusal of load |
| a stream the peer's GOAWAY ended, processed or not, and a request that hyper dropped unsent | neither | a draining server is not an overloaded one |
| the engine's own: the call's deadline, a cancel, a message over the send or the receive limit, a request over the header-list limit, malformed trailers or messages, a closed channel, a failed dial task | neither | the caller's or the engine's doing, whatever code it carries |

A call that waits for a connection (wait-for-ready) has no attempt and counts nothing.

**What the engine must carry for this.** Today a failed attempt is a `GrpcStatus` and a code, and
several origins share a code. The engine maps a reset's reason to `CANCELLED` (`CANCEL`),
`RESOURCE_EXHAUSTED` (`ENHANCE_YOUR_CALM`), `PERMISSION_DENIED` (`INADEQUATE_SECURITY`),
`UNAVAILABLE` (`REFUSED_STREAM`, a peer's GOAWAY, and everything that is not a reset) and `INTERNAL`
(the framing errors and `NO_ERROR`); a proxy's HTTP status to `UNAVAILABLE` (429, 502, 503, 504),
`INTERNAL` (400), `UNAUTHENTICATED`, `PERMISSION_DENIED`, `UNIMPLEMENTED` and otherwise `UNKNOWN`
(500, 408, 3xx, 413, 507); a dial failure to an `UNAVAILABLE` it makes itself; and
`Unprocessed::Refused` stands for both `REFUSED_STREAM` and a stream a GOAWAY left unprocessed. The
pushback is read only on a failure before the response head. The classification needs the attempt's
origin carried with its status (trailers, HTTP status, reset reason, GOAWAY, dial, local), the two
meanings of `Refused` split, and the pushback read on every failed attempt.

**Two blind spots, stated.** A poison call, one that fails the same way every time with `UNKNOWN`,
counts as accepts, so it does not trip the estimate; the per-call ceiling is what bounds it (4.7).
And a server that slows down and times out shows nothing, since a timeout is neutral; a latency
signal would close that (section 10).

**What it computes.** With `K` the multiplier (default 2) and `S` the slack (default 10), the excess
`E = R - K * A`. The channel is **healthy** while `E <= S`. The SRE book's rejection probability is
`max(0, E) / (R + 1)`. The slack is the engine's addition: the book's `+ 1` does not stop one
rejection on an idle channel from reading as a failing server, and a channel with little traffic
must not lose its retries to a single failure.

**How it is kept.** A ring of twelve slots of `W / 12`, each one atomic 64-bit word holding the low
20 bits of its interval number and the two counts `R` and `A`, 22 bits each. An interval number is
`now / (W / 12)` as a 64-bit integer, and its slot is that number modulo 12. A read sums the slots
whose stored interval is one of the last twelve intervals' low 20 bits; the others are expired. The
window has the granularity of a slot: a count leaves between `11 W / 12` and `W` after it was
recorded.

Recording is a compare-exchange loop on the slot, never an unconditional add, so a count cannot
carry into the interval bits. It loads the slot and compares the interval it stores with the
record's by equality. If they are equal the record is added; if not the slot belongs to another
interval, and the compare-exchange replaces it with the record's interval and the first count. If a
count is at its limit, 4,194,303, both counts are halved before the add, which keeps the ratio. A
request and its accept are counted by one compare-exchange on one word, so a read never sees an
accept without its request, and a failed compare-exchange is retried with the word it returned. A
record whose clock reading is a whole window old, from a thread stalled for that long between
reading the clock and recording, replaces a newer slot and costs its counts: the only way a record
can regress a slot.

Two limits of the packing. The counts are exact while a slot holds fewer than 4,194,303 attempts,
which at the default `W` is about 1.7 million attempts a second and at the largest `W` of 3,600 s
about 14,000 a second; beyond that the ratio, and so the trip fraction, is kept and the scale is
not, so that `r_a = K * A / W'` is a lower bound by the lost factor. A channel does not carry 1.7
million attempts a second, which is why the count is not wider. The 20-bit interval repeats after
2^20 slots, 30 days at the default `W`: a decision after an idle gap that is exactly a multiple of
that, to within the window, finds the stale counts of up to twelve slots read as current, and they
stay for at most one window. That is one wrong window per such gap, and the packing cannot tell it.
Everything is integers and a `tokio::time::Instant`, so a paused clock drives it. The rest of the
design is in 4.12.

### 4.3 What the estimate does

1. **It gates retries.** When an attempt fails with a status the policy retries, the attempt is
   recorded and the retry is considered only if the channel is healthy; otherwise the call ends at
   once with that status. The decision is taken at the failure and again when the backoff has
   elapsed, so a call that will not be retried does not sleep first. A6's throttle decides at the
   failure too.
2. **It caps the rate of starts.** While healthy there is no cap. When not healthy the channel may
   start `r_a = max(K * A / W', r_floor)` attempts a second, with `W'` the window or the age of the
   estimate if younger and `r_floor` the `FloorPerSecond` option, 0.5 a second by default (the
   lesser of it and the ceiling cell's rate, `Calls / PerSeconds`): it sends at most `K` times what
   the server accepts, per second, and never stops probing. A first attempt over the cap **waits**
   for its turn of the adaptive cell, in order, as AWS's adaptive mode delays the initial request;
   the SRE book rejects it instead, and the engine does not (Q2, decided). The deadline ends the
   wait `DEADLINE_EXCEEDED` with nothing sent, a cancel ends it `CANCELLED`. A configured
   `Grpc.Rate.Limit` is a second, separate bound, taken after the cap is passed (4.4).
3. **The per-call ceiling stays.** `MaxAttempts`, default 5, bounds one call whatever the estimate
   says (4.7).

The cap is continuous with what the channel was doing, not with the ceiling: just past `E = S`, `K *
A / W'` equals the window's average send rate less `S / W'`, so the cap meets the load the channel
was already carrying, and falls from there as accepts age out of the window.

What each protects, which decides the numbers:

- The **ceiling cell protects a number the user declared**: the rate `Calls / PerSeconds` with a
  burst of at most `Calls`, whatever the server's state. Over one `PerSeconds` up to `2 * Calls` can
  start (the burst, then the refill), as in the fixed window's worst case, but never more than
  `Calls` at one instant. It knows nothing of health, and it is the only gate that sees the rate
  that callers ask for, so retries are bounded by the capacity left after first attempts.
- The **health estimate protects the server under failure**, and with it every other caller of it.
  While attempts are accepted, retries cost the server little and help the caller, so nothing caps
  them. When rejections dominate, each retry is work for a result unlikely to come, added to the
  load that is making the server fail. `K` says how many sends per accept the channel tolerates: the
  waste from this client is bounded by `K`, and with the default of 2 it takes more than half of the
  attempts being refused to trip. It is blind to the rate the user declared, which the ceiling cell
  protects.
- The **per-call ceiling protects against one call** that fails the same way every time.

### 4.4 The cells

Both caps are GCRA cells (section 2.6): a state `TAT`, nanoseconds since the channel's epoch, an
emission interval `T`, and a depth `B`. The level in tokens at `now` is `L = B - max(0, TAT - now) /
T`. A request conforms when `L >= 1`; taking the turn sets `TAT = max(TAT, now) + T`.

- **The ceiling cell** exists with `Grpc.Rate.Limit`: `T = PerSeconds / Calls`, `B = Calls`.
- **The adaptive cell** is consulted only while the channel is unhealthy: `T = 1 / r_a`, `B = max(1,
  ceil(r_a))`, one second of burst at the lower rate, both read from the current `r_a` at each
  admission. A decision that finds the channel healthy loads its `TAT` and, only if that is not
  zero, compare-exchanges it from the loaded value to zero, so no debt outlives the cap; if the
  compare-exchange fails a take came first and the next healthy decision tries again. A `TAT` in the
  past is a fresh cell, so none is created when the channel turns unhealthy. Its queue of waiters is
  not discarded with it.

A first attempt meets the adaptive cell first, while the channel is unhealthy or its queue is not
empty: it tries the cell and, if it does not conform or the queue is not empty, queues for it in
order on a fair queue of its own, so a newcomer never takes the fast path past a waiter of either
cell. Only when it has the adaptive turn does it take a turn of the ceiling cell, waiting in order
when it does not conform: it joins the ceiling's fair queue when that queue is not empty or the cell
does not conform, and a newcomer never takes the fast path past a waiter. Both queues are first in,
first out and the second is entered in the order the first leaves, a call joining the ceiling's
count before it leaves the adaptive's, so the calls keep the order in which they arrived whatever
mix of the two bounds delays them. A call waiting on the adaptive cell holds no ceiling turn, so the
calls delayed by a cap do not spend the ceiling while they wait. A retry conforms to the ceiling
cell when no first attempt is waiting and `L >= 1 + F`, with the reserve `F = floor((B - 1) / 2)`,
the shape of A6's `maxTokens / 2`; it never waits, and it does not consult the adaptive cell, since
an unhealthy channel has refused it already.

**The waiters of the adaptive cell.** A waiting call has sent nothing and holds no replay: it holds
its request. On the C ABI that request is charged to the memory ceiling like any send, so a host
that outruns the cap meets the ceiling's `BUDGET_BUSY`; a Rust host has no such bound in the engine,
and its queue is bounded by what it spawns. The cell serves its queue at `r_a`, down to the floor,
one call every `1 / r_a`, and each call it serves is a probe: an attempt that ends and counts. A
call with a deadline ends `DEADLINE_EXCEEDED` having sent nothing if its turn comes after it.

What a call with **no deadline** waits for depends on the server. If it is still dead, the call
waits for its turn, about its position in the queue times `1 / r_a`, two seconds a call at the
default floor, and then ends `UNAVAILABLE` as the probe it is. If the server has come back, a probe
succeeds, the estimate reopens within about a window (4.10), and the whole queue is released at
once, in order, into the ceiling cell if there is one and straight to the server if not: the pacing
by the ceiling is the one bound on that herd (4.8). Either way the call waits for the outage and the
reopening, and for a deep queue behind a dead server that is hours, where today the call ends in
about 8 s: with 60 calls a second offered against a floor of 0.5, the queue grows by about 59 a
second. A caller that wants a bound sets one, the call's own deadline or
`Grpc.DefaultDeadlineSeconds`; the engine has no option that fails a first attempt instead of making
it wait.

Guarantees of the ceiling cell, with `lambda_f` the served first-attempt rate (at most `Calls /
PerSeconds`, written `rate` below):

- **G1, retries leave the next `F` first attempts their turns.** A retry leaves `L >= F`, so a burst
  of up to `F` first attempts after it, with no retry between, starts at once, and no retry is
  admitted while a first attempt waits. With `F = 0` (`Calls` 1 or 2) a retry may take the last
  token, and a first attempt that follows waits up to `T`.
- **G2, the rate is bounded.** In any interval of length `t` at most `B + t / T` requests of all
  kinds start; the instantaneous burst is `Calls`, not `2 * Calls`.
- **G3, retries take only the capacity left.** Over an interval of length `D` at most `D / T + B`
  requests start, of which `lambda_f * D` are first attempts, so retries are at most `(rate -
  lambda_f) * D + B`. When demand is at `rate` or above the level stays near zero and no retry
  passes.

### 4.5 The rule

| kind | when | needs | on admission | when it cannot start now |
|---|---|---|---|---|
| first attempt | the call starts, the connection ready | healthy with no adaptive waiter, or a turn of the adaptive cell; then a turn of the ceiling cell | takes the turns | waits in order for the adaptive cap and then for the ceiling; the deadline ends the wait `DEADLINE_EXCEEDED`, a cancel `CANCELLED` |
| transparent resend | the peer never processed the request | as a first attempt | takes the turns | as a first attempt |
| retry | health at the failure and after the backoff; the ceiling cell after the backoff; the call under its ceiling | healthy, no first attempt waiting, and `L >= 1 + F` of the ceiling cell | takes a turn | refused: section 4.6 |

With no rate limit there is no ceiling cell; with `Adaptive` `Off` the health need is empty.

### 4.6 A refused retry: two rules

What happens to a retry that a gate refuses depends on whether a judgment of the server's health
exists, that is whether `Adaptive` is anything but `Off`, which it is by default.

- **Without the estimate, only a maximum exists** (the ceiling cell and the per-call ceiling). This
  is what the branch builds. A retry refused for capacity is not sent and its call goes into its
  next backoff, in case room frees up; the capacity decision is asked again after that backoff. The
  skipped attempt counts toward `MaxAttempts`, does not appear in `grpc-previous-rpc-attempts` (the
  server never saw it), and does not touch the estimate (it has no status of its own).
- **With the estimate enabled**, a retry has been judged worth sending, so a retry the ceiling cell
  refuses ends the call with the status of its last attempt, as a retry the health gate refuses does
  (and as A6 and every other stack surveyed do). Waiting for room is not needed to find out whether
  retrying is useful, and a call that cannot be served at once frees its replay memory and its
  caller. This is the rule in force by default, and it applies to a channel that sets only
  `Grpc.Rate.Limit`: a call that fails while first attempts are using the rate ends at its first
  failure. Raise `Calls` if retries matter more.

A limit slower than the backoff leaves no room for retries: with `Calls` 1 and `PerSeconds` 10 a
retry finds the cell empty, since its own failed attempt took the turn less than a backoff ago.
Under the first rule such a call skips until `MaxAttempts` is spent; under the second it ends at its
first failure. A limit meant to leave room for retries has an emission interval well under the retry
backoff.

### 4.7 Per-call bounds, and the poison call

A call is bounded by the conjunction of: a retryable code; not committed (contract.md's commitment
point, unchanged); the deadline (a backoff that would pass it ends the call with its last status);
the server's pushback; **`MaxAttempts`, default 5, whatever the channel's gates say**; and the gates
of 4.1. The deadline and the channel's gates are not substitutes for the ceiling: a call that fails
the same way every time, with a long deadline and a channel that is otherwise healthy, would be
retried at a mean interval of half of `MaxBackoff` (each draw is uniform below it) for the whole
deadline, over a thousand requests in an hour. The ceiling ends it after four retries, in at most
about 8 s at the default backoff (the bounds are 1, 1.5, 2.25 and 3.4 s, and each draw is below its
bound). The ceiling is the option's value as built, any value of at least 1; a service config's
`maxAttempts` reaches it through the mapping of section 5, which clamps it to 5 as A6 requires of a
reader. Sections 2.10 and 2.8 are the evidence: every stack keeps the layer, and Huang et al. find
the uncapped policy the one with no stable region. The estimate sees the poison call as accepts, so
the ceiling and the retryable codes are its only bound: the case for the `GoogleRpc` preset of
retryable codes (Q5).

### 4.8 Backoff, transparent resends, wait-for-ready, no rate limit

- **Backoff.** The failure backoff is as built (the policy's bounds and multiplier, drawn uniformly
  below the bound, the server's pushback replacing it). It is the one clock of a call's retries; the
  capacity decision is asked once per backoff, after it. With the estimate enabled it grows only
  with failures the server returned. The limiter is not asked when capacity returns: setting the
  wait to the instant `TAT - (B - 1 - F) * T` would wake every refused retry together, which the
  jitter exists to prevent.
- **Transparent resends** take the first-attempt class: each is a request the server sees, one is
  allowed per way of not being seen. After a GOAWAY, N calls resend at once and the ceiling cell
  paces them. A resend after `REFUSED_STREAM` records a reject; one a GOAWAY left unsent records
  nothing.
- **Wait-for-ready.** A call that waits for a connection waits on the channel's connection backoff
  as built. The order inside an attempt is: a connection is ready, the adaptive cell is tried, the
  ceiling turn is taken, the deadline that remains is read, the request is sent. The turns come
  after the wait, and the ceiling turn after the adaptive one, so that calls do not spend turns
  while they wait and leave together when the connection opens or the cap lifts; reading the
  deadline last keeps `grpc-timeout` stating what remains, as the branch does by taking the turn at
  the top of the loop. A retry's capacity decision is taken at that same point, and is a try, never
  a wait. If the connection fails between the turn and the send, the request was unsent: a
  transparent resend, with a turn of its own, once per call.
- **No rate limit.** There is no ceiling cell; the estimate stands alone. Retries are gated by it
  and, when it is unhealthy, first attempts are capped by it, with nothing above. The herd after an
  outage is then paced only while the channel is unhealthy; once it turns healthy again there is no
  rate to pace the queued calls by.

### 4.9 Which budget, and which control law

**Which budget.** Three shapes were compared.

(a) *A fixed-ratio ledger*: retries at most `max(floor, ratio * rate)` (Envoy's
`min_retry_concurrency` is a floor; Finagle's and Linkerd's `minRetriesPerSec` and tower's
`min_per_sec` are added to the share; the AWS quota deposits per success). It bounds the load
retries add in absolute terms, and needs a window to estimate the rate. Without a floor a channel
with little traffic is starved; with one, it still caps retries when the server is healthy but
flaky, which costs calls for no benefit to the server, and against a dead server it keeps sending at
the floor and the share.

(b) *A6's bucket*: failures take a token, successes add `tokenRatio`, retries stop at half. It is
the SRE statistic with `K = 1 + tokenRatio`, clamped instead of windowed: its memory is the clamp,
so it recovers only as fast as the channel succeeds (about 60 successes at 10 and 0.1, a minute at
one call a second), and it counts the retryable codes, `UNKNOWN` among them, as failures.

(c) *The SRE statistic over a window*: adopted. Retries flow while the server accepts and stop when
it does not, with no volume term, so low traffic is not starved; memory is a time window rather than
a count of successes; what counts is the server's refusal of load, so application failures do not
trip it; and `K` has a meaning a user can check: the channel sends at most `K` times what is
accepted.

**Which control law moves the rate.** Three laws can move the effective rate under a ceiling.

| | SRE statistic, as a cap (adopted) | AIMD (Netflix, TCP) | CUBIC (AWS adaptive mode) |
|---|---|---|---|
| State beyond the counts | none: `r_a` is a function of `R`, `A`, `W` | the limit itself | `r_max`, the time of the last throttle, a smoothed measured rate |
| Signal | accepts and rejects | a drop (a rejection or a timeout) | a throttling error |
| On a failure | `r_a` falls as `A` ages out against `R` | multiply by 0.9 (Netflix default) | multiply by 0.7, remember the rate |
| Recovery | the cap lifts when accepts catch up with `R / K` (4.10) | add 1 per sample while in flight is at least half the limit: linear | concave to the last good rate in about `cbrt(r_max * 0.3 / 0.4)` s, then convex probing |
| Stability | no sawtooth; at saturation sends `K` times the capacity, by design | sawtooth around capacity, converging | sawtooth; designed for TCP and for AWS throttling responses |
| Parameters | `K`, `W`, `S`, floor | initial, min, max, ratio, timeout | `BETA` 0.7, scale 0.4, smoothing 0.8, floor 0.5 per second |
| Tests under paused time | counts and arithmetic | event sequences | time and cubic roots, floats |
| Fit here | explicit refusals (`UNAVAILABLE`, `RESOURCE_EXHAUSTED`); no extra controller state; one number a user can check | needs an increase step that suits a ceiling of 1 or 1000 per second | fast recovery, but the page does not recommend it as a default |

The statistic's weakness is the one the table shows: its recovery is a step. It holds the cap near
the floor until the counts say the server accepts, then lifts it at once. If that proves too abrupt
with a real control plane, the refinement is a ramp of `r_a` from the floor, geometric or cubic,
with the cell's `T` as the only thing that moves (section 10).

### 4.10 Stability and recovery, derived

These follow from the definitions at a steady offered rate `lambda` of first attempts, `K` 2, `n`
attempts per failing call while retries pass (at most `MaxAttempts`, 5), and a window `W` that was
healthy before the outage. They are derived, not measured.

- **Fixed point.** While the server accepts at most `a` per second, the channel sends at most `K *
  a`: with a saturated server, `K` times its capacity, of which the server refuses `(K - 1) / K`.
  That waste is the price the SRE book accepts, because a rate that stays above what is accepted is
  how recovery is noticed.
- **When retries stop.** In steady state, after an outage of length `d`, `A = lambda * (W - d)` and
  `R = A + n * lambda * d`, so `E = lambda * ((n + 1) * d - W)`: retries stop after `d = W / (n +
  1)`. At the default `W` of 30 s and `n` 5 that is 5 s, and about 15 s (`W / 2`) if the calls were
  not retried (`n` 1). An outage shorter than that changes nothing. Attempts end only after their
  backoffs, whose means are about 0.5, 0.75, 1.1 and 1.7 s, so counting the attempts that have
  ended, the trip is near 6.5 s and about a thousand retries pass before it, which is the cost of a
  window that remembers a healthy past.
- **When first attempts are capped.** After the retries have stopped, `A` keeps ageing out, and the
  cap `K * A / W' = 2 * lambda * (W - d) / W` falls under the offered rate `lambda` at `d = W / 2`,
  15 s, and reaches the floor at `d = W`, 30 s. Until then first attempts reach the dead server
  uncapped; after, the surplus waits for its turns at the falling cap (4.4).
- **Recovery.** The estimate reopens when the rejections recorded during the outage have aged out of
  the window down to the slack. That is at most about one window after the server returns, 30 s, and
  the longer the retries amplified the outage the nearer that bound. After an outage of `2 W` or
  more the window holds only probes at the floor (at `W` it still holds the uncapped attempts of the
  first half): `R` is about `r_floor * W`, 15, all rejected; each probe after the return adds one
  accept while the oldest rejections age out, so `E = r_floor * W - K * r_floor * t` falls from 15
  to `S` = 10 in 5 s. The cap and the retries come back together, in one step.
- **If the server is still failing at the step**, `E` passes `S` again after about `S` further
  rejections and the cap returns, so each cycle wastes about `S` requests plus the adaptive cell's
  burst of one second at the lower rate.

### 4.11 Worked examples

`W` 30 s, `K` 2, `S` 10, floor 0.5 a second; callers asking for 60 calls a second; five attempts a
call by the policy's default; no `RateLimit` unless stated.

| situation | baseline: the skip rule, fixed window | proposal |
|---|---|---|
| a 3 s outage | each call retried up to 4 times over about 8 s | `E` stays negative: healthy; nothing changes |
| 5% of attempts refused, steady | the retries pass | healthy; the retries pass |
| 50% refused, steady | the retries pass | `E` is 0, so healthy: the server sees at most twice what it accepts |
| 60% refused, steady | the retries pass | `E` is 0.2 `R`, far past `S`: retries refused at their failure; the cap `2A / W'` is 80% of the counted rate and falls with it, so the surplus first attempts wait for a turn; if the refusals do not depend on the channel's load the cap converges to the floor, and if they do it settles where the server accepts what is sent |
| the server dead for 60 s | each call takes about 8 s to end `UNAVAILABLE`, 240 retries a second | retries stop after about 5 to 7 s (about a thousand pass); first attempts go uncapped to about 15 s, then the cap falls to the floor at 30 s and the surplus queues: each call ends `DEADLINE_EXCEEDED` at its deadline having sent nothing, or is served as a probe at the floor and ends `UNAVAILABLE` |
| the server dead, calls with no deadline | each call ends `UNAVAILABLE` in about 8 s | after the retries stop, each call waits about two seconds for each call ahead of it, then ends `UNAVAILABLE` as a probe |
| the server back after that outage | the load resumes | retries and first attempts are back about 5 s after it returns, and within 30 s after any outage |
| 1 call a second, 30% refused | all retries pass | healthy: retries pass; no volume term starves it |
| `RateLimit` 100 over 1 s, healthy, 5% failing | the 3 retries a second pass | pass; 37 of the 100 turns a second stay unused |
| `RateLimit` 100 over 1 s, demand 100 a second | retries refused when no turn is left or someone waits | no retry once the level is under `1 + F`, 50; first attempts proceed at the rate |
| one call that always fails `UNKNOWN`, deadline 1 hour, with the `GrpcClient` preset (`GoogleRpc` does not retry it: one attempt) | 5 attempts | 5 attempts in about 8 s at most; without the ceiling about 1,400, one every 2.5 s on average; five accepts per call lower `E` |

### 4.12 The state: lock-free, valid under any number of threads

One design serves both hosts, with no choice of mode. The C ABI gives each channel its own thread
running a current-thread tokio runtime (`AkRuntime::start_channel_thread`, `new_current_thread`),
`GrpcChannel::new` takes that runtime's handle, and every driver is spawned on it (a
`spawner.spawn(...)` around `driver.drive(...)` in `armonik-transport-ffi/src/call/actor.rs`): the
decisions of a channel run on one thread, so the words below are never contended there. The
`armonik` crate gives `GrpcChannel::new` the handle of the host's runtime (`Handle::current()`),
multi-thread unless the host chose otherwise, and starts calls with `start_call`, which spawns each
driver on it, so the drivers of one channel run on whichever worker is free and the same words are
contended. `prepare_call` hands the driver to its caller, which may run it anywhere. The state is
therefore written to be correct under any number of threads, and it is cheap when only one uses it.

**The words.** Every decision reads and writes a few atomic 64-bit words. Those that hold counts and
times are accessed with relaxed ordering: none publishes other data, so none needs an acquire or a
release, and a decision tolerates a view a few records stale. The two waiter counts alone are
sequentially consistent, for the reason given under the lock below.

| Word | Holds | Written by |
|---|---|---|
| ring slot, twelve of them | interval (20 bits), `R` (22), `A` (22) | compare-exchange: to record, to start a new interval, to halve |
| ceiling `TAT` | the theoretical arrival time of the GCRA cell, nanoseconds from the channel's epoch | compare-exchange loop to take a turn |
| adaptive `TAT` | the same, for the adaptive cell | compare-exchange loop; compare-exchanged to zero by a decision that finds the channel healthy, only if it is not zero |
| two waiter counts | first attempts queued for the adaptive cell and for the ceiling | `fetch_add` on joining, `fetch_sub` by a drop guard on leaving, by a turn, a deadline or a cancel |

Where a decision must read several fields consistently they share a word: a cell is one `TAT`, from
which the level is computed with `T` and `B`, which are not stored, since `T` is `PerSeconds /
Calls` or `1 / r_a` and `r_a` is computed from a read of the ring; a slot's interval and counts are
one word. What a decision reads across words, the twelve slots, is a snapshot that may mix moments a
few records apart, which `S` absorbs. The words sit on separate cache lines (the ring on two) so
that a record does not invalidate a cell.

**What a decision costs, uncontended.**

| Decision | Atomic read-modify-writes | Loads |
|---|---|---|
| a first attempt with the estimate healthy and no rate limit | 0 | 12 slots, to read `healthy`; the adaptive `TAT` and waiter count |
| a first attempt, rate limit set | 1, the ceiling's compare-exchange | 12 slots, both `TAT`, both waiter counts |
| a first attempt over an adaptive cap | 2, one on each cell | as above |
| recording an attempt's end | 1, the slot's compare-exchange, once more per `W / 12` to start a slot | the slot |
| a retry's decision | 1 with a rate limit, the ceiling's compare-exchange; 0 without | 12 slots at the failure, the ceiling `TAT`, both waiter counts |
| the stats of T10.2 | 0 | one per word |

So a call that succeeds at its first attempt costs two read-modify-writes with a rate limit and one
without. A failed compare-exchange is retried with the value it returned; the loops are lock-free,
since a failure means another thread succeeded. The loads are 96 bytes of ring and a few words.

**Contention.** On the C ABI there is one thread per channel, so no read-modify-write is contended
and the words stay in that core's cache. On a multi-thread host, threads deciding at the same
instant contend on the ring's current slot, on `TAT` and on the waiter counts; the contention is per
channel and bounded by that channel's call rate, and a compare-exchange loses only to a concurrent
success. Two costs are specific to that host: every decision reads the ring that every record
writes, so a healthy channel with no limit still moves a cache line between cores; and a hot cell is
a single word. The remedies, if the benchmark of the task that builds this (the unary path on the C
ABI, and the `armonik` client with eight worker threads) shows they matter, are a cached verdict
word that a recorder refreshes when it changes a slot, and a ring sharded by thread, whose counts
are additive. Neither is needed to be correct.

**Where a lock remains.** One place: the FIFO queue of first attempts that must wait, one for the
adaptive cell and one for the ceiling, each an async fair mutex held by the head waiter while it
sleeps until its turn is due. A first attempt that finds its cell conforming and its waiter count
zero never touches it: the fast path is the compare-exchange above. The waiter counts are the
arrival order. A first attempt, and a retry, reads both with sequentially consistent ordering before
it takes a turn, the adaptive count first and the ceiling count second; a waiter increments its
count, also sequentially consistent, before it parks, and a call leaves the adaptive count only
after it has joined the ceiling's or taken its turn. In their one total order, a newcomer that reads
a count after a waiter's increment sees it and queues behind the waiter, and one that reads it
before has arrived first and may take its turn; a call in transit from one queue to the other is in
at least one of the counts when either is read, in that order. The order is that of these accesses,
not of the compare-exchange that follows, so a retry never passes a waiter that registered before it
read. A cancelled or deadline-ended waiter decrements the count in a drop guard, so a count left
above zero cannot block the fast path or the retries. The queue cannot be lock-free for the property
it exists for: first in, first out among tasks that park and resume, and a task that is dropped, by
a deadline or a cancel, must leave the queue and take no turn, which means removing a waiter from
the middle of a list. The async fair mutex does it with an intrusive list under a short internal
lock, and it is an async wait: the task parks, no thread blocks. Its contention is among waiting
calls only, which are already waiting for a rate.

**Stats and tests.** The stats of T10.2 are plain loads of the same words, with no lock and no
effect on a decision. Tests 6 to 9 of section 9 cover the lock-free parts under many threads and
under both runtimes.

## 5. Honouring a service config's `retryPolicy`

The engine reads no service config (requirements.md excludes it, and the engine's vocabulary is the
options of section 6). A host that has one, as `GrpcClient`'s `ServiceConfig` is today, maps it onto
those options in the host binding, which is T6.14's loader, and refuses a field it cannot map by
naming that service-config field in its error.

| `retryPolicy` field | Becomes | Note |
|---|---|---|
| `retryableStatusCodes` | `Grpc.Retry.Codes` as `List` of those codes | as written; A6 requires the list, so the mapping always states one |
| `initialBackoff`, `maxBackoff`, `backoffMultiplier` | the failure backoff | as written |
| `maxAttempts` | the per-call ceiling | clamped to 5 by the mapping, as A6 requires of a reader; A6 requires at least 2, the engine's own option allows 1 (no retry) |
| `retryThrottling.maxTokens`, `tokenRatio` | `Grpc.Rate.Adaptive` as `On` with `Multiplier` `1 + tokenRatio` and `Slack` `ceil(maxTokens / 2)` | the same failure share trips it (A6 when the failure share passes `r / (1 + r)`, the estimate when it passes `(K - 1) / K`); the rest differs, below |
| `hedgingPolicy` | refused by the loader | hedging is not wanted; ignoring it would change what a call costs the server |

**`retryThrottling` is mapped, not run as written, and the two are not equivalent.** A6's bucket
counts the retryable codes as failures, `UNKNOWN` and `ABORTED` among them, recovers by successes,
and throttles retries only; the estimate counts refusals of load, recovers by the window ageing, and
also caps first attempts. The mapping keeps the failure share and the tolerance at the start.
Running A6's bucket beside the estimate would give the channel two health judgments with two
meanings. The service owner's fields describe one call and the throttle; the rate limit is the
channel's, a declaration of the user's that a service config cannot express, and composes with them
by conjunction: a retry must pass every gate.

## 6. Option vocabulary

Names under `Grpc.*` follow `options.rs`: PascalCase, `Seconds` for durations, every field optional
with a default, a struct merging field by field, and alternatives that exclude one another as an
enum, which the schema renders as a `oneOf` of objects of one key (`true` for an alternative that
carries nothing), as `Http2.Receive` does with `Fixed` and `Adaptive`, and the proxy with `None`,
`System` and `Url`. A new group, `Grpc.Rate`, holds what bears on how fast calls start:

    Grpc.Rate.Limit    { Calls, PerSeconds }            the ceiling: today's Grpc.RateLimit (Q12)
    Grpc.Rate.Adaptive  { "On": { ... } } | { "Off": true }   the health estimate

`Adaptive` is an option with two alternatives, as `Http2.Receive` has `Fixed` and `Adaptive`: `On`
carries the parameters and `Off` carries nothing, so no reader checks pairs of keys. **When
`Adaptive` is absent from the merged document it is `On` with every default**, so a channel and a
`ChannelDefaults` that state nothing give the estimate. It merges as any alternative does
(decisions.md, "Where a channel's defaults are set"): a channel and the defaults that state `On`
merge their parameters field by field, so a channel that states `{"On": {"Multiplier": 3}}` keeps
the defaults' `Slack`; one that states `Off` over the defaults' `On`, or the reverse, takes its
alternative whole.

| Option | Type | Default | Refused when |
|---|---|---|---|
| `Grpc.Rate.Limit.Calls` | int | none | below 1, or without `PerSeconds` (as built) |
| `Grpc.Rate.Limit.PerSeconds` | seconds | none | not above 0, or without `Calls` (as built) |
| `Grpc.Rate.Adaptive` | `On` or `Off` | `On` | a document naming both |
| `Grpc.Rate.Adaptive.On.Multiplier` (`K`) | number | 2 | not finite, below 1 or above 100 |
| `Grpc.Rate.Adaptive.On.Slack` (`S`) | int | 10 | below 0 or above 1,000,000 |
| `Grpc.Rate.Adaptive.On.WindowSeconds` (`W`) | seconds | 30 | below 0.012 (a slot of under a millisecond) or above 3,600 |
| `Grpc.Rate.Adaptive.On.FloorPerSecond` | number | 0.5 | not finite, not above 0 (a floor of 0 stops the probing) or above 1,000,000 |
| `Grpc.Retry.MaxAttempts` | int | 5 | below 1 (as built) |
| `Grpc.Retry.Codes` | `GoogleRpc`, `GrpcClient` or `List` | `GoogleRpc`, recommended (Q5, open) | `List` empty, naming `OK`, or naming a name that is not a status code |

**`Grpc.Retry.Codes`** selects which statuses a call is tried again for, as an enum, so that a
preset and an explicit list exclude one another and nothing needs combining. `{"GoogleRpc": true}`
is what `google.rpc.Code` advises for retrying the same call: `UNAVAILABLE` alone. `{"GrpcClient":
true}` is the .NET `GrpcClient` default, `UNAVAILABLE`, `ABORTED` and `UNKNOWN`. `{"List":
["UNAVAILABLE", "ABORTED"]}` is an explicit list, spelled as A6's `retryableStatusCodes` spells it.
It merges as any alternative does: the same preset over itself is the same, a different alternative
is taken whole, and a `List` over a `List` replaces the array, since a list is one value. Absent
from the merged document it is the default alternative. The `GrpcClient` translation of T6.8 states
`GrpcClient` explicitly, as the mapping of a service config states `List`.

For `Adaptive`, a document that names both `On` and `Off` is refused as it is read. A value out of
bounds is refused when the channel is created, naming its key, and the defaults' when the runtime is
created, naming `ChannelDefaults`, as decisions.md has it. **To turn the estimate off**, set
`Adaptive` to `{"Off": true}`: the channel then has the per-call gate and the ceiling alone, with
the first rule of 4.6. The reserve `F` is a constant, not an option. `Calls` and `PerSeconds` give
the ceiling `Calls / PerSeconds` and the burst `Calls`; a smaller burst at the same rate is a
smaller `Calls` over a shorter `PerSeconds` (10 over 0.1 s is 100 a second with a burst of 10). The
rest of `Grpc.Retry` stays. The control is `Adaptive` and not `Throttling` so as not to be taken for
A6's `retryThrottling`, which it replaces and maps (section 5). It sits under `Rate`, though it also
gates retries, because it is a rate: what the channel may start, by what the server accepts.

**Why this is safe for a healthy server.** With the defaults nothing is imposed while `E <= S`, that
is while the server has refused fewer than `S` more attempts than `K - 1` times its accepts over the
last 30 s. A server accepting everything has `E` at `-R`; one refusing 30% of attempts has `E` at
`-0.4 R`; the estimate acts only past 50% net of the slack. A restart of a few seconds is ridden out
by retries as today (4.10).

## 7. What changes in decisions already taken

Proposals, each against the document it touches.

1. **`Grpc.RateLimit` becomes GCRA** (`Grpc.Rate.Limit`, as Q12 decides; decisions.md, "What a rate
   limit counts, and what a request over it does"). Kept: what counts (every attempt, a stream
   once), arrival order, the end of a waiting call by deadline or cancel, the limit being the
   channel's. Changed: no window; `Calls` start at once after idle, then one every `PerSeconds /
   Calls`. This removes the burst of `2 * Calls` at one instant, not the `2 * Calls` that can start
   over one `PerSeconds`. The cost the row cites for a sliding window, a record per start, is not
   paid. The limit is now the ceiling of an effective rate that the estimate may lower. The merged
   code is amended by a follow-up commit on `wk/feat/phase1`, while the option is unreleased: GCRA
   and the reserve, and the rename of Q12; its skip rule and counters stay as the first rule of 4.6.
2. **"A saturated limit skips retries (interim, until the retry budget)"** keeps its first rule for
   a channel that disables the estimate and gives way to the second, ending the call, by default:
   section 4.6. The branch's `try_admit`, which reads a mutex, goes.
3. **T6.15's retry throttling is built as the health estimate**, mapped from A6's parameters
   (section 5). contract.md's "not specified yet" sentence on the throttle is replaced by 4.2 and
   4.3.
4. **Requirement 3.1** ("calls that fail with `UNAVAILABLE`, `ABORTED` or `UNKNOWN` are retried")
   gains two: not while the channel is unhealthy, and the default codes are those of the
   `Grpc.Retry.Codes` default (Q5), with `GrpcClient`'s and the `armonik` crate's translations
   stating the `GrpcClient` preset.
5. **`RetryConfig` keeps `max_attempts` and its default of 5**; the estimate is a new config beside
   it. Its retryable codes follow Q5: with a `GoogleRpc` default, `RetryConfig::default` and
   `RetryOptions::default().to_config()`, which options.rs tests equal, both carry `UNAVAILABLE`
   alone.
6. **The formal model** needs no change if, as now, neither `RetryConfig` nor `options` is modelled:
   a retry not sent is a call that ends with a status it could already end with, and a first attempt
   that waits for a turn is a call that has not started. To be checked when built.
7. **The engine's attempt outcome carries its origin** (4.2), which the status mapping does not keep
   today.
8. **T10.2's `GrpcChannel::stats()`** (observability.md, not built) gains counters: starts by kind,
   first attempts that waited, first attempts that waited for the cap, retries refused by each gate,
   `R` and `A` over the window, `E`, the cap and the ceiling cell's level.
9. **SPEC.MD** lists this document.
10. **decisions.md's grouping row** ("How a channel's options are grouped") gains `Rate` among the
    groups of `Grpc`, holding the limit and the adaptive control.
11. **`Grpc.Retry.Codes` lands in the places an option lands:** `options.rs` (the enum, its
    documentation and refusals), both schemas, the generated C#, the loader's mapping of
    `GrpcClient` (T6.8 states `GrpcClient`) and of a service config (`List`), and the tests and the
    vocabulary test.

## 8. Decisions and open questions

Answered on 2026-10-07 unless marked open.

**Q1. The defaults: `K` 2, `S` 10, floor 0.5 a second, `W` 30 s. Decided, to be tested.** `K` 2 is
the SRE book's, and the floor is AWS adaptive mode's minimum fill rate. The book's window is two
minutes; the proposal shortens it to 30 s because retries stop after `W / (n + 1)` of outage (4.10),
5 s at 30 s and 20 s at two minutes. `S` is a judgment, about A6's `maxTokens / 2` at the grpc.io
example doubled. The plan that shows them right or wrong is in section 9.

**Q2. A first attempt over the adaptive cap waits. Decided, as AWS adaptive mode delays.** The wait
is bounded by the call's deadline; a call with none waits for its place in the queue and is then
served as a probe or, if the estimate reopens first, released in order (4.4). There is no option
that fails the call instead: that would be a second behaviour to document and test, and a caller who
wants a bound has the deadline. This is the rate limit's ordering too: the waiters of the adaptive
cell and of the ceiling form one first-in, first-out order (4.4).

**Q3. The estimate is on by default, and a capacity refusal is final. Decided.** Requirement 3.1
gains an exception (7.4); a channel that sets only `Grpc.Rate.Limit` has a retry refused by the
ceiling end its call, instead of skipping into the next backoff (4.6). The `GrpcClient` this engine
replaces has no throttle and no such rule.

**Q4. The accept, reject and neutral classification, and the plumbing it needs. Open.** As proposed
in 4.2: refusals of load reject, application statuses (`ABORTED` and `UNKNOWN` included) accept,
deadlines, cancels, GOAWAY-ended streams and the engine's own statuses are neutral. The blind spot
is a slow server that answers late. The plumbing (7.7) is a prerequisite of the estimate.

**Q5. An option selects the retryable codes. Decided; its default is open.** What the standards say:
A6 sets no default list and requires one (`retryableStatusCodes` is non-empty); the grpc.io example
retries `UNAVAILABLE` alone. `google.rpc.Code` advises `UNAVAILABLE` where the client can retry just
the failing call, `ABORTED` where the client should retry at a higher level (a failed sequencer
check or a transaction abort, which means starting the unit of work again, not repeating the call),
and describes `UNKNOWN` as a status from an error space this address space does not know.
`GrpcClient` and requirement 3 retry all three. The option is `Grpc.Retry.Codes`, an enum of two
presets, `GoogleRpc` and `GrpcClient`, and an explicit `List` (section 6); the `GrpcClient`
translation of T6.8 picks `GrpcClient`. Open: the engine's default. Recommendation: `GoogleRpc`.
Compatibility of ArmoniK.Api.Client is carried by the translation, which states `GrpcClient`, so it
does not need the engine's default to match; the default then serves the hosts that state nothing,
for which the standard is the right behaviour and a poison call, the failure the per-call ceiling
alone bounds and the estimate does not see, is not made by default. Against: requirement 3.1 and
`RetryConfig::default` list three codes, so the default changes the contract text and the equality
options.rs tests between `RetryOptions::default().to_config()` and `RetryConfig::default()`, and a
Rust host that reads no `GrpcClient` configuration changes behaviour; the validation plan's last
scenario counts the codes that end calls to show whether `ABORTED` and `UNKNOWN` matter in practice.

**Q6. The reserve is a constant, half the bucket. Decided.**

**Q7. Retries have no floor at saturation. Decided.** When first attempts ask for the rate or more,
retries get nothing.

**Q8. The turn of a wait-for-ready call is taken after the connection is ready. Decided** (4.8). A
connection that fails in between makes the call a transparent resend with a second turn.

**Q9. A transparent resend waits as a first attempt. Decided.**

**Q10. No ramp after reopening for now. Decided.** To be reopened if a real control plane shows the
step is too abrupt (4.9).

**Q11. One lock-free design for both hosts. Decided.** Atomic words with lock-free
read-modify-writes, valid under any number of threads, for the C ABI and the `armonik` crate alike;
a lock remains only in the FIFO queue of waiting first attempts (4.12). There is no choice of model
by runtime flavor. The C ABI's single channel thread makes contention nil there.

**Q12. `Grpc.RateLimit` becomes `Grpc.Rate.Limit`. Decided.** The rate limiter is already merged on
`wk/feat/phase1`, so the rename is a follow-up commit there while the option is unreleased. It
touches the five places a rename touches: the options type and its documentation, the two schemas,
the generated C#, the loader's mapping of `GrpcClient__RateLimit` (T6.14), and the tests and the
vocabulary test. After a release it would be a break.

## 9. Test plan

Observable: the instants at which requests reach the server; the statuses calls end with; the
attempts a call sent (`grpc-previous-rpc-attempts` as the server reads it); the counters of 7.8.

**Deterministic under paused time.** The estimate and the cells are functions of state and an
`Instant`: `record(outcome, now)`, `healthy(now)`, `cap(now)`, and `admit(kind, now) -> Admit |
Wait(until) | Refused(Health | Capacity)`, with integer counts and nanoseconds, so unit tests need
no runtime. Channel tests use `#[tokio::test(start_paused = true)]` with `tokio::time::advance`, as
`call.rs` uses `start_paused`, and the units take explicit instants, as `backoff.rs` does;
`test-util` is already a dev-dependency. A real server on a paused clock is a hazard, since time
auto-advances when the runtime idles and a timer awaited beside socket I/O can fire early: keep each
end-to-end test to timers the limiter alone owns, and give each behaviour one real-time variant with
short windows, as `tests/grpc_rate_limit.rs` does. The backoff draw is the only randomness.
`retry.rs` calls `fastrand` directly, and only `backoff.rs` takes the draw as an argument, so a seam
has to be added; until then assert bounds, not values.

Estimate, unit tests with explicit instants:

1. One test per row of the table in 4.2: the outcome records as an accept, a reject or nothing,
   including each origin (trailers, HTTP status, reset reason, dial, GOAWAY-unprocessed, local). A
   non-negative pushback on a failed attempt is a reject whatever the code; a negative or unreadable
   one is decided by the code.
2. Ring: a count recorded at `t` is in the window until between `t + 11 W / 12` and `t + W`, and out
   after; an idle gap of several slots leaves none of them in the window, and so does a very long
   one, after which a record replaces the stale slot and is counted; a slot at its count limit
   halves both counts and keeps the ratio, and the interval bits are never disturbed; a gap that is
   exactly 2^20 slots reads the old counts as current, which the test documents.
3. `E = R - K * A` against hand-computed cases: 600 rejected against 6,600 accepted is healthy at
   `K` 2; 7,000 against 5,000 is not; a lone rejection on an idle channel is healthy at `S` 10 and
   not at `S` 0.
4. The figures of 4.10 for `lambda` 60, `n` 5, `W` 30, each within a slot of 2.5 s: no trip at 3 s;
   retries stop between 5 and 9 s; the cap falls under the offered rate at about 15 s and reaches
   the floor at about 30 s; after a 30 s outage the channel reopens within 30 s of the server
   returning, and about 5 s after an outage of 2 `W`, when the window held only probes. With `n` 1
   the retries stop at about 15 s.
5. The cap: no cap while healthy; just past `S` it is within `S / W'` of the window's average send
   rate; it never falls under the floor; with `K` 1 the channel trips on rejections beyond `S`
   whatever it accepts.
6. Under many threads, the ring and the cells with a hand-driven clock that all threads read at one
   instant in each phase: after any interleaving of records from eight threads the sum of the slots
   in the window equals the number of records in it; eight threads taking a cell at one instant
   leave `TAT` at `max(TAT, now)` plus one `T` per take, as one thread would; a waiter that joins is
   never passed by a newcomer or a retry that reads the count after it, and a call leaving the
   adaptive queue for the ceiling's is never passed in between; a waiter dropped by a deadline or a
   cancel leaves the count as it was; and a model check with `loom`, a dev-dependency built only for
   that test with `cfg(loom)`, of two threads recording across an interval boundary and taking one
   cell, small enough to be exhaustive.
7. On the C ABI's runtime, a current-thread runtime: the channel tests run under `start_paused`.
8. On a multi-thread runtime, as the `armonik` crate's: `time::pause` needs a current-thread
   runtime, so its channel tests run in real time with short windows (a `WindowSeconds` of 1.2 and a
   floor that keeps them under a second), driving one channel from several threads.
9. The same scripted scenario, a server dead and then up, run under the current-thread runtime with
   paused time and under the multi-thread runtime in real time with scaled windows, classifies,
   trips and reopens in the same order; the order is compared, not the instants.

Cells, unit tests with explicit instants:

10. From idle, `Calls` first attempts start at once and the next `T` later; no sequence starts more
    than `B + t / T` in any interval `t` (G2, checked against a sliding log over generated runs).
11. `Calls` requests at the end of one `PerSeconds` and `Calls` at the start of the next do not all
    start at once.
12. Reserve: with `B` 100 a retry at level 50 passes and at 49.9 is refused; with no first attempt
    between them, after any run of retries the level is at least `F`, so `F` first attempts that
    follow start at once (G1); with `Calls` 1 a retry passes when the cell is free; with `F` 0 a
    retry is refused while a first attempt waits.
13. G3: first attempts at 0.6 of the rate, retries offered at the rate: admitted retries stay within
    0.4 of it plus the burst.
14. A waiting first attempt that gives up takes no turn and the next waiter starts when due, as
    `a_call_that_stopped_waiting_takes_no_turn_and_holds_none_back` has it. A newcomer does not pass
    a waiter on the fast path.
15. When the channel turns healthy the first decision resets the adaptive `TAT`, so a probe taken at
    the floor leaves nothing to wait for after the cap lifts; a reset that loses to a take leaves
    the debt for the next healthy decision.

Channel, paused clock:

16. A retry at the failure: healthy, it is considered; unhealthy, the call ends at once with its
    status and no backoff is slept; unhealthy when its backoff ends, the same.
17. With the estimate enabled a capacity refusal ends the call with its last status and sends
    nothing more; with it disabled the refusal takes the first rule of 4.6: nothing sent, the next
    backoff, a count toward `MaxAttempts`, no gap in `grpc-previous-rpc-attempts`.
18. Poison call: with the `GrpcClient` preset, a call that always fails `UNKNOWN`, deadline an hour,
    sends exactly `MaxAttempts` requests and ends `UNKNOWN`, and `E` falls by `MaxAttempts`; with
    `GoogleRpc` it sends one.
19. A slow limit (`Calls` 1, `PerSeconds` 10) refuses every retry; with the estimate disabled the
    call skips until the ceiling is spent, with it enabled the call ends at its first failure.
20. A server scripted dead for 60 s and then up: first attempts over the cap wait in order and none
    is refused; the server sees probes at the floor; a waiting call with a deadline ends
    `DEADLINE_EXCEEDED` having sent nothing, one with none is served as a probe and ends
    `UNAVAILABLE`, about `1 / r_a` after the one ahead of it; a call waiting for the adaptive turn
    has taken no ceiling turn, so when the cap lifts the ceiling cell is as full as it was; and with
    a ceiling cell too the calls leave in the order they arrived. When the estimate reopens the
    waiters are released in order, and a call that arrives while the adaptive queue is not empty
    queues behind it instead of taking the fast path.
21. No rate limit, estimate enabled, server dead: the retries stop after `W / (n + 1)`, first
    attempts go uncapped until the cap falls under the offered rate; with `Adaptive` `Off` none of
    this happens and the channel is as today's.
22. A GOAWAY with 50 calls in flight: the resends are paced by the ceiling cell and record nothing;
    a `REFUSED_STREAM` records a reject.
23. Wait-for-ready, with `Calls` 10 and `PerSeconds` 1: 50 calls waiting for a connection that opens
    at `t` start ten at once and then one every 100 ms, and a waiting call holds no turn;
    `grpc-timeout` states what remains after the turn's wait.
24. Mapping (host side): a service config's fields give the options of section 5, with `Multiplier`
    `1 + tokenRatio` and `Slack` `ceil(maxTokens / 2)`; `retryableStatusCodes` becomes a `List`;
    `hedgingPolicy` is refused naming its field.
25. Vocabulary: the new keys exist in the schema, the generated C# and the loader; an unknown
    spelling is logged with its path; a channel stating `{"On": {"Multiplier": 3}}` over defaults
    stating `On` keeps the defaults' `Slack`, `Off` over `On` and the reverse is taken whole, and an
    `Adaptive` absent from the merged document is `On` with the defaults; a document naming `On` and
    `Off` is refused as it is read, an out-of-bounds value when the channel is created, and the
    defaults' naming `ChannelDefaults`. `Grpc.Retry.Codes`: `GoogleRpc` over `GrpcClient` is taken
    whole, a `List` over a `List` replaces the array, and an empty list, `OK` and a name that is not
    a status code are refused naming the key.
26. Retryable codes: with `GoogleRpc` a call failing `ABORTED` or `UNKNOWN` is not retried and one
    failing `UNAVAILABLE` is; with `GrpcClient` all three are; with a `List` exactly those named.

### Validating the defaults

`K`, `S`, `W` and the floor are the SRE book's and AWS's, and a judgment for `S` and `W`; the plan
below shows whether they suit an ArmoniK control plane. It runs against a scripted server first and
a real deployment after, and each scenario names what would show a default wrong.

1. **No false trips.** A server that refuses a steady share `f` of attempts at random, `f` from 0.05
   to 0.6, with callers at 1, 60 and 600 calls a second, for an hour each. At 60 calls a second or
   more the window holds enough attempts for the threshold to be sharp. Right: no trip for `f` up to
   0.45, a trip within `W` for `f` from 0.6. At 1 call a second the window holds about 30 attempts
   and `E` has a standard deviation of about 5, so the criterion is the rate of false trips and not
   zero: none for `f` up to 0.3 over the hour, and a trip at `f` 0.3 shows `S` is too small. A trip
   at 60 a second under 0.45, or none at 600 a second at 0.6, shows `S` or `W` is wrong.
2. **Outage length.** The server dead for 3, 5, 10, 20 and 60 s, then up, at 60 calls a second with
   the default policy. Measure the retries sent before the trip, the time to the trip, the time to
   reopen. Right: the trip near `W / (n + 1)` and the reopening within `W`, as 4.10 derives; the
   figures are the check of the derivation as much as of the defaults. Wrong: a rolling restart that
   takes 10 s losing calls that the baseline would have saved (then `W` is too short).
3. **The real restart.** A rolling restart of the control plane under a representative load,
   measuring calls that end in error, retries sent and time to full rate, against the same run with
   `Adaptive` `Off`. Right: no more failed calls than the baseline, fewer requests to the server
   while it is down.
4. **Overload.** A server of capacity `C` offered 2 `C` and 4 `C`, with load shedding. Measure the
   requests it receives against what it accepts, and the goodput. Right: requests at most `K` times
   the accepts, goodput no lower than the baseline's.
5. **Low traffic.** One call a second against a server that fails 30% of its calls and one that dies
   for a minute. Right: the first keeps its retries; the second stops them within `S` rejections and
   recovers within `W` of the server's return.
6. **A flapping server**, up and down every 5 to 20 s. Right: no sustained oscillation of the cap;
   the number of cycles between healthy and unhealthy is at most one per `W`.
7. **Sweeps**, each parameter alone with the others at their defaults: `K` in 1.5, 2, 3; `S` in 0,
   10, 50; `W` in 10, 30, 120; the floor in 0.1, 0.5, 2. Report, for scenarios 1 to 6, the false
   trips, the failed calls, the retries and requests sent, and the time to reopen. A default is kept
   unless a neighbouring value is better on one metric and no worse on the others.
8. **Which codes end calls.** In the real deployment, count the statuses that end calls and the
   attempts that are retried, by code, to settle Q5.

## 10. Not proposed

- **A fixed-ratio ledger** (4.9 (a)): compared, not built; it would be added only if the estimate
  proves unreliable, and the combined bound would read `retries/s <= min(rate - lambda_f, max(floor,
  ratio * lambda_f))`.
- **A ramp after reopening** (AIMD or CUBIC on `T`): Q10.
- **A latency signal or a concurrency limit** (Netflix, Envoy's retry concurrency): the estimate is
  blind to a server that slows without refusing (4.2). Concurrency is the right bound when latency
  moves and the rate does not (2.5); it needs latency samples and would sit beside the cells.
- **A sliding window rate limit**: exact but O(Calls), or approximate; GCRA is neither.
- **Hedging, per-method policies, client-side load balancing**: unchanged, not wanted.

## 11. Sources, and what was not read in full

gRPC: A6, the grpc.io retry guide, connection backoff, wait-for-ready, and the throttle code of
grpc-java, grpc-go, grpc core and grpc-dotnet (links in 2.1 and 2.7). Overload: the Google SRE book
chapters, Brooker, the AWS Builders' Library and SDK reference, the AWS jitter study (2.2 to 2.4,
2.7). Budgets and breakers: Finagle, Linkerd, tower and Envoy (2.3). Limits and control laws:
Netflix, Little, Cloudflare, Go `x/time/rate`, `governor`, GCRA, Polly (2.5, 2.6, 2.9).
Metastability: Bronson et al. and Huang et al. (2.8), both read as text.

Constants quoted for Finagle, tower, Envoy, the AWS quota and CUBIC are those of the pages and
sources cited; Linkerd's budget defaults are not given by the pages cited. Netflix's AIMD figures
are from the library's source, not its blog post. The 12 of 22 incidents in 2.8 is the paper's own
table; its text says more than half.