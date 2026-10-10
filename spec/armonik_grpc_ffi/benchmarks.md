# Benchmarks: native against managed

The baseline requirement 16 asks for: the native engine through `NativeChannel` against
grpc-dotnet through `GrpcChannelFactory`, on .NET Framework 4.8, .NET 8 and .NET 10, the last two
just-in-time and ahead-of-time, measured with the engine of the commit that adds these figures.
It is a baseline, not a verdict: one machine, one server, the loopback interface, and numbers to
compare the next measurement with.

## What is measured

`ArmoniK.Api.Client.RustGrpcChannel.Benchmarks` measures one transport per process, so that what
one leaves allocated is not counted against the other, against the test server over TLS on
loopback. TLS for both, because grpc-dotnet on .NET Framework speaks HTTP/2 only over it.

- **Unary latency**: 1 000 warm-up calls, then 10 000 sequential `Say` calls of a one-character
  message, each timed alone. P50, P95 and P99 in microseconds.
- **CPU per call**: the process's CPU time over those 10 000 calls, every thread's, per call.
- **Allocation**: the managed bytes the process allocates over those calls, per call, and on .NET
  the thread pool's work items per call.
- **Download**: `Stream` sends 2 000 chunks of 64 KiB, 125 MiB; five runs, the median in MiB/s.
  At 64 KiB a chunk, 16 messages a second per MiB/s.
- **Upload**: 150 MiB sent to the server, five runs of each, the median in MiB/s: as a client
  stream of 64 KiB chunks (`Upload`), and as one unary message (`UploadWhole`); and the process's
  CPU time per MiB sent over each one's five runs.
- **Memory**: what the process holds after the latency and the download, less what it held
  before the channel was opened, each read after a full collection: private bytes, and the
  managed heap. Read with the channel still open, so each stack's one-time costs - loading,
  compiling - are in it. The private bytes are read again after the uploads, the 150 MiB message
  unreachable by then.

Every transport option is left at its default, the runtime's ceilings included.

Every measurement runs in two scenarios. **Idle**: the benchmark is all the process does, so the
.NET thread pool sleeps between calls. **Busy**: two pool workers compute for 50 us and yield,
over and over, as in an application whose pool is never idle. A busy run reports no CPU and no
memory, and the allocation it reports counts the load's too: the tables leave it out.

On .NET the benchmark runs just-in-time, and ahead-of-time (`-aot`), published with Native AOT,
whose thread pool on Windows is, by default, the operating system's rather than .NET's portable
one.

## Results

Each cell is the median of three passes, the runs of a pass interleaved, every run on the
performance cores at AboveNormal priority (see the machine below). Two campaigns: the first runs
every framework just-in-time, the second .NET 8 and .NET 10 just-in-time and ahead-of-time, and
adds the CPU per MiB uploaded. In the second, the unary column also counts building the 150 MiB
message, once for its five sends: one copy, which these figures do not separate. The benchmark as
it is leaves it out.

Just-in-time, idle:

| Host | Transport | P50 (us) | P99 (us) | Download (MiB/s) | Upload, stream (MiB/s) | Upload, unary (MiB/s) | CPU per call (us) | Allocated per call (B) |
|------|-----------|---------:|---------:|-----------------:|-----------------------:|----------------------:|------------------:|-----------------------:|
| .NET Framework 4.8 | native | 432 | 977 | 236 | 314 | 222 | 808 | 4691 |
| .NET Framework 4.8 | managed | 2960 | 7555 | 184 | 46 | 188 | 5220 | 36139 |
| .NET 8 | native | 404 | 1374 | 247 | 329 | 280 | 394 | 3627 |
| .NET 8 | managed | 386 | 1305 | 228 | 79 | 79 | 691 | 7981 |
| .NET 10 | native | 375 | 1346 | 251 | 316 | 235 | 450 | 3629 |
| .NET 10 | managed | 381 | 1652 | 206 | 81 | 78 | 841 | 7990 |

Just-in-time, busy:

