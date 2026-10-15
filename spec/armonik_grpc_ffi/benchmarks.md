# Benchmarks: native against managed

The baseline requirement 16 asks for: the native engine through `NativeChannel` against
grpc-dotnet through `GrpcChannelFactory`, on .NET Framework 4.8, .NET 8 and .NET 10, the last two
just-in-time and ahead-of-time, measured with the engine of the commit that adds these figures.
It is a baseline, not a verdict: one machine, one server, the loopback interface, and numbers to
compare the next measurement with.

## What is measured

`ArmoniK.Api.Client.RustGrpcChannel.Benchmarks` measures one transport per process, so that what
one leaves allocated is not counted against the other, against the test server over TLS on
loopback. TLS for both, because grpc-dotnet on .NET Framework speaks HTTP/2 only over it; the
`-h2c` transports, which run in the clear, are named where their figures are given.

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
- **Calibration**: the fewest microseconds of five tries of a fixed computation, before the server
  starts and again once the rest is measured and a busy run's load has stopped (`calib_us`,
  `calib_end_us`). It measures the processor, whose speed follows a power and thermal state that
  neither the affinity nor the priority holds: runs whose calibrations differ ran on processors
  of different speeds. Comparable only between runs of one framework and one build.

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

## The channel thread's spin, TLS's share, and the driver's trim

Two campaigns of 2026-10-15, both run entirely after the machine's last wake (11:59:17; the System
log, Id 42, 107 and 1, shows no sleep, wake or clock change from then to their end), on the
performance cores at AboveNormal priority, each run started once no build, test or prover ran
and the processor was under 30 % busy. Another session's builds started during nine runs of the
first two passes; those runs, and the runs whose first calibration is above 8 000 us, a
processor slower than the campaign's, are left out. Two runs left no line in the campaign's log:
.NET 10 native idle in pass 3 and .NET 10 AOT native-nospin busy in pass 1. Each cell says how many passes it keeps (n). The calibrations of
the runs kept read 7 526 to 7 990 us at their start, so these figures compare with each other
and not with the tables above. The end calibration, read after a run, often reads 12 700 or
more: the processor slows over a run, which the interleaving shares out between the variants.

### The spin

`native` is the engine with its default `BeforeSleep`, a spin of 100 us after each callback into
the host (`decisions.md`); `native-nospin` the same engine with `"None"`; `managed` grpc-dotnet.
The engine of `276a8c5a5`. Five passes interleaved, 12:27 to 13:39; each cell the median of the
passes kept, the range in brackets.

Idle:

| Host | Transport | n | P50 (us) | P99 (us) | CPU per call (us) | Download (MiB/s) | Upload, stream (MiB/s) | Upload, unary (MiB/s) |
|------|-----------|--:|---------:|---------:|------------------:|-----------------:|-----------------------:|----------------------:|
| .NET 8 | native | 4 | 270 (252-288) | 447 (432-622) | 297 (286-341) | 474 (358-589) | 296 (281-414) | 380 (264-421) |
| .NET 8 | native-nospin | 4 | 255 (238-277) | 498 (461-1108) | 262 (217-269) | 528 (421-591) | 346 (285-451) | 371 (336-491) |
| .NET 8 | managed | 4 | 310 (296-351) | 763 (497-1313) | 576 (531-691) | 244 (223-258) | 116 (109-131) | 97 (91-104) |
| .NET 10 | native | 4 | 250 (240-267) | 536 (418-704) | 347 (331-372) | 415 (338-570) | 288 (284-430) | 445 (274-474) |
| .NET 10 | native-nospin | 3 | 286 (242-288) | 747 (491-916) | 323 (278-338) | 410 (294-475) | 290 (199-455) | 471 (436-491) |
| .NET 10 | managed | 4 | 294 (283-330) | 672 (530-1098) | 622 (594-642) | 262 (244-305) | 96 (88-119) | 102 (95-111) |
| .NET 8 AOT | native | 4 | 258 (253-303) | 523 (419-1000) | 210 (198-241) | 451 (344-509) | 354 (323-374) | 283 (269-493) |
| .NET 8 AOT | native-nospin | 4 | 274 (252-302) | 526 (390-1066) | 180 (164-192) | 436 (360-504) | 374 (316-471) | 334 (234-447) |
| .NET 8 AOT | managed | 4 | 254 (247-260) | 928 (644-1184) | 172 (145-183) | 482 (408-565) | 186 (170-205) | 219 (183-234) |
| .NET 10 AOT | native | 4 | 246 (240-299) | 839 (400-1680) | 200 (180-330) | 528 (459-540) | 455 (331-472) | 313 (224-416) |
| .NET 10 AOT | native-nospin | 5 | 272 (249-297) | 663 (392-900) | 180 (164-191) | 411 (353-547) | 348 (320-450) | 413 (278-448) |
| .NET 10 AOT | managed | 5 | 251 (220-288) | 486 (349-1005) | 144 (122-173) | 533 (494-597) | 204 (182-242) | 209 (158-229) |

Busy:

| Host | Transport | n | P50 (us) | P99 (us) | Download (MiB/s) | Upload, stream (MiB/s) | Upload, unary (MiB/s) |
|------|-----------|--:|---------:|---------:|-----------------:|-----------------------:|----------------------:|
| .NET 8 | native | 5 | 303 (296-352) | 627 (481-989) | 283 (265-380) | 472 (445-481) | 377 (342-385) |
| .NET 8 | native-nospin | 3 | 312 (280-340) | 526 (484-526) | 281 (225-287) | 469 (455-471) | 385 (291-389) |
| .NET 8 | managed | 5 | 358 (305-432) | 646 (542-1284) | 284 (242-334) | 129 (125-134) | 126 (102-128) |
| .NET 10 | native | 5 | 348 (265-360) | 577 (456-781) | 281 (267-299) | 452 (331-487) | 348 (268-404) |
| .NET 10 | native-nospin | 5 | 330 (319-342) | 525 (498-796) | 270 (222-285) | 481 (441-493) | 283 (279-340) |
| .NET 10 | managed | 5 | 398 (382-412) | 633 (559-1523) | 318 (265-368) | 111 (104-145) | 117 (114-127) |
| .NET 8 AOT | native | 5 | 273 (256-276) | 564 (422-819) | 283 (273-366) | 365 (281-443) | 420 (401-425) |
| .NET 8 AOT | native-nospin | 5 | 270 (270-279) | 443 (410-470) | 359 (271-414) | 273 (254-433) | 370 (302-432) |
| .NET 8 AOT | managed | 4 | 260 (257-264) | 398 (389-612) | 335 (308-389) | 194 (158-206) | 142 (134-156) |
| .NET 10 AOT | native | 5 | 258 (242-273) | 420 (398-765) | 383 (286-418) | 283 (251-410) | 438 (373-472) |
| .NET 10 AOT | native-nospin | 4 | 284 (266-296) | 590 (434-788) | 284 (251-299) | 352 (275-476) | 428 (367-439) |
| .NET 10 AOT | managed | 5 | 254 (248-264) | 390 (385-739) | 286 (282-356) | 205 (171-225) | 131 (112-147) |

What they say:

- **The spin costs CPU per call, 11 to 17 % ahead-of-time and 7 to 13 % just-in-time**: 210
  against 180 us on .NET 8 AOT, the ranges apart; 200 against 180 on .NET 10 AOT, 297 against 262
  on .NET 8 and 347 against 323 on .NET 10, the ranges overlapping. grpc-dotnet spends 172 and
  144 ahead-of-time.
- **What it does to P50 is within the spread**: ahead-of-time 258 against 274 us on .NET 8 and
  246 against 272 on .NET 10, grpc-dotnet at 254 and 251, the ranges overlapping; the same
  engine without the spin reads 261 (244-313) in the trim's table below, so 13 us is what two
  cells of one configuration differ by here. Just-in-time 270 against 255 and 250 against 286;
  busy, from 26 us lower to 18 us higher with the spin. No difference is reported.
- **The downloads and the uploads move within their ranges** with the spin, in both directions.
- **The one stage the spin clearly moves** is the one it aims at: in the instrumented calls
  below, the send to the driver's first poll is 4.5 us (4.0-4.8 over the passes) with it
  against 12.8 (12.3-15.2) without, the operating system's wake-up of the channel's thread. The
  stages after it overlap between the two, but for the write held for coalescing, 5.9 (5.0-6.4)
  against 6.8 (6.6-8.9); the whole call overlaps too. A host that wants the CPU
  rather than those microseconds states `"BeforeSleep": "None"`.
- **Just-in-time, grpc-dotnet is the slower transport in P50, CPU and the uploads** on both
  hosts, the ranges apart for CPU and the uploads.

### What TLS costs

The same campaign: `native-h2c` and `managed-h2c` speak HTTP/2 in the clear, against the test
server started without TLS.

| Host | Transport | n | P50 (us) | P99 (us) | CPU per call (us) |
|------|-----------|--:|---------:|---------:|------------------:|
| .NET 8 | native, TLS | 4 | 270 (252-288) | 447 (432-622) | 297 (286-341) |
| .NET 8 | native, h2c | 4 | 208 (200-235) | 393 (371-716) | 354 (314-380) |
| .NET 8 | managed, TLS | 4 | 310 (296-351) | 763 (497-1313) | 576 (531-691) |
| .NET 8 | managed, h2c | 4 | 272 (262-287) | 510 (433-1173) | 581 (473-594) |
| .NET 8 AOT | native, TLS | 4 | 258 (253-303) | 523 (419-1000) | 210 (198-241) |
| .NET 8 AOT | native, h2c | 5 | 235 (227-246) | 393 (357-742) | 200 (181-227) |
| .NET 8 AOT | managed, TLS | 4 | 254 (247-260) | 928 (644-1184) | 172 (145-183) |
| .NET 8 AOT | managed, h2c | 5 | 228 (212-231) | 542 (361-726) | 139 (105-145) |
| .NET 10 AOT | native, TLS | 4 | 246 (240-299) | 839 (400-1680) | 200 (180-330) |
| .NET 10 AOT | native, h2c | 4 | 236 (222-268) | 372 (366-937) | 200 (180-231) |
| .NET 10 AOT | managed, TLS | 5 | 251 (220-288) | 486 (349-1005) | 144 (122-173) |
| .NET 10 AOT | managed, h2c | 5 | 216 (207-225) | 427 (311-578) | 123 (102-148) |