| Host | Transport | P50 (us) | P99 (us) | Download (MiB/s) | Upload, stream (MiB/s) | Upload, unary (MiB/s) |
|------|-----------|---------:|---------:|-----------------:|-----------------------:|----------------------:|
| .NET Framework 4.8 | native | 454 | 720 | 287 | 241 | 214 |
| .NET Framework 4.8 | managed | 3604 | 5171 | 193 | 40 | 181 |
| .NET 8 | native | 481 | 746 | 326 | 257 | 201 |
| .NET 8 | managed | 418 | 744 | 258 | 98 | 92 |
| .NET 10 | native | 431 | 1021 | 264 | 255 | 213 |
| .NET 10 | managed | 476 | 956 | 275 | 102 | 87 |

Just-in-time against ahead-of-time, idle:

| Host | Transport | P50 (us) | P99 (us) | Upload, stream (MiB/s) | Upload, unary (MiB/s) | CPU per call (us) | CPU per MiB, stream (us) | CPU per MiB, unary (us) |
|------|-----------|---------:|---------:|-----------------------:|----------------------:|------------------:|-------------------------:|------------------------:|
| .NET 8 | native | 459 | 1914 | 241 | 181 | 473 | 6438 | 4271 |
| .NET 8 | managed | 492 | 1904 | 73 | 71 | 778 | 17000 | 13771 |
| .NET 8 AOT | native | 641 | 1888 | 222 | 170 | 450 | 5000 | 4146 |
| .NET 8 AOT | managed | 410 | 1209 | 139 | 115 | 277 | 7688 | 7708 |
| .NET 10 | native | 343 | 1105 | 276 | 220 | 438 | 5042 | 3792 |
| .NET 10 | managed | 400 | 1620 | 76 | 74 | 842 | 15062 | 13271 |
| .NET 10 AOT | native | 404 | 1448 | 250 | 240 | 292 | 4479 | 3188 |
| .NET 10 AOT | managed | 336 | 884 | 135 | 121 | 258 | 8188 | 7917 |

What the numbers say:

- **On .NET Framework the native engine is the faster transport in every respect**: about 7
  times lower at P50 idle and 8 times busy, a sixth of the CPU per call, an eighth of the
  allocation, and a streamed upload 6 to 7 times faster. grpc-dotnet there runs over
  `WinHttpHandler`, which sends a whole message better than a stream.
- **On .NET 8 and .NET 10 just-in-time, the latencies are within the spread**, and the native
  engine spends 40 to 50 % less CPU per call, allocates half as much, runs 2.0 thread pool work
  items per call against 9.2, ahead-of-time too, and uploads faster for
  about a third of the CPU per MiB: 3.3 to 4.2 times streamed and 2.5 to 3.5 times unary, across
  the two campaigns.
- **Ahead-of-time, grpc-dotnet spends three times less CPU per call than just-in-time**, at or
  below the native engine's, uploads 1.6 to 1.9 times as fast, and is the faster transport for
  latency, in every pass: P50 410 against 641 us on .NET 8 and 336 against 404 on .NET 10, and
  P99 likewise. The native engine still uploads faster, 1.6 to 1.9 times streamed and 1.5 to 2.0
  times unary, for 35 to 45 % less CPU per MiB streamed and 45 to 60 % unary. The Windows thread
  pool, which does not spin, and the absence of just-in-time compilation are the likely reasons,
  the native engine's .NET side being thin; these runs do not separate them.
- **Downloads on .NET 8 and .NET 10 just-in-time are within the spread**, the medians within
  26 % of each other, either way; on .NET Framework the native engine is ahead.
- **Memory, idle**: native holds 11 to 15 MiB of private bytes and grows the managed heap by
  0.3 MiB, managed 14 to 21 MiB and 0.7 to 2.0 MiB. After the uploads, native stands 313 to 466
  MiB above its start, managed 179 to 1143, the most ahead-of-time on .NET 10.
- **After a 150 MiB send, the native channel keeps the 150 MiB arena** as a spare for the next
  send of that size, counted against the runtime's memory ceiling and freed first when the count
  nears it, which with the default ceiling is not reached here: the private bytes after the
  uploads hold it.
- **The passes spread.** Pinned to one kind of core, a cell's P50 still moves by up to about
  300 us across its passes on .NET, and 650 us for grpc-dotnet on .NET Framework; an upload's
  throughput or its CPU per MiB by up to a factor of two. The ratios above are between medians;
  take the upload ones as indicative. A difference smaller than the spread is not reported as
  one.