The instrumented calls: the engine with QueryPerformanceCounter marks on both sides of every
stage of a unary call (not committed), .NET 8 ahead-of-time, idle, four passes interleaved from
12:07 to 12:27, calibration 7 530 to 7 934 us at the start of each run. The engine of
`4bf4a4912`, before the trim, in the first three columns, and that of `276a8c5a5` in the last,
the same configuration as the second but for the trim, which shows how far two cells of one
configuration differ. Each cell is the median over the passes of the per-call medians of the
10 000 measured calls, the range of the passes in brackets.

| Stage (us) | TLS, spin | TLS, no spin | h2c, spin | TLS, no spin, trimmed |
|---|---:|---:|---:|---:|
| send to the driver's first poll | 4.5 (4.0-4.8) | 12.8 (12.3-15.2) | 4.6 (4.4-6.0) | 13.4 (12.2-13.9) |
| driver's first poll to `send_request` | 7.4 (6.2-7.5) | 7.5 (7.1-9.5) | 6.8 (6.4-9.8) | 7.7 (6.4-7.9) |
| `send_request` to the first write | 10.7 (8.3-11.0) | 10.9 (10.0-14.8) | 9.3 (8.6-13.8) | 12.7 (10.1-13.4) |
| write held, coalescing | 5.9 (5.0-6.4) | 6.8 (6.6-8.9) | 5.5 (4.9-8.2) | 7.6 (6.3-8.0) |
| socket write | 31.9 (25.2-32.7) | 27.6 (25.0-37.4) | 27.0 (25.1-40.4) | 31.7 (25.2-33.5) |
| write to the first read | 153.1 (146.0-172.6) | 143.6 (140.2-185.9) | 131.6 (120.3-148.8) | 143.7 (133.2-177.8) |
| first read to the head | 15.2 (10.9-16.0) | 12.5 (12.0-17.8) | 11.8 (10.8-19.0) | 15.7 (11.5-16.5) |
| the whole call, to the callback's return | 270.6 (245.8-286.2) | 259.1 (249.9-339.2) | 235.9 (230.4-301.0) | 275.0 (240.6-307.4) |

- **The socket write is mostly the send, not TLS**: 27.0 us in the clear against 31.9 with
  TLS, both with the spin, the ranges overlapping; rustls's record is at most a few us of it.
  Two cells of one configuration differ by 4 us here (27.6 and 31.7).
- **TLS's cost to a call lies after the write rather than in it**: the whole instrumented call
  is 235.9 us in the clear against 270.6 with TLS, of which 21.5 between the write and the first
  read, the stage that holds the server, both sides' TLS on the answer and the loopback, and the
  ranges of every stage overlap. In the plain runs above, it is 10 to 23 us of P50
  ahead-of-time for the native engine and 26 to 35 for grpc-dotnet, the ranges apart on .NET 8
  for both and overlapping on .NET 10 for both; in the clear grpc-dotnet leans faster on both hosts (228 against
  235 us, 216 against 236), where with TLS the two are within 5 us of each other.

### The driver's trim

`276a8c5a5` writes the engine's headers into the request's own and gives tonic's encoder a
buffer of 0, which tonic allocated, 1 KiB, for every request. `native-nospin`, the engine before
it (`4bf4a4912`) and after, in the campaign above:

| Host | Engine | n | P50 (us) | P99 (us) | CPU per call (us) | Allocated per call (B) |
|------|--------|--:|---------:|---------:|------------------:|-----------------------:|
| .NET 8 | before | 4 | 277 (232-281) | 502 (440-1158) | 249 (223-288) | 4100 |
| .NET 8 | after | 4 | 255 (238-277) | 498 (461-1108) | 262 (217-269) | 4099 |
| .NET 8 AOT | before | 5 | 261 (244-313) | 1164 (561-1251) | 178 (159-197) | 4067 |
| .NET 8 AOT | after | 4 | 274 (252-302) | 526 (390-1066) | 180 (164-192) | 4066 |

- **The trim is not visible in a whole call**: every range overlaps the other, in both
  directions; the allocation the benchmark counts is managed, and the buffer was native.
- In the instrumented calls, the engine's headers and the header list's check take 0.6 us
  after it against 0.9 before (spin off, four passes each, 0.4-0.6 against 0.9-1.2), and the
  stretch from the driver's first poll to `send_request` 7.7 against 7.5 us: what is saved is
  below what the rest of the stretch varies by at this clock.