What this baseline does not settle: the HTTP/2 window default, which the audit response leaves to
these benchmarks, needs a link with latency. Loopback has none, so no window size shows here.

## After integration: the engine of `bf86fece3`

The baseline's three passes again, on 2026-10-10, with the engine of `bf86fece3`: the adaptive
throttle and the outbound traffic options, the strict configuration loader, the engine's counters
behind the `metrics` feature, and the two native builds. The benchmark is that of `cdd3ae0fa`,
which adds the `native-metrics` transport. Every transport option is left at its default, which is
this engine's. The runs are the baseline's: three passes, interleaved, each cell the median, on
the performance cores at AboveNormal priority, from the campaign script with `-Aot`, on the
machine below with no build, test or prover running when the campaign started. The desktop's
applications were open; the processor was 6 to 24 % busy in samples taken by hand before some
runs, which are not logged. The unary upload does not count building the 150 MiB message, so its
figures set against the baseline's second campaign differ in what they measure; against its first
they do not.

Just-in-time, idle:

| Host | Transport | P50 (us) | P99 (us) | Download (MiB/s) | Upload, stream (MiB/s) | Upload, unary (MiB/s) | CPU per call (us) | Allocated per call (B) |
|------|-----------|---------:|---------:|-----------------:|-----------------------:|----------------------:|------------------:|-----------------------:|
| .NET Framework 4.8 | native | 466 | 888 | 249 | 227 | 216 | 992 | 5197 |
| .NET Framework 4.8 | managed | 3438 | 5232 | 168 | 42 | 187 | 5838 | 36782 |
| .NET 8 | native | 431 | 840 | 313 | 224 | 205 | 392 | 4098 |
| .NET 8 | managed | 418 | 1021 | 241 | 84 | 83 | 892 | 8368 |
| .NET 10 | native | 430 | 814 | 325 | 237 | 231 | 445 | 4099 |
| .NET 10 | managed | 451 | 933 | 242 | 95 | 82 | 983 | 8352 |

Just-in-time, busy:

| Host | Transport | P50 (us) | P99 (us) | Download (MiB/s) | Upload, stream (MiB/s) | Upload, unary (MiB/s) |
|------|-----------|---------:|---------:|-----------------:|-----------------------:|----------------------:|
| .NET Framework 4.8 | native | 502 | 802 | 246 | 201 | 200 |
| .NET Framework 4.8 | managed | 3689 | 4521 | 174 | 38 | 167 |
| .NET 8 | native | 470 | 834 | 281 | 216 | 203 |
| .NET 8 | managed | 452 | 955 | 275 | 89 | 91 |
| .NET 10 | native | 441 | 866 | 267 | 218 | 210 |
| .NET 10 | managed | 459 | 928 | 243 | 88 | 94 |

Just-in-time against ahead-of-time, idle:

| Host | Transport | P50 (us) | P99 (us) | Upload, stream (MiB/s) | Upload, unary (MiB/s) | CPU per call (us) | CPU per MiB, stream (us) | CPU per MiB, unary (us) |
|------|-----------|---------:|---------:|-----------------------:|----------------------:|------------------:|-------------------------:|------------------------:|
| .NET 8 | native | 431 | 840 | 224 | 205 | 392 | 6875 | 3750 |
| .NET 8 | managed | 418 | 1021 | 84 | 83 | 892 | 15875 | 13479 |
| .NET 8 AOT | native | 424 | 906 | 231 | 228 | 300 | 4521 | 3479 |
| .NET 8 AOT | managed | 393 | 685 | 151 | 122 | 247 | 7812 | 7292 |
| .NET 10 | native | 430 | 814 | 237 | 231 | 445 | 7167 | 3583 |
| .NET 10 | managed | 451 | 933 | 95 | 82 | 983 | 15562 | 13250 |
| .NET 10 AOT | native | 417 | 810 | 249 | 216 | 303 | 4375 | 3667 |
| .NET 10 AOT | managed | 386 | 726 | 150 | 125 | 259 | 7604 | 7312 |

What the numbers say, the ranges of the three passes in brackets where a ratio rests on them:

- **On .NET Framework the native engine is the faster transport but for the unary upload**: P50
  466 (463-489) against 3438 (3339-3593) us, 5.9 times less CPU per call, 7 times less
  allocation, a streamed upload 5.4 times as fast, and a download 1.5 times. The unary upload is
  216 (188-220) against 187 (182-198) MiB/s: the ranges overlap, a tie.