## h2-batch, what it costs

`decisions.md` named two costs of the build against h2-batch's patch, besides its writes, when
this was measured: a control frame, or a neighbouring call's frame, waits behind the DATA the
patched connection has queued; and above one frame per write, a reset call sends DATA after its
reset. Three engines here: stock h2, which takes one frame per write; the build against the patch
at one (`patched, 1`); and that build at 16 (`patched, 16`, `native-batch`). The engine is
that of `e24169ad3`, the benchmark that of `b38f87e34`, the Rust test `h2_batch_costs` of
`36364eacf`; the last subsection measures the engine once its queue is bounded.

One campaign on 2026-10-15, 16:48 to 17:08: five passes, the eight runs of a pass interleaved and
their order rotated from one pass to the next, on the performance cores at AboveNormal priority.
Each run started once no build, test or prover of another session ran, and a sample of the processes
every 3 s saw none during any run, but for the first, which began before the sampling; the System
log (Id 42, 107 and 1, and the Modern Standby events 506 and 507) shows no sleep, standby, wake or
clock change from 14:55 to the end. The processor was 9 to 71 % busy one second before the runs
kept, the desktop's applications open. Four benchmark runs whose first calibration is above 8 000
us, a processor slower than the campaign's, are left out: 12 982, 9 283, 9 554 and 8 127 us; the
calibrations of the runs kept read 7 528 to 7 756 us at their start. The Rust test has no
calibration; its runs are interleaved with the benchmark's.

### A neighbouring call

The benchmark's `pinned` and `shared` scenarios, .NET 8 just-in-time, against the test server
over TLS: 10 000 sequential `Say` calls after 1 000 to warm up, on a channel held to one
connection (`Http2.SimultaneousCallsPerConnection` `{"Limit": 16}`), alone on it, and beside a
streamed upload of 64 KiB chunks that starts a second before the measured calls and stops after
them. A run of `shared` with stock h2 showed one established connection in 12 samples over 24 s.
The last column is what the upload hands to the channel during the measured calls. The test
server's windows are 768 KiB per stream and 1 MiB for the connection. Each cell is the median of
the passes kept, the range in brackets.

| Engine | Upload beside | n | P50 (us) | P95 (us) | P99 (us) | Upload beside (MiB/s) |
|--------|---------------|--:|---------:|---------:|---------:|----------------------:|
| stock | none | 4 | 391 (374-609) | 1086 (871-1140) | 1409 (1206-1857) | |
| stock | 64 KiB chunks | 3 | 1327 (1314-1610) | 3057 (2883-3453) | 7361 (6368-11464) | 255 (233-280) |
| patched, 1 | none | 5 | 371 (352-418) | 761 (609-1637) | 1079 (828-1993) | |
| patched, 1 | 64 KiB chunks | 5 | 1376 (1257-1623) | 3034 (2939-3624) | 8296 (7414-12077) | 260 (229-270) |
| patched, 16 | none | 5 | 365 (349-437) | 682 (633-793) | 1084 (1002-1443) | |
| patched, 16 | 64 KiB chunks | 4 | 1126 (1083-1188) | 2510 (2401-2567) | 7738 (4652-9222) | 362 (346-372) |

### A PING's acknowledgement

The engine does not expose how long its own keepalive PINGs wait: hyper sends them and times
their acknowledgements, and reads neither out. `h2_batch_costs`'s `a_ping_behind_an_upload`
measures the other direction: a server that sends a PING, waits for its acknowledgement, 2 ms,
and the next, 500 times or for 20 s, over a call that sent one byte (idle) and over a streamed
upload of 64 KiB chunks for as long as the PINGs last. The wait runs from the PING's write to the
server's read of the acknowledgement, behind whatever the client sent before it; the server reads
every frame and gives back half a window at a time. Under two sets of windows: the benchmark's test
server's, 768 KiB a stream and 1 MiB for the connection, and the largest HTTP/2 allows (2^31 - 1
for each stream and the connection). The release build of the test, its threads on the
performance cores at AboveNormal priority. Each cell is the median of five passes, the range in
brackets; every PING of every pass was acknowledged.

| Windows | Engine | Upload | P50 (us) | P99 (us) | Most (us) | PINGs |
|---------|--------|--------|---------:|---------:|----------:|------:|
| 768 KiB | stock | none | 207 (184-265) | 419 (321-753) | 493 (351-1889) | 500 |
| 768 KiB | stock | 64 KiB chunks | 397 (350-471) | 1199 (1034-1723) | 6672 (2390-12255) | 500 |
| 768 KiB | patched, 1 | none | 238 (158-285) | 515 (436-775) | 742 (555-1576) | 500 |
| 768 KiB | patched, 1 | 64 KiB chunks | 553 (354-885) | 2217 (1540-3243) | 9236 (2454-20806) | 500 |
| 768 KiB | patched, 16 | none | 245 (196-264) | 523 (359-623) | 1012 (471-1687) | 500 |
| 768 KiB | patched, 16 | 64 KiB chunks | 388 (190-476) | 16426 (1224-19420) | 18845 (16166-29539) | 500 |
| largest | stock | none | 214 (165-221) | 378 (328-850) | 1211 (465-1464) | 500 |
| largest | stock | 64 KiB chunks | 1442 (1066-1657) | 4715 (3779-11990) | 15798 (12926-26698) | 500 |
| largest | patched, 1 | none | 203 (174-243) | 422 (396-440) | 967 (749-1488) | 500 |
| largest | patched, 1 | 64 KiB chunks | 1714 (1409-2149) | 4646 (3474-13351) | 20483 (3963-24670) | 500 |
| largest | patched, 16 | none | 196 (160-215) | 485 (384-552) | 1279 (870-1482) | 500 |
| largest | patched, 16 | 64 KiB chunks | 47691 (40121-55417) | 99188 (91515-104931) | 123787 (114607-128442) | 373 (324-443) |

### DATA after a cancel

`h2_batch_costs`'s `data_after_a_cancel`: a streamed upload of 64 KiB chunks to a server that, at
the first DATA frame, reads nothing more; once the client has written nothing for 300 ms, the call
is cancelled, and 200 ms later the server reads again, to the stream's RST_STREAM. The test hooks
count the bytes the client has written, so the server tells which DATA frames end past the offset
the client had written to when it cancelled, and their bytes past it. The server's windows are
those above and never updated. Ten trials a pass, five passes, 50 trials a cell: the median, the
range in brackets.

| Windows | Engine | DATA frames after the cancel | Their bytes | Offset at the cancel |
|---------|--------|-----------------------------:|------------:|---------------------:|
| 768 KiB | stock | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 |
| 768 KiB | patched, 1 | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 |
| 768 KiB | patched, 16 | 36 (0-49) | 491 826 (0-655 791) | 295 339 (131 374-787 165) |
| largest | stock | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 |
| largest | patched, 1 | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 |
| largest | patched, 16 | 2 560 (2 560-2 560) | 33 580 032 (33 580 032-33 580 032) | 459 304 (196 960-1 311 922) |

### What the unbounded queue costs

- **DATA after a reset is not negligible at 16 frames per write, and it goes past the bound
  documented for it.** `decisions.md` and the option's description then said a call reset while
  its part is being written can still send up to FramesPerWrite - 1 frames; measured, a cancelled
  upload sends up to 49 frames, 655 791 bytes, after its cancel under 768 KiB windows, and 2 560
  frames,
  33 580 032 bytes, in every trial under the largest, against one frame of 16 393 bytes for stock
  h2 and for the patched build at one frame per write, in every trial. Under 768 KiB windows,
  the offset at the cancel and the bytes after it add up to 787 165 in all 50 trials, the stream's
  window being 786 432 bytes of DATA: consistent with all that the window allowed and the socket
  had not taken going out after the cancel. Under the largest, the 2 560 frames are 512 whole
  messages, each its 65 541 bytes in four frames of 16 384 and one of 5 (512 x 5 frames, and
  512 x (65 541 + 5 x 9) = 33 580 032 bytes), the same in every trial while the offset at the
  cancel ranges from 196 960 to 1 311 922; the window, 2^31 - 1, is not what stops it, and what
  does is not measured here.
- **A neighbouring call is not slowed under 768 KiB windows.** Beside the upload, the patched
  build at 16 has the lowest P50, 1 126 us, its range clear below stock's (1 083-1 188 against
  1 314-1 610), and a P99 within the spread of the others; at one frame per write it is a tie with
  stock in every column. The upload beside it hands 362 MiB/s to the channel against 255 for stock.
  The upload, not the build, is what moves the call: it raises P50 3.1 to 3.7 times and P99 5.2
  to 7.7 times for every engine.
- **A PING waits longer behind an upload at 16, by up to tens of milliseconds under 768 KiB windows
  and around 50 under the largest.** Under 768 KiB windows, the median is a tie (388 us against 397
  for stock), but P99 is 16 426 (1 224-19 420) us against 1 199 (1 034-1 723), one pass of five
  overlapping stock's, and the longest wait 18 845 (16 166-29 539) against 6 672 (2 390-12 255),
  those ranges apart; the unary P99 of the benchmark, under the same windows, does not move with it.
  Under the largest, the median is 47 691 us against 1 442, P99 99 188 against 4 715, the longest
  123 787 against 15 798, every range apart; the 20 s ended before the 500 PINGs did. At one frame
  per write the patched build is within the spread of stock in every column. Against the keepalive's
  default timeout of 20 s, the longest wait measured, 128 ms, is 0.6 %. These are a server's PINGs,
  acknowledged by the client; the engine's own keepalive PINGs wait in the same queue on their way
  out, which is not measured. A neighbouring unary call's latency under the largest windows is not
  measured here, the benchmark's test server granting 768 KiB.