- **On .NET 8 and .NET 10 just-in-time, P50 is a tie** (431 (415-432) against 418 (415-428) us on
  .NET 8; 430 (417-431) against 451 (437-468) on .NET 10, 5 % lower, the ranges 6 us apart: a lean,
  not a difference). P99 is 18 and 13 % lower for the native engine, its ranges clear of the
  other's on both (831-921 against 946-1400, 779-853 against 892-1000). The native engine spends 56
  and 55 % less CPU per call, allocates 51 % less, runs 2.0 thread pool work items per call against
  9.2, downloads 1.3 times as fast on both, its ranges clear of the other's, and uploads 2.5 to 2.8
  times as fast, streamed and unary, for 54 to 57 % less CPU per MiB streamed and 72 to 73 % less
  unary.
- **Ahead-of-time, grpc-dotnet is ahead on .NET 8 and leans ahead on .NET 10, for latency and CPU
  per call**: P50 393 (368-394) against 424 (424-455) us on .NET 8, clear by 30 us, and 386
  (376-412) against 417 (417-491) on .NET 10, 7 % lower on both, the ranges there 5 us apart, a
  lean of the size of the just-in-time one the other way; P99 685 (675-756) against 906 (815-940)
  on .NET 8, clear, and 726 (706-882) against 810 (782-1182) on .NET 10, overlapping; CPU per call
  247 against 300 and 259 against 303 us, 18 and 15 % lower. The downloads are a tie (304 against
  285, 305 against 297). The native engine still uploads 1.5 and 1.7 times as fast streamed and 1.9
  and 1.7 times unary, for 42 % less CPU per MiB streamed and 50 to 52 % unary.
- **Busy**: the .NET Framework result stands (P50 502 against 3689 us). On .NET 8 and .NET 10
  just-in-time the P50s are within 4 %, the native P99 13 and 7 % lower, the downloads a tie, the
  uploads 2.4 to 2.5 times as fast streamed. Ahead-of-time, grpc-dotnet is ahead at P50 (412
  against 449, 407 against 436 us) and P99 (702 against 799, 699 against 779).
- **Memory, idle**: the native engine holds 11.6 to 15.4 MiB of private bytes above where it
  started and grows the managed heap by 0.3 to 0.4 MiB; grpc-dotnet holds 13.8 to 23.1 and grows
  it by 0.7 to 1.8. After the uploads, native stands 315 to 467 MiB above its start, grpc-dotnet
  181 to 1812, the ranges of the .NET passes wide (848-1857 for .NET 10 just-in-time).
- **The passes spread** less than in the baseline: a cell's P50 moves by up to 74 us across its
  passes on .NET and 254 us for grpc-dotnet on .NET Framework, an upload's throughput by up to 27
  %; a difference smaller than the spread is not reported as one.

Against the baseline: the same machine and the same grpc-dotnet, but a test server that differs.
Its `Say` now answers with three more response headers and a response trailer (`x-saw-encoding`,
`x-saw-accept-encoding`, `x-saw-content-length`, `x-sent-encoding`), which both transports receive
and decode on every call, so the per-call figures are not like for like with the baseline's, for
either transport. grpc-dotnet's rows moved too: in the first campaign's idle table its P50 is 8 to
18 % higher on every host, its CPU per call 17 to 29 % higher on .NET, its P99 22 to 44 % lower on
.NET. For the native engine, in the same table:

- **Moved over 10 %, for the worse**: the streamed upload, 314 to 227 MiB/s on .NET Framework
  (-28 %), 329 to 224 on .NET 8 (-32 %) and 316 to 237 on .NET 10 (-25 %), while grpc-dotnet's
  rose 6 to 17 % on .NET; the unary upload on .NET 8, 280 to 205 (-27 %), the others within 3 %;
  the allocation per call, 3627 and 3629 to 4098 and 4099 B on .NET (+13 %) and 4691 to 5197 B on
  .NET Framework (+11 %), about 470 to 510 B more, grpc-dotnet's moving by 2 to 5 %; the CPU per
  call on .NET Framework, 808 to 992 us (+23 %), the .NET ones unchanged (394 to 392, 450 to 445);
  P50 on .NET 10, 375 to 430 us (+15 %), .NET 8 within 7 %.