- So of the two costs, the second is not negligible: up to 655 791 bytes sent after a cancel under
  768 KiB windows and 33 580 032 under the largest, against 16 393. The first is negligible for a
  neighbouring call under 768 KiB windows, and a PING's wait stays far below the keepalive's
  timeout under both.

### With the queue bounded at 1 MiB

`f6cf86e10` bounds the patched build's queue: no more DATA is queued once 1 MiB of payload waits,
and one more part may follow. That engine is measured as above, with two additions: the Rust test
of `11bdb9c0c` also cancels uploads of 4 KiB chunks, shorter than a frame, which the patched build
queues whole at any frames per write; and the full benchmark runs idle, `native` on stock h2 and
`native-batch`, for what the patch gains.

Two campaigns on 2026-10-15, on the performance cores at AboveNormal priority, each run started once
no build, test or prover of another session ran, the 3 s sampling seeing none during any run; the
System log shows no sleep, standby, wake or clock change from 22:15 to their end. The first, 22:19
to 22:34, five passes of the benchmark's eight runs interleaved. The second, 22:36 to 22:51, five
passes of the Rust test of both builds interleaved, the patched build's test run from the build's
own directory. The processor was 13 to 51 % busy one second before the runs kept. Four benchmark
runs whose first calibration is outside 7 500 to 8 000 us are left out: 4 859, a faster processor,
and 8 528, 9 477 and 14 378; the runs kept read 7 526 to 7 571 us.

What the patch gains, idle, .NET 8 just-in-time:

| Engine | n | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) | P50 (us) | CPU per call (us) |
|--------|--:|-----------------------:|-------------------------:|----------------------:|------------------------:|---------:|------------------:|
| stock | 5 | 480 (263-491) | 3208 (2875-5771) | 357 (316-366) | 2458 (2167-2875) | 336 (330-353) | 325 (278-380) |
| patched, 16 | 3 | 605 (583-767) | 2479 (1917-2542) | 417 (401-452) | 1562 (1438-1667) | 333 (330-342) | 325 (308-406) |

A neighbouring call, the benchmark's `pinned` and `shared` scenarios:

| Engine | Upload beside | n | P50 (us) | P95 (us) | P99 (us) | Upload beside (MiB/s) |
|--------|---------------|--:|---------:|---------:|---------:|----------------------:|
| stock | none | 3 | 336 (335-629) | 871 (856-893) | 1136 (973-1253) | |
| stock | 64 KiB chunks | 5 | 1127 (978-1187) | 2851 (2268-3033) | 8006 (4386-10736) | 291 (281-358) |
| patched, 1 | none | 5 | 340 (336-343) | 610 (473-878) | 924 (724-1237) | |
| patched, 1 | 64 KiB chunks | 5 | 1077 (1039-1141) | 2625 (2545-2840) | 5702 (5358-10023) | 314 (288-315) |
| patched, 16 | none | 5 | 336 (329-343) | 485 (446-956) | 912 (723-1298) | |
| patched, 16 | 64 KiB chunks | 5 | 919 (840-1116) | 2306 (2248-2468) | 3901 (3679-4724) | 434 (380-448) |

A PING's acknowledgement:

| Windows | Engine | Upload | P50 (us) | P99 (us) | Most (us) | PINGs |
|---------|--------|--------|---------:|---------:|----------:|------:|
| 768 KiB | stock | none | 208 (157-221) | 387 (378-459) | 681 (466-1968) | 500 |
| 768 KiB | stock | 64 KiB chunks | 382 (300-556) | 4014 (1183-10513) | 12900 (2503-18686) | 500 |
| 768 KiB | patched, 1 | none | 153 (101-184) | 366 (300-383) | 490 (411-816) | 500 |
| 768 KiB | patched, 1 | 64 KiB chunks | 420 (315-449) | 1472 (910-10378) | 14509 (1400-17303) | 500 |
| 768 KiB | patched, 16 | none | 200 (140-253) | 373 (368-519) | 587 (527-703) | 500 |
| 768 KiB | patched, 16 | 64 KiB chunks | 306 (247-401) | 14968 (2343-18189) | 19369 (17095-30019) | 500 |
| largest | stock | none | 181 (173-208) | 416 (403-478) | 630 (457-722) | 500 |
| largest | stock | 64 KiB chunks | 1193 (629-1302) | 3116 (2831-4907) | 15746 (6911-18429) | 500 |
| largest | patched, 1 | none | 149 (98-220) | 381 (316-415) | 484 (412-559) | 500 |
| largest | patched, 1 | 64 KiB chunks | 1412 (1369-1654) | 2844 (2754-5087) | 13063 (3136-19584) | 500 |
| largest | patched, 16 | none | 184 (131-214) | 378 (334-389) | 542 (408-1393) | 500 |
| largest | patched, 16 | 64 KiB chunks | 2230 (1900-2825) | 31653 (14506-33173) | 38717 (29506-51949) | 500 |

DATA after a cancel, 50 trials a cell:

| Windows | Engine | Chunk | DATA frames after the cancel | Their bytes | Offset at the cancel |
|---------|--------|-------|-----------------------------:|------------:|---------------------:|
| 768 KiB | stock | 64 KiB | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 |
| 768 KiB | stock | 4 KiB | 1 (1-1) | 4 110 (4 110-4 110) | 74 182 (74 182-78 292) |
| 768 KiB | patched, 1 | 64 KiB | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 (82 181-114 967) |
| 768 KiB | patched, 1 | 4 KiB | 150 (114-176) | 613 485 (467 580-722 400) | 174 877 (65 962-320 782) |
| 768 KiB | patched, 16 | 64 KiB | 2 (0-44) | 32 758 (0-590 205) | 754 406 (196 960-787 165) |
| 768 KiB | patched, 16 | 4 KiB | 150 (117-175) | 617 595 (479 910-718 290) | 170 767 (70 072-308 452) |
| largest | stock | 64 KiB | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 |
| largest | stock | 4 KiB | 1 (1-1) | 4 110 (4 110-4 110) | 74 182 (74 182-78 292) |
| largest | patched, 1 | 64 KiB | 1 (1-1) | 16 393 (16 393-16 393) | 82 181 |
| largest | patched, 1 | 4 KiB | 256 (256-256) | 1 052 160 (1 052 160-1 052 160) | 146 107 (65 962-1 151 002) |
| largest | patched, 16 | 64 KiB | 80 (80-80) | 1 049 376 (1 049 376-1 049 376) | 328 132 (196 960-1 311 922) |
| largest | patched, 16 | 4 KiB | 256 (256-256) | 1 052 160 (1 052 160-1 052 160) | 154 327 (65 962-1 118 131) |

- **The gain remains, smaller than measured before**: the streamed upload is 1.26 times as fast
  (605 against 480 MiB/s) for 23 % less CPU per MiB, the unary upload 1.17 times (417 against 357)
  for 36 % less, every range clear of stock's; P50 and the CPU per call are a tie. "h2-batch, where
  it acts" measured 1.55 times for both, for 25 and 43 % less CPU per MiB, with stock h2 uploading
  at 219 MiB/s streamed where it uploads at 480 here; whether the bound or the machine accounts for
  the difference is answered in the next subsection, which runs both builds. The
  patched row keeps three passes, two left out for their calibration, and stock's streamed uploads
  read 263 and 288 MiB/s in two passes, 480 to 491 in the other three.
- **DATA after a cancel is bounded**: under the largest windows by the queue, at most 1 052 160
  bytes, 256 messages of 4 KiB at one and at 16 frames per write, or 16 of 64 KiB at 16 (1 049 376),
  in every trial, against 33 580 032 before the bound; under 768 KiB windows by the window, at most
  722 400.
  Stock h2 sends one frame, 4 110 or 16 393 bytes, and so does the patched build at one frame per
  write with 64 KiB chunks.
- **A neighbouring call is not slowed**: at 16 frames per write, P50 919 (840-1 116) us against
  1 127 (978-1 187) for stock and P99 3 901 (3 679-4 724) against 8 006 (4 386-10 736), the
  ranges overlapping, the upload beside it handing 434 MiB/s to the channel against 291; at one
  frame per write, a tie.
- **A PING waits longer behind an upload at 16 under the largest windows**: P50 2 230
  (1 900-2 825) us against 1 193 (629-1 302) for stock, P99 31 653 (14 506-33 173) against 3 116
  (2 831-4 907), the longest 38 717 (29 506-51 949) against 15 746 (6 911-18 429), every range
  apart; before the bound, 47 691, 99 188 and 123 787. One MiB at the 434 MiB/s the upload reached
  beside a call takes 2.3 ms; what holds the P99 and the longest waits past that is not measured.
  Under 768 KiB windows, where the window rather than the bound limits the queue, the cost measured
  before the bound remains: P99 14 968 (2 343-18 189) against 4 014 (1 183-10 513) for stock and
  the longest wait 19 369 (17 095-30 019) against 12 900 (2 503-18 686), each range overlapping
  stock's only through stock's own spread, wider than in the earlier campaign; the median is a tie
  (306 against 382). At one frame per write under the largest windows, the
  median is 1 412 (1 369-1 654) against 1 193, the ranges 67 us apart, and the rest overlaps. The
  longest wait measured, 52 ms, is 0.26 % of the keepalive's default timeout of 20 s.

### Throughput, without and with the bound

The full benchmark (see "What is measured"), idle, .NET 8 just-in-time, for three engines side by
side: stock h2 (`native`), and the build against h2-batch's patch at 16 frames per write
(`native-batch`), with the patch as it was before `f6cf86e10`, unbounded, and as `f6cf86e10` has
it, bounded at 1 MiB. One campaign on 2026-10-16, 00:23 to 00:27: five passes of the three runs
interleaved, their order rotated, on the performance cores at AboveNormal priority, the 3 s
sampling seeing no build during any run; the System log shows no sleep, standby, wake or clock
change from 00:10 to the end. The processor was 3 to 36 % busy one second before the runs kept.
One run whose first calibration is outside 7 500 to 8 000 us is left out, the bounded build's of
pass 2 at 14 920 us; the runs kept read 7 528 to 7 544 us.