- **Moved over 10 %, for the better**: P99, 39 and 40 % lower on .NET 8 and .NET 10 (9 % on .NET
  Framework), grpc-dotnet's 22 to 44 % lower on .NET; the download on .NET 8 and .NET 10, 247 to
  313 and 251 to 325 MiB/s (+27 and +29 %), grpc-dotnet's 6 and 17 % higher.
- **In the busy table**, the native P50 is 11 % higher on .NET Framework, its P99 11 % higher there
  and 12 % on .NET 8 but 15 % lower on .NET 10, its download 14 % lower on .NET Framework and .NET
  8, and its streamed upload 15 to 17 % lower on all three hosts; grpc-dotnet's streamed upload
  there is 5 to 14 % lower.
- **Against the second campaign's table**, a selection of the moves over 10 %: P99 is 26 to 56 %
  lower for the native engine in all four variants, 18 to 46 % for grpc-dotnet; the streamed
  upload's CPU per MiB on .NET 10 just-in-time is 5042 to 7167 us (+42 %), its P50 343 to 430 us
  (+25 %) and its streamed upload 276 to 237 MiB/s (-14 %); on .NET 8 ahead-of-time the native P50
  is 641 to 424 us (-34 %) and its CPU per call 450 to 300 (-33 %); the native CPU per call on .NET
  8 just-in-time is 473 to 392 us (-17 %).
- **No cause is measured here**, and the engine has changed in many ways since the baseline; these
  runs do not separate the commits, nor the engine from the server. The native streamed upload fell
  25 to 32 % on .NET and .NET Framework while grpc-dotnet's rose 6 to 17 % on .NET; the native
  allocation per call rose 11 to 13 % and grpc-dotnet's 2 to 5 %.

## What the metrics build costs

The native transport alone, idle, .NET 8 and .NET 10 just-in-time, on the performance cores at
AboveNormal priority: the default engine against the one built with `metrics`, the `native` and
`native-metrics` runs interleaved, seven passes on 2026-10-11. `native-metrics` selects the build
with `NativeLibrarySelection.Select(NativeEngineBuild.Metrics)` before the first runtime exists.
The benchmark listens to none of the meters, so what this prices is the counting, not the
reading. No build, test or prover was running when it started, the desktop's applications were
open, and its cells spread more than the first campaign's. Each cell is the median, the range of
the passes in brackets.

| Host | Engine | P50 (us) | P99 (us) | CPU per call (us) | Allocated per call (B) | Pool work items per call | Upload, stream (MiB/s) |
|------|--------|---------:|---------:|------------------:|-----------------------:|-------------------------:|-----------------------:|
| .NET 8 | default, campaign above | 431 | 840 | 392 | 4098 | 2.02 | 224 |
| .NET 8 | default | 450 (428-537) | 1076 (770-1253) | 441 (366-491) | 4098 (4097-4099) | 2.02 | 212 (187-224) |
| .NET 8 | metrics | 444 (430-551) | 1020 (836-1237) | 405 (377-478) | 4099 (4098-4102) | 2.03 | 213 (162-240) |
| .NET 10 | default, campaign above | 430 | 814 | 445 | 4099 | 2.03 | 237 |
| .NET 10 | default | 458 (425-495) | 1004 (841-1217) | 492 (467-542) | 4099 (4098-4102) | 2.03 | 203 (183-229) |
| .NET 10 | metrics | 489 (421-614) | 1077 (836-1402) | 488 (431-620) | 4098 (4098-4099) | 2.02 | 209 (188-222) |

- **The metrics build costs no difference larger than the spread**: every range of the metrics
  engine overlaps the default's. The medians sit 1 % below to 7 % above at P50, 5 % below to 7 %
  above at P99, and 8 % below to 1 % below in CPU per call; the signs of P50 and P99 differ between
  the hosts, and the CPU per call is lower on both. A first run of five passes on 2026-10-10, in
  which the default engine's P50 was higher (566 us on .NET 8), gave P50 566 (default) against 500
  (metrics) us on .NET 8 and 483 against 483 on .NET 10, CPU per call 488 against 453 and 486
  against 533 (+10 %, the ranges 431-556 and 495-538 overlapping).