| Engine | n | Download (MiB/s) | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) |
|--------|--:|-----------------:|-----------------------:|-------------------------:|----------------------:|------------------------:|
| stock | 5 | 325 (213-350) | 271 (233-334) | 5062 (3875-5583) | 226 (214-231) | 3792 (3292-3917) |
| patched, 16, unbounded | 5 | 252 (229-292) | 339 (308-408) | 4083 (3312-4229) | 307 (276-338) | 2188 (1917-2312) |
| patched, 16, bounded | 4 | 286 (229-341) | 386 (343-422) | 3688 (3167-4104) | 304 (302-307) | 2208 (2062-2396) |

- **The bound costs nothing measurable**: every range of the bounded build overlaps the unbounded
  one's, the streamed upload leaning higher (386 against 339 MiB/s) and the unary one a tie (304
  against 307).
- **Against stock h2, both builds upload a whole message 1.35 and 1.36 times as fast** (304 and 307
  against 226 MiB/s), for 42 % less CPU per MiB, the ranges clear. The streamed upload is 1.42
  times as fast bounded, its range clear of stock's, and 1.25 times unbounded, the ranges
  overlapping; its CPU per MiB is 27 and 19 % lower, the ranges overlapping.
- **The download is a tie**: the three ranges overlap.
- **The bound does not account for the smaller gain of the campaign before**: unbounded, the
  build here uploads 1.25 times as fast as stock streamed and 1.36 times unary, also below the
  1.55 of "h2-batch, where it acts". What does is not measured.
- Stock's streamed upload reads 271 MiB/s here and 480 in the campaign before, the same build, and
  the bounded build's 386 and 605; the ratios differ too, 1.42 against 1.26 streamed and 1.35
  against 1.17 unary. The cause is not measured.

## The machine

Windows 11 Pro 10.0.26100, 13th Gen Intel Core i7-1360P, 16 GB, with the desktop's usual
applications open and no other build or test running. The i7-1360P is a hybrid processor: four
performance cores, logical processors 0 to 7, and eight efficiency cores. A unary call takes about
twice as long on the second kind, and a run the scheduler spreads over both measures where its
threads landed, so every run is held to logical processors 0 to 7, the test server with it, at
AboveNormal priority.

Built with .NET SDK 11.0.100-rc.1; run on .NET 8.0.19, .NET 10.0.12 and .NET Framework 4.8,
ahead-of-time with the 8.0.31 and 10.0.12 ILCompiler packages; rustc 1.94.1, the engine built
with the release profile. The client and the server share the machine. The runs of `bf86fece3`,
and those of 2026-10-15, used the same SDK, runtimes, ILCompiler packages and compiler.

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
benchmarks do not carry that build, so it does not run there. `native-nospin` runs the engine
with `BeforeSleep` `"None"`, a channel's thread sleeping as soon as it runs out of work.
`native-h2c` and `managed-h2c` speak HTTP/2 in the clear, prior knowledge, to the test server
started without TLS, which tells what TLS costs from the rest; `managed-h2c` does not run on
.NET Framework, whose `WinHttpHandler` speaks HTTP/2 only over TLS. The h2-batch run builds the engine
over the one the other transports use, so a run of both builds takes a copy of the output
directory before the second build. A run that fails is reported and left out, and a cell left
with fewer passes says so. `pinned` and `shared`, in `-Scenarios`, measure the unary latency alone,
on a native channel held to one connection, without and with a streamed upload beside it; the
managed transports skip them.

The DATA after a cancel and the PINGs' waits come from the test `h2_batch_costs`, run in the
stock build and in the build against h2-batch's patch:

```bash
cargo test -p armonik-transport --features test-hooks --release --test h2_batch_costs -- --include-ignored --nocapture --test-threads 1
```

```bash
packages/rust/patches/h2-batch/build.sh test -p armonik-transport --features test-hooks --release --test h2_batch_costs -- --include-ignored --nocapture --test-threads 1
```

A single run is the benchmark itself, with the transport and `busy` for the busy scenario:

```bash
bin/Release/net8.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native busy
```

It prints one line: the latencies, the download and the two uploads, the allocation per call and
on .NET the thread pool work items, and for an idle run the CPU per call and per MiB uploaded and
the memory before and after the uploads. `ARMONIK_BENCH_AFFINITY` and `ARMONIK_BENCH_PRIORITY` set
where it and its server run. The benchmark starts its test server, a `net8.0` program, with the
`dotnet` the system finds first, which has to be an install with a .NET 8 runtime. Every run's
line ends with the two calibrations.