- **The allocation per call is the same**, to within one byte; the benchmark counts managed
  bytes, so what the engine allocates natively is not in it.
- **The default engine's own medians differ between the first table here and this one**, the
  same cell on a later day, by up to 13 % in CPU per call, 28 % in P99 and 14 % in the streamed
  upload: the size a difference would have to exceed.
- The counting is not shown to be free: seven passes do not see a cost under a cell's spread, which
  is up to 193 us at P50 and up to 189 us of CPU per call, the metrics build's .NET 10 cells.

## h2-batch, where it acts

The native transport against `native-batch`, on .NET 8 just-in-time, idle, on the performance cores
at AboveNormal priority, with no build, test or prover running when it started, five passes
interleaved on 2026-10-11, the engine of `bf86fece3` as it is and built against h2-batch's patch
(`-p:NativeEngineH2Batch=true`, 16 DATA frames per write). Each cell is the median, the range of
the passes in brackets. The CPU is the process's, every thread's, per MiB sent over the five runs
of each upload.

| Engine | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) | P50 (us) |
|--------|-----------------------:|-------------------------:|----------------------:|------------------------:|---------:|
| native | 219 (193-224) | 7354 (7271-7812) | 199 (172-221) | 3833 (3583-4021) | 497 (450-577) |
| native-batch | 340 (327-369) | 5542 (4583-5812) | 309 (291-336) | 2188 (1938-2375) | 446 (438-485) |

- **The streamed upload is 1.55 times as fast, for 25 % less CPU per MiB**; its range, 327 to 369,
  is clear of the engine's without it, 193 to 224. The unary upload is 1.55 times as fast for 43 %
  less CPU per MiB, its range clear too. The T6.13 section's figures for the same comparison are
  1.5 to 1.6 times and 26 to 32 % streamed, 1.2 to 1.5 times and 36 to 48 % unary: these agree
  within the spread of five passes.
- **P50 is unchanged within the spread** (the ranges 450-577 and 438-485 overlap), as there: a
  unary call of one character fills no second frame. The `native` row's 497 us is 15 % above the
  431 of the campaign above, a cell of this run, not a difference between the engines.
- **Syscalls per send are not counted.** Counting them on Windows takes an elevated event
  tracing session (the kernel logger, `xperf` or `logman`), which this machine's shell is not,
  and the process monitor is not installed; the figure of the patch's README, 1245 to 286
  syscalls for a 16 MiB upload over TCP loopback, is a measurement of its own and is not
  repeated here.

## T6.13 and h2-batch

A measurement of its own, not the baseline's engine nor its passes: the native transport alone,
idle, on .NET Framework 4.8 and .NET 8 just-in-time, on 2026-10-07 on the machine below. Three
engines: before T6.13 (`d8af8f079`); after it, at `21a0b7807`, which also carries the four
commits after T6.13 - the Rust client on the engine, and a cancelled call's request reset rather
than ended, which adds a flag read when a request body ends; and that engine built
against h2-batch's patch, run as `native-batch`, 16 DATA frames per write. Before and after, six
passes each, a pass of one interleaved with a pass of the other; h2-batch, three passes,
interleaved with the first three. Each cell is the median, the range of the passes in brackets.

| Host | Engine | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) | P50 (us) |
|------|--------|-----------------------:|-------------------------:|----------------------:|------------------------:|---------:|
| .NET Framework 4.8 | before T6.13 | 340 (319-350) | 7260 (6833-8104) | 252 (195-272) | 3240 (3146-4208) | 346 (332-423) |
| .NET Framework 4.8 | `21a0b7807` | 322 (266-358) | 7396 (6583-9125) | 242 (201-274) | 3376 (3167-4146) | 342 (323-359) |
| .NET Framework 4.8 | `21a0b7807`, h2-batch | 515 (414-534) | 5042 (4792-5396) | 374 (345-378) | 1750 (1604-1854) | 360 (346-373) |
| .NET 8 | before T6.13 | 271 (246-328) | 5750 (5042-6958) | 247 (194-295) | 3406 (2833-4000) | 383 (315-448) |
| .NET 8 | `21a0b7807` | 318 (270-339) | 5448 (5146-5542) | 261 (223-299) | 3364 (2667-3542) | 356 (310-553) |
| .NET 8 | `21a0b7807`, h2-batch | 465 (412-539) | 4042 (3521-4188) | 312 (312-356) | 2167 (1792-2229) | 344 (314-390) |

What they say:

- **T6.13 shows no difference larger than the spread**, on either host, in any column: every
  range of `21a0b7807` overlaps the engine's before it. Probably because the copy it removes, one
  of 64 KiB per message, is small against the rest of what a message costs on its way out, and
  loopback does not make it the bottleneck.
- **h2-batch is a difference.** Against `21a0b7807`, the streamed upload is 1.5 to 1.6 times as
  fast, its range clear of the engines' without it on both hosts, for 26 to 32 % less CPU per MiB;
  the unary upload 1.2 to 1.5 times, for 36 to 48 % less CPU per MiB. Latency is unchanged within
  the spread: a unary call of one character fills no second frame.
- Three passes showed a gap six did not confirm: the medians of the first three put the .NET
  Framework streamed upload of `21a0b7807` 22 % below the engine's before it. h2-batch's gap, 46
  to 60 % over the six-pass median of the same engine, is wider than any range.

## The machine

Windows 11 Pro 10.0.26100, 13th Gen Intel Core i7-1360P, 16 GB, with the desktop's usual
applications open and no other build or test running. The i7-1360P is a hybrid processor: four
performance cores, logical processors 0 to 7, and eight efficiency cores. A unary call takes about
twice as long on the second kind, and a run the scheduler spreads over both measures where its
threads landed, so every run is held to logical processors 0 to 7, the test server with it, at
AboveNormal priority.

Built with .NET SDK 11.0.100-rc.1; run on .NET 8.0.19, .NET 10.0.12 and .NET Framework 4.8,
ahead-of-time with the 8.0.31 and 10.0.12 ILCompiler packages; rustc 1.94.1, the engine built
with the release profile. The client and the server share the machine. The runs of `bf86fece3`
used the same SDK, runtimes, ILCompiler packages and compiler.

## Running it again

On Windows 11 or Windows Server 2022 or later: on .NET Framework the managed side runs over
`WinHttpHandler`, which carries gRPC there only. From a Visual Studio x64 developer PowerShell, in
`packages/csharp/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks`, `campaign.ps1` builds, publishes
the ahead-of-time variants, runs the passes interleaved and prints the medians:

```powershell
.\campaign.ps1 -Frameworks net4.8,net8.0,net10.0 -Aot -Affinity 0xFF -Priority AboveNormal
```

`-Affinity` takes a mask of logical processors, the performance cores on a hybrid processor.
`-Dotnet` names the SDK that builds, and `-DotnetRoot` the install that holds the .NET 10 and .NET
11 runtimes when the system-wide one does not, which `-DotnetRootFor` lists. A developer prompt's
linker is passed to the ahead-of-time publish with `-PublishArgs -p:IlcUseEnvironmentalTools=true`.
`-Transports` and `-Scenarios` choose what runs, `native` and `managed`, idle and busy, by default;
`native-batch` needs `-BuildArgs -p:NativeEngineH2Batch=true`, which builds the engine against
h2-batch's patch and takes Git Bash first on the PATH, and a build without it refuses the
option. `native-metrics` loads the engine's build with its counters, which the benchmark's build
copies beside it unless `-p:NativeEngineMetrics=false`, and which it selects with
`NativeLibrarySelection` before the first runtime exists; the published ahead-of-time
benchmarks do not carry that build, so it does not run there. The h2-batch run builds the engine
over the one the other transports use, so a run of both builds takes a copy of the output
directory before the second build. A run that fails is reported and left out, and a cell left
with fewer passes says so.

A single run is the benchmark itself, with the transport and `busy` for the busy scenario:

```bash
bin/Release/net8.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native busy
```

It prints one line: the latencies, the download and the two uploads, the allocation per call and
on .NET the thread pool work items, and for an idle run the CPU per call and per MiB uploaded and
the memory before and after the uploads. `ARMONIK_BENCH_AFFINITY` and `ARMONIK_BENCH_PRIORITY` set
where it and its server run. The benchmark starts its test server, a `net8.0` program, with the
`dotnet` the system finds first, which has to be an install with a .NET 8 runtime.
