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

The test server's `Say` answers in one of two ways. The plain `Say` answers with the reply and one
head entry, `x-answered`; the benchmark starts its server with `--plain-say`, which gives it this
one. The other `Say` adds what the compression tests read: three head entries (`x-saw-encoding`,
`x-saw-accept-encoding`, `x-saw-content-length`) and a trailer (`x-sent-encoding`), which both
transports decode on every call: about 450 B more allocation per call for the native engine, 360 B
for grpc-dotnet. The figures per call - latency, CPU and allocation per call - of "Results", "T6.13
and h2-batch" and "A whole request's write, unheld" use the plain `Say`; those of "After
integration", "What the metrics build costs", "h2-batch, where it acts", "The channel thread's
spin, TLS's share, and the driver's trim" and "h2-batch, what it costs" use the other. The
downloads and uploads call no `Say`.

Every transport option is left at its default, the runtime's ceilings included, but where a
transport or a scenario states one: `native-batch` sets `Http2.Send.FramesPerWrite` to 16, and
`pinned` and `shared` set `Http2.SimultaneousCallsPerConnection` to `{"Limit": 16}`. The native
transports take the build's frames per write: 16 since the .NET binding builds its engine against
h2-batch's patch by default, and 1 with `-p:NativeEngineH2Batch=false`. In every campaign before
that they ran at 1, and the engine was stock unless a section says otherwise; on the default build
`native` and `native-batch` now agree, and a comparison of the two builds takes both. `native` is
the engine as a host gets it, a channel's thread sleeping as soon as it runs out of work
(`BeforeSleep` `"None"`), and `native-spin` the same engine with `{"Spin": {}}`. `native` ran
`"None"` too in every campaign whose engine predates the option, those of `bf86fece3` and
`21a0b7807` among them.
In the campaigns of 2026-10-15, and in that of 2026-10-16 in "h2-batch, what it costs", whose
engines had the spin as their default, the native transports ran the spin, `native-batch` among
them, and `native-nospin`, a name the benchmark no longer takes, ran `"None"`.

The full measurement runs in two scenarios; `pinned` and `shared` measure the unary latency
alone ("h2-batch, what it costs"). **Idle**: the benchmark is all the process does, so the
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

`native` is the engine with what was then its default `BeforeSleep`, a spin of 100 us after each
callback into the host, which is `native-spin` today; `native-nospin` the same engine with
`"None"`, which is `native` today; `managed` grpc-dotnet.
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
  against 6.8 (6.6-8.9); the whole call overlaps too.
- **Just-in-time, grpc-dotnet is the slower transport in P50, CPU and the uploads** on both
  hosts, the ranges apart for CPU and the uploads.

The spin's CPU, for a P50 within the spread, is why the default is `"None"` (`decisions.md`); a
host that wants the microseconds of that stage rather than the CPU states
`"BeforeSleep": {"Spin": {}}`.

### What TLS costs

The same campaign: `native-h2c` and `managed-h2c` speak HTTP/2 in the clear, against the test
server started without TLS. Both native rows ran the spin, the default then.

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
`36364eacf`; the last subsections measure the engine once its queue is bounded. Every engine of
this section predates `fe3b6e09f`, so `BeforeSleep` defaulted to the spin and every native row ran
it, `native` and `native-batch` among them; run again, those transports do not.

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

`h2_batch_costs`'s cancels, as its ignored `data_after_ten_cancels` runs them: a streamed upload
of 64 KiB chunks to a server that, at the first DATA frame, reads nothing more; once the client
has written nothing for 300 ms, the call is cancelled, and 200 ms later the server reads again, to
the stream's RST_STREAM. The test hooks count the bytes the client has written, so the server
tells which DATA frames end past the offset the client had written to when it cancelled, and their
bytes past it. The server's windows are those above and never updated. Ten trials a pass, five
passes, 50 trials a cell: the median, the range in brackets.

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
  it acts", which ran without the spin, measured 1.55 times for both, for 25 and 43 % less CPU per
  MiB, with stock h2 uploading at 219 MiB/s streamed where it uploads at 480 here; whether the
  bound or the machine accounts for the difference is answered in the next subsection, which runs
  both builds. The patched row keeps three passes, two left out for their calibration, and stock's
  streamed uploads read 263 and 288 MiB/s in two passes, 480 to 491 in the other three.
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
  1.55 of "h2-batch, where it acts", which ran without the spin. What does is not measured.
- Stock's streamed upload reads 271 MiB/s here and 480 in the campaign before, the same build, and
  the bounded build's 386 and 605; the ratios differ too, 1.42 against 1.26 streamed and 1.35
  against 1.17 unary. The cause is not measured.

## A whole request's write, unheld

`6fc0f9702` sends at once a write that ends every request it carries, where the coalescing held it a
round more (`decisions.md`): in the engine's tests a unary call is held one round instead of two.
The engine does not keep it ("Concurrent calls" below). One campaign of 2026-10-15, 16:47 to 18:08,
entirely after the machine's last wake (11:59:17; the System log, Id 42, 107 and 1, and Kernel-Power
Id 105, 506 and 507, show no sleep, standby, power source or clock change from then to its end), on
the performance cores (logical processors 0 to 7, `-Affinity 0xFF`) at AboveNormal priority, five
passes interleaved, the order rotated each pass. Each run started once no build, test or prover ran,
other sessions' included, which held the campaign from about 17:13 to 17:34 but for one run at
17:28, and once the processor was under 30 % busy (three of those readings round to 30 in the log).
Every call is the plain `Say`. Three variants, from the benchmark of `b1823d7e8`: `native`, the
engine of `6fc0f9702`; `native, hold kept`, the same tree with the engine's `coalesce.rs` of
`e24169ad3`, which holds such a write as before; `managed`, grpc-dotnet. Both engines predate
`fe3b6e09f`, so `BeforeSleep` defaulted to the spin and every native row ran it. The runs whose first
calibration (`calib_us`, see "What is measured") is above 8 000 us, 22 of 130 (8 065 to 13 415 us),
ran on a slower processor and are left out; each cell says how many passes it keeps (n). The
calibrations of the runs kept read 7 526 to 7 987 us. Each cell is the median of the passes kept,
the range in brackets; the allocation, whose range within a cell is at most about 50 B, is the
median alone.

Idle:

| Host | Transport | n | P50 (us) | P99 (us) | CPU per call (us) | Allocated per call (B) |
|------|-----------|--:|---------:|---------:|------------------:|-----------------------:|
| .NET 8 | native | 4 | 282 (263-322) | 852 (618-1142) | 358 (303-417) | 3647 |
| .NET 8 | native, hold kept | 4 | 266 (254-331) | 818 (607-977) | 318 (280-406) | 3646 |
| .NET 8 | managed | 4 | 357 (289-475) | 951 (679-1292) | 662 (536-878) | 8008 |
| .NET 10 | native | 4 | 258 (244-295) | 850 (535-1126) | 342 (327-373) | 3648 |
| .NET 10 | native, hold kept | 4 | 300 (237-345) | 940 (618-1134) | 392 (309-483) | 3648 |
| .NET 10 | managed | 3 | 374 (267-388) | 899 (892-1234) | 741 (622-778) | 7950 |
| .NET 8 AOT | native | 4 | 278 (258-307) | 556 (496-1106) | 210 (202-245) | 3614 |
| .NET 8 AOT | native, hold kept | 4 | 298 (284-306) | 727 (548-1425) | 236 (217-242) | 3612 |
| .NET 8 AOT | managed | 3 | 285 (262-296) | 688 (644-839) | 177 (134-180) | 7840 |
| .NET 10 AOT | native | 4 | 294 (278-398) | 938 (676-1310) | 238 (186-369) | 3612 |
| .NET 10 AOT | native, hold kept | 4 | 291 (270-320) | 834 (691-1100) | 242 (219-255) | 3614 |
| .NET 10 AOT | managed | 5 | 264 (244-275) | 711 (502-1063) | 162 (148-192) | 7837 |

Busy:

| Host | Transport | n | P50 (us) | P99 (us) |
|------|-----------|--:|---------:|---------:|
| .NET 8 | native | 5 | 338 (300-483) | 625 (540-1364) |
| .NET 8 | native, hold kept | 5 | 386 (302-475) | 716 (525-1482) |
| .NET 8 | managed | 3 | 398 (310-430) | 671 (581-823) |
| .NET 10 | native | 5 | 312 (269-407) | 545 (515-853) |
| .NET 10 | native, hold kept | 5 | 342 (311-416) | 704 (490-1174) |
| .NET 10 | managed | 5 | 415 (371-446) | 675 (596-771) |
| .NET 8 AOT | native | 4 | 261 (257-266) | 543 (434-601) |
| .NET 8 AOT | native, hold kept | 4 | 264 (257-270) | 476 (405-655) |
| .NET 8 AOT | managed | 4 | 294 (267-311) | 542 (480-628) |
| .NET 10 AOT | native | 5 | 286 (248-315) | 647 (561-1151) |
| .NET 10 AOT | native, hold kept | 4 | 282 (278-337) | 1005 (543-1268) |
| .NET 10 AOT | managed | 4 | 310 (264-346) | 679 (486-841) |

What they say:

- **The unheld write is not visible in a whole call**: in every cell, idle and busy, its range
  overlaps the hold's, in P50, P99 and CPU per call, and the medians move both ways. Set side by
  side within each pass, the 31 pairs where both runs are kept put the unheld write's P50 a median
  5 us lower, 17 lower and 14 higher, and its CPU per call a median 5 us lower over 14 idle pairs,
  7 each way: a lean of the size of the held round in the instrumented calls of "The channel
  thread's spin, TLS's share, and the driver's trim" (5.5 to 7.6 us), which five passes whose
  cells spread by up to 190 us at P50 do not resolve. The 40 pairs with no run left out give the
  same, 4 us lower at P50. The allocation does not move, nor, idle, the thread pool's work items
  per call (2.04 to 2.08).
- **Ahead-of-time, against grpc-dotnet**: idle, P50 is a tie on .NET 8 (278 against 285 us) and
  grpc-dotnet is lower on .NET 10 (264 against 294, the ranges 3 us apart); grpc-dotnet spends
  less CPU per call, 177 against 210 us on .NET 8, the ranges apart, and 162 against 238 on .NET
  10, overlapping. Busy, the native engine is lower on .NET 8 (261 against 294, the ranges 1 us
  apart) and the two overlap on .NET 10. The native rows ran the spin, which "The spin" measures
  at 11 to 17 % more CPU per call ahead-of-time: at today's default the native CPU per call is not
  measured, and the gap to grpc-dotnet may be smaller.
- **Just-in-time, the native engine spends half the CPU per call** (358 against 662 us on .NET 8,
  342 against 741 on .NET 10, the ranges apart, with the spin); its P50 is lower on both, idle and
  busy, the ranges overlapping.
- **The plain `Say`** allocates 3 647 B per call for the native engine on .NET 8, against 4 099
  with the other in "The driver's trim", 452 B less; grpc-dotnet 8 008 against 8 368 in "After
  integration", 360 B less.

### Against the baseline's engine

The baseline's engine and benchmark (`1eca48506`), the same campaign, .NET 8 just-in-time, idle,
interleaved with the variants above. Its own test server answers the plain `Say`, the only one it
has. Its benchmark prints no calibration: the calibration of today's is added to it for these runs,
and is not committed. The first two rows are the first table's ("Results"), measured without a
calibration and not reproduced here.

| Build | Transport | n | P50 (us) | P99 (us) | CPU per call (us) | Allocated per call (B) |
|-------|-----------|--:|---------:|---------:|------------------:|-----------------------:|
| `1eca48506`, first table | native | 3 | 404 | 1374 | 394 | 3627 |
| `1eca48506`, first table | managed | 3 | 386 | 1305 | 691 | 7981 |
| `1eca48506` | native | 4 | 318 (291-334) | 1038 (826-1254) | 326 (286-362) | 3630 |
| `1eca48506` | managed | 4 | 330 (301-447) | 1030 (763-1217) | 604 (552-847) | 8004 |
| `b1823d7e8` | native | 4 | 282 (263-322) | 852 (618-1142) | 358 (303-417) | 3647 |
| `b1823d7e8` | managed | 4 | 357 (289-475) | 951 (679-1292) | 662 (536-878) | 8008 |

- **Today's engine and the baseline's are within the spread of each other**: P50 282 against 318
  us, P99 852 against 1038, CPU per call 358 against 326, every range overlapping. Today's engine
  ran the spin and the baseline's predates it; "The spin" puts the spin at 7 to 13 % more CPU per
  call just-in-time, which these runs do not separate. It allocates 17 B more per call, which
  these runs do not attribute to a commit.
- **The baseline's engine does not reproduce the first table's native figures today**: 318 us at
  P50 and 326 of CPU per call against 404 and 394 there, both outside today's ranges. grpc-dotnet's
  386 and 691 lie inside its ranges here (301-447 and 552-847). Today's figures compare with that
  table through these rows, not directly.

### Concurrent calls

The engine does not keep the unheld write: the hold also gathers the frames of other calls in
flight, which the unheld write gives up. Two measurements of 2026-10-15 show it, on engines that
predate `fe3b6e09f` and so ran the spin.

The writes, counted with the transport's test hooks in a release build of the engine's FFI tests:
bursts of N one-request unary calls on one channel, started and sent from one host thread, or from
N threads at once; 30 bursts, three repetitions, the range of the repetitions. Both rules run from
one build, through an environment switch, not committed, that restores the hold.

| Calls in flight | Hold, one thread | Unheld, one thread | Hold, N threads | Unheld, N threads |
|----------------:|-----------------:|-------------------:|----------------:|------------------:|
| 1 | 1.00 | 1.00 | 1.00 | 1.00 |
| 2 | 0.50 | 0.88-0.95 | 0.53-0.55 | 0.85-0.97 |
| 8 | 0.13 | 0.86-0.89 | 0.16-0.18 | 0.84-0.88 |
| 64 | 0.04-0.05 | 0.83-0.84 | 0.08-0.12 | 0.83-0.84 |

Writes per request. With the hold, a burst goes out in a few writes; unheld, in about one per call,
since each completed request goes before the next calls' frames join it.

What that costs, from 22:29 to 22:59: the benchmark with a mode, not committed, in which N workers
call the plain `Say` at once, 40 000 calls after 2 000 of warm-up; `native, unheld` the engine of
`6fc0f9702`, `native, hold` the same build with the switch, `managed` grpc-dotnet. .NET 8
just-in-time, .NET 8 and .NET 10 ahead-of-time; N of 1, 8 and 64; five passes interleaved, the
order rotated each pass, on the performance cores (`-Affinity 0xFF`) at AboveNormal priority, each
run started on mains power once no build, test or prover ran and the processor was under 30 % busy.
The CPU per call is the client process's, every thread's. A first run of this campaign, from 20:31
to 22:27, is left out whole: the machine went into Modern Standby (System log, Kernel-Power Id 506
and 507) from 20:56 to 21:47, during its fourth pass, which Id 42 and 107 do not
record. The System log shows no standby, power source change, sleep or clock change during the
second. The runs whose first calibration is above 8 000 us, 32 of 135, are left out. Each cell is
the median of the passes kept, the range in brackets.

| Host | Calls in flight | Transport | n | Calls a second | CPU per call (us) | P50 (us) |
|------|----------------:|-----------|--:|---------------:|------------------:|---------:|
| .NET 8 | 1 | native, unheld | 4 | 4542 (3951-5161) | 303 (279-341) | 192 (181-206) |
| .NET 8 | 1 | native, hold | 4 | 3980 (3737-4951) | 313 (267-349) | 212 (187-213) |
| .NET 8 | 1 | managed | 5 | 3952 (3520-4420) | 449 (414-525) | 216 (181-222) |
| .NET 8 | 8 | native, unheld | 2 | 13951 (12977-14925) | 198 (175-222) | 520 (488-551) |
| .NET 8 | 8 | native, hold | 5 | 15140 (14595-15280) | 157 (153-163) | 515 (509-528) |
| .NET 8 | 8 | managed | 4 | 15772 (14126-16165) | 258 (248-274) | 496 (488-526) |
| .NET 8 | 64 | native, unheld | 5 | 15600 (14560-15812) | 166 (159-182) | 4053 (4014-4196) |
| .NET 8 | 64 | native, hold | 2 | 24980 (23320-26639) | 60 (59-61) | 2518 (2361-2674) |
| .NET 8 | 64 | managed | 4 | 41677 (39775-42668) | 104 (94-106) | 1391 (1354-1420) |
| .NET 8 AOT | 1 | native, unheld | 3 | 4479 (3188-4847) | 206 (197-250) | 193 (183-280) |
| .NET 8 AOT | 1 | native, hold | 5 | 3902 (3374-4519) | 220 (196-241) | 210 (199-236) |
| .NET 8 AOT | 1 | managed | 5 | 4246 (3557-4624) | 153 (146-177) | 193 (185-228) |
| .NET 8 AOT | 8 | native, unheld | 3 | 15040 (9730-16775) | 96 (90-138) | 494 (455-753) |
| .NET 8 AOT | 8 | native, hold | 2 | 16836 (16547-17124) | 70 (70-71) | 470 (457-483) |
| .NET 8 AOT | 8 | managed | 4 | 19724 (16436-20000) | 98 (67-108) | 394 (386-430) |
| .NET 8 AOT | 64 | native, unheld | 5 | 16161 (12282-16442) | 96 (86-113) | 4018 (3924-4433) |
| .NET 8 AOT | 64 | native, hold | 3 | 24166 (22786-27218) | 54 (52-56) | 2510 (2346-2567) |
| .NET 8 AOT | 64 | managed | 5 | 53385 (51023-53889) | 44 (38-59) | 1080 (1075-1134) |
| .NET 10 AOT | 1 | native, unheld | 4 | 4515 (3247-4777) | 201 (180-270) | 196 (189-238) |
| .NET 10 AOT | 1 | native, hold | 3 | 4535 (3137-4574) | 210 (196-263) | 194 (193-257) |
| .NET 10 AOT | 1 | managed | 5 | 4531 (4158-5108) | 140 (132-173) | 186 (178-198) |
| .NET 10 AOT | 8 | native, unheld | 3 | 16114 (16063-16820) | 97 (93-98) | 477 (456-479) |
| .NET 10 AOT | 8 | native, hold | 5 | 17138 (15511-17274) | 73 (59-82) | 464 (464-516) |
| .NET 10 AOT | 8 | managed | 2 | 20288 (20232-20343) | 70 (68-72) | 384 (384-384) |
| .NET 10 AOT | 64 | native, unheld | 4 | 15150 (13936-16603) | 99 (89-108) | 4178 (3863-4205) |
| .NET 10 AOT | 64 | native, hold | 3 | 26168 (24564-26316) | 49 (46-49) | 2433 (2419-2505) |
| .NET 10 AOT | 64 | managed | 4 | 57747 (51987-60366) | 37 (31-41) | 1002 (945-1117) |

- **At 64 calls in flight, the unheld write makes 33 to 42 % fewer calls a second for 1.8 to 2.8
  times the CPU per call**, and its P50 is 1.6 to 1.7 times as high, on every host, every range
  clear of the hold's.
- **At 8, it spends 26 to 37 % more CPU per call**, the ranges clear on every host; the calls a
  second and P50 overlap.
- **One call at a time, every range overlaps**, as in the campaign above.
- **At 64 calls in flight, grpc-dotnet makes 1.7 to 2.2 times as many calls a second as the native
  engine with the hold**, for less CPU per call ahead-of-time and more just-in-time; these runs do
  not say why.

## The processor levels, on LTO engines

Are builds of the engine for higher x86 levels worth shipping? A first campaign, on 2026-10-16,
measured the default workloads on engines without fat LTO, and found no level moving a figure
beyond the spread on more than one host; a second, on 2026-10-17, on fat-LTO engines, looks
where the engine's own code may be a larger share: compression, many calls in flight, large
messages, and HTTP/2 in the clear.

### The default workloads, without fat LTO

Five passes on 2026-10-16 from 22:34 to 23:34, on `1251a2907`, with the engine built in Cargo's
default release profile against h2-batch's patch, and again with `-C target-cpu=x86-64-v2`,
`x86-64-v3` and `native`; for i686, the default and the same two levels. The full measurement of
"What is measured", idle, over TLS, against the server built here with the plain `Say`; the runs
interleaved, the order rotated by seven each pass, on the performance cores at AboveNormal
priority, each started on AC power with no build or test running, and no prover seen at the end
of any run; the System log shows no Kernel-Power 42, 105, 506 or 507 and no Power-Troubleshooter
1 from 22:00. Most first calibrations, and every last one, ran about 20 % slower than the 7 500
to 8 000 us the sections above keep: 31 of the 140 x64 runs started under 8 000 us. Every run is
kept, since filtering on the calibration leaves too few pairs, and each level's run is paired
with the default build's of the same pass; the change in %, the median of the five pairs, their
range in brackets:

| Host | Engine | P50 | CPU per call | Download | Upload, stream | Upload, unary |
|---|---|---:|---:|---:|---:|---:|
| .NET 8 | v2 | +1.5 (-20.9 to +24.1) | -9.3 (-30.2 to +27.0) | -17.2 (-25.0 to +10.5) | -3.3 (-15.2 to -2.2) | -1.1 (-15.8 to +4.1) |
| .NET 8 | v3 | -2.1 (-22.8 to +16.9) | -9.3 (-28.4 to +29.0) | +2.9 (-33.5 to +14.6) | +4.2 (-4.3 to +17.0) | -2.9 (-9.8 to +35.5) |
| .NET 8 | native | -6.3 (-24.6 to +14.0) | -15.9 (-26.8 to +22.1) | +7.6 (-28.3 to +39.4) | +7.1 (-13.2 to +32.8) | +6.3 (-8.8 to +27.8) |
| .NET 8 AOT | v2 | +2.1 (-11.9 to +7.7) | +1.2 (-13.5 to +6.7) | +1.1 (-8.0 to +26.4) | -3.0 (-15.5 to +8.6) | -5.1 (-12.3 to +24.7) |
| .NET 8 AOT | v3 | -0.3 (-12.9 to +15.1) | -7.3 (-27.1 to +41.8) | +3.1 (-7.2 to +45.5) | -8.0 (-35.6 to -2.4) | -1.9 (-9.6 to +5.6) |
| .NET 8 AOT | native | +3.9 (-13.8 to +15.1) | +2.1 (-24.9 to +29.3) | +3.4 (-18.9 to +23.1) | -14.4 (-22.9 to +17.4) | +14.2 (+3.5 to +38.6) |
| .NET Framework x86 | i686 v2 | -2.3 (-11.1 to +1.9) | -2.2 (-16.6 to +5.9) | -3.6 (-17.4 to +2.3) | -1.9 (-7.4 to +3.8) | +2.5 (-17.2 to +27.4) |
| .NET Framework x86 | i686 v3 | -4.3 (-9.6 to +1.1) | -0.6 (-13.0 to +4.0) | +6.5 (-21.3 to +14.8) | -10.1 (-21.0 to +6.8) | +11.2 (-16.2 to +30.9) |

Every range holds zero but three: two against a level, the upload as a stream at v2 on .NET 8
(-3.3 %) and at v3 ahead-of-time (-8.0 %), and one for it, `native`'s unary upload
ahead-of-time (+14.2 %); none repeats on another host with a range clear of zero. The absolute
figures, at that night's slower calibrations, do not compare with the sections above, and are
left out.

### Compression, calls in flight, large messages, and in the clear

The engines are all built from `fcfbcfd64` with the workspace's release profile, fat LTO in one
codegen unit, and the patched h2 the binding builds by default: for x64, `x86-64` (v1, the
default), `-C target-cpu=x86-64-v2`, `x86-64-v3` and `native` (the i7-1360P, without AVX-512);
for i686, the default (`pentium4`) and `-C target-cpu=x86-64-v3`; and the four x64 levels again
with the `metrics` feature. The x64 engines run under .NET 8, just-in-time and ahead-of-time, the
i686 ones under .NET Framework 4.8 in an x86 process, the `metrics` ones under .NET 8
just-in-time, all with the native transport; one row per workload runs grpc-dotnet on .NET 8
just-in-time as the reference.

The workloads are the benchmark's `gzip`, `zstd`, `concurrent` and `large` scenarios and its
`-h2c` transports. The server is the test server rebuilt for .NET 11 RC1, which the repository
does not build: grpc-dotnet has no zstd provider, and .NET 11's `ZstandardStream` makes one. To
build it again, copy `ArmoniK.Api.Client.RustGrpcChannel.TestServer` out of the tree, set its
`TargetFramework` to `net11.0`, and give `AddGrpc` three `CompressionProviders`: grpc-dotnet's
`GzipCompressionProvider` and `DeflateCompressionProvider` at `CompressionLevel.Fastest`, and one
named `zstd` whose streams are `new ZstandardStream(stream, level ?? CompressionLevel.Fastest,
true)` and `new ZstandardStream(stream, CompressionMode.Decompress)`. Build it with an SDK 11, and
run the benchmark with `ARMONIK_BENCH_SERVER` naming its `.dll` and `ARMONIK_BENCH_SERVER_HOST`
that SDK's `dotnet`. Every run of the campaign, gzip and the rest included, used that server,
its `Say` the plain one. The figures do not compare with the sections above, which ran the
server built here.

- **gzip** and **zstd**: `Grpc.Send.Compression` and `Grpc.Receive.Compression` set to the
  encoding; 64 MiB uploaded as 64 KiB chunks and as one message, and 64 MiB downloaded as 64 KiB
  chunks the server compresses (grpc-dotnet at `CompressionLevel.Fastest`), five runs of each, the
  median in MiB/s and the process's CPU per MiB over the five. The data is text: words drawn from
  a fixed vocabulary of 2048, which zlib shrinks 2.3 to 2.5 times. The server answers each run with
  the encoding it saw or sent, and every run of the campaign announced its encoding. The engine
  compresses gzip at flate2's default level, 6, and zstd at level 3; grpc-dotnet's client
  compresses gzip only, at its default level, which the benchmark does not set.
- **concurrent**: unary calls of the plain `Say` with 16 and then 64 in flight on one channel,
  each worker starting its next call when its last ends; 40 000 calls after 2 000 of warm-up;
  calls a second, the process's CPU per call, and P50.
- **large**: 512 MiB sent as unary messages of 4 MiB and of 16 MiB, one call at a time, and, on
  the native transport only, received as server-streaming calls of one such message; MiB/s and
  CPU per MiB. grpc-dotnet's channel refuses a message past 4 MiB, so its row only sends.
- **h2c**: the full measurement of "What is measured", and the concurrent one, in the clear.

What a level can reach differs per workload. gzip is flate2 over miniz_oxide, Rust, which the
level recompiles. zstd is zstd-sys, C compiled by cl.exe on Windows: neither LTO nor
`-C target-cpu` reaches it, and its runtime BMI2 dispatch is compiled for gcc and clang only, so a
level changes only the Rust around zstd there. TLS's AES-GCM and the C runtime's `memcpy` choose
their AVX paths at run time at every level.

Five passes on 2026-10-17 from 15:21 to 16:57, 73 runs each, interleaved, the order rotated by
seven each pass, on the performance cores (`0xFF`) at AboveNormal priority, each run started on AC
power once no cargo, rustc, MSBuild, compiler server, test host or prover was running. No run
failed, and none found such a process running once it ended. The System log shows no Kernel-Power
42, 105, 506 or 507 and no Power-Troubleshooter 1 over the campaign.

The first calibration takes one of two durations for nearly every x64 run, 7 525 to 7 954 us or
about 9 270 us, three runs falling between them. 236 of the 305 x64 runs start above 8 000 us,
so the rule of the sections above, leaving those runs out, would leave too few to pair: every
run is kept, and each level is compared with the default build of the same pass. 34 runs
of 365 have a calibration above 9 500 us (19 000 for .NET Framework) at the start or the end.
Leaving those out too, some pairings keep two to four passes, and seven more cells leave the
spread: four of the `metrics` build, against its levels, and three for a level, the 4 MiB upload
at v3 on .NET 8 (+28 % MiB/s, -11 % CPU per MiB) and i686 v3's P50 in the clear (-1.9 %). The
verdict below holds either way. Each cell is the median of the five passes, the range in
brackets.

gzip, both ways:

| Host | Engine | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) | Download (MiB/s) | CPU per MiB, download (us) |
|---|---|---:|---:|---:|---:|---:|---:|
| .NET 8 | v1 | 36 (33-39) | 30078 (27832-33350) | 38 (38-39) | 22363 (22266-22852) | 117 (117-121) | 8838 (8691-9570) |
| .NET 8 | v2 | 36 (35-39) | 31494 (28906-32031) | 38 (36-38) | 22412 (22217-23096) | 113 (106-122) | 9082 (8350-9766) |
| .NET 8 | v3 | 37 (36-40) | 30371 (28174-31104) | 39 (38-39) | 22021 (21826-22461) | 113 (107-116) | 8789 (8545-9180) |
| .NET 8 | native | 37 (35-39) | 29834 (27979-31885) | 38 (37-39) | 22510 (22070-22852) | 118 (108-120) | 8594 (8350-9473) |
| .NET 8 AOT | v1 | 37 (35-39) | 28711 (27490-30859) | 39 (38-40) | 22363 (22119-22461) | 110 (100-117) | 7910 (7568-9473) |
| .NET 8 AOT | v2 | 37 (31-38) | 29639 (26709-32812) | 37 (37-38) | 22998 (22607-23145) | 115 (114-116) | 7617 (7568-7861) |
| .NET 8 AOT | v3 | 38 (36-39) | 28711 (27441-30518) | 39 (38-40) | 21973 (21533-22266) | 115 (82-119) | 7861 (7373-9717) |
| .NET 8 AOT | native | 39 (36-40) | 28955 (27197-29785) | 38 (36-39) | 22656 (22363-22900) | 112 (103-120) | 7617 (7178-8398) |
| .NET Framework x86 | i686 v1 | 34 (31-36) | 43994 (41113-46387) | 35 (35-35) | 25244 (25049-25488) | 108 (103-116) | 17480 (16504-19873) |
| .NET Framework x86 | i686 v3 | 34 (33-34) | 43311 (41650-44434) | 35 (35-35) | 25391 (25293-25537) | 122 (114-122) | 15283 (14990-17285) |
| .NET 8 | managed | 58 (57-59) | 21045 (20703-21973) | 46 (45-47) | 18018 (17676-18066) | 112 (89-118) | 11328 (10693-13525) |

zstd, both ways:

| Host | Engine | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) | Download (MiB/s) | CPU per MiB, download (us) |
|---|---|---:|---:|---:|---:|---:|---:|
| .NET 8 | v1 | 127 (115-143) | 10693 (10547-14258) | 100 (95-108) | 6885 (6543-7666) | 97 (95-97) | 4297 (4102-5176) |
| .NET 8 | v2 | 126 (100-143) | 10840 (10596-16357) | 97 (94-108) | 7617 (6885-7764) | 94 (93-96) | 4541 (3906-5225) |
| .NET 8 | v3 | 130 (113-149) | 11035 (9961-12451) | 98 (93-113) | 7178 (6152-7666) | 94 (91-97) | 4199 (3955-4688) |
| .NET 8 | native | 110 (99-155) | 13330 (10205-15039) | 93 (91-94) | 7812 (7471-7959) | 96 (90-96) | 4346 (3711-4736) |
| .NET 8 AOT | v1 | 117 (109-158) | 11523 (9033-11719) | 94 (93-101) | 7471 (6738-8203) | 96 (86-98) | 3320 (3271-4102) |
| .NET 8 AOT | v2 | 136 (104-147) | 9326 (9131-12402) | 94 (93-97) | 7471 (7227-7715) | 95 (91-97) | 3662 (3320-3760) |
| .NET 8 AOT | v3 | 143 (137-158) | 8936 (8838-9473) | 100 (90-112) | 7129 (6299-7715) | 95 (89-100) | 3516 (3076-3955) |
| .NET 8 AOT | native | 111 (100-130) | 10693 (9424-14209) | 95 (90-119) | 7520 (6152-7812) | 96 (86-97) | 3564 (3125-4688) |
| .NET Framework x86 | i686 v1 | 61 (49-69) | 29932 (25098-34961) | 70 (68-72) | 11475 (10645-11816) | 88 (75-91) | 16260 (15332-20410) |
| .NET Framework x86 | i686 v3 | 57 (53-69) | 30420 (24609-31836) | 69 (69-71) | 11719 (10791-12158) | 88 (82-95) | 16455 (15918-18359) |

Calls in flight, over TLS:

| Host | Engine | 16: calls/s | 16: CPU per call (us) | 16: P50 (us) | 64: calls/s | 64: CPU per call (us) | 64: P50 (us) |
|---|---|---:|---:|---:|---:|---:|---:|
| .NET 8 | v1 | 15668 (13760-15710) | 104 (90.2-116) | 1008 (1006-1109) | 21286 (18444-22837) | 64.5 (59.4-94.1) | 2946 (2757-3343) |
| .NET 8 | v2 | 15822 (15647-15971) | 100 (94.5-106) | 998 (983-1007) | 20829 (20103-22726) | 66.8 (59.8-70.3) | 2992 (2774-3104) |
| .NET 8 | v3 | 15805 (15231-19578) | 99.6 (78.9-104) | 996 (808-1010) | 21976 (21196-28116) | 61.3 (52-68.8) | 2828 (2224-2869) |
| .NET 8 | native | 15961 (15856-16086) | 105 (101-108) | 991 (988-995) | 22004 (20920-22472) | 60.5 (59.4-63.3) | 2818 (2799-2967) |
| .NET 8 AOT | v1 | 16042 (12052-20187) | 68.8 (45.3-80.9) | 989 (786-1117) | 21466 (18034-25518) | 52.7 (48-80.9) | 2918 (2502-3252) |
| .NET 8 AOT | v2 | 15249 (13676-19845) | 68.4 (59-89.8) | 1006 (796-1061) | 20923 (20678-26655) | 55.5 (44.5-61.3) | 2987 (2350-3041) |
| .NET 8 AOT | v3 | 15965 (13773-16141) | 73.4 (65.6-78.9) | 992 (984-1074) | 21222 (19751-21809) | 59.8 (55.5-66.4) | 2969 (2900-3120) |
| .NET 8 AOT | native | 15921 (12602-16060) | 65.2 (62.1-77.7) | 992 (984-1068) | 21481 (17216-22016) | 59.4 (47.7-73.8) | 2965 (2893-3329) |
| .NET Framework x86 | i686 v1 | 11244 (10395-12301) | 422 (369-428) | 1318 (1203-1423) | 13603 (12610-13721) | 364 (342-372) | 4551 (4496-4907) |
| .NET Framework x86 | i686 v3 | 10633 (10543-12071) | 425 (398-456) | 1389 (1225-1414) | 14007 (13736-14652) | 350 (307-360) | 4362 (4179-4482) |
| .NET 8 | managed | 18689 (17089-22044) | 210 (170-226) | 800 (681-878) | 29279 (23999-30321) | 136 (130-176) | 1990 (1937-2420) |

Calls in flight, in the clear:

| Host | Engine | 16: calls/s | 16: CPU per call (us) | 16: P50 (us) | 64: calls/s | 64: CPU per call (us) | 64: P50 (us) |
|---|---|---:|---:|---:|---:|---:|---:|
| .NET 8 | v1 | 17599 (16200-21223) | 84.4 (80.1-103) | 878 (748-947) | 23677 (22555-27116) | 62.1 (53.1-69.5) | 2610 (2288-2774) |
| .NET 8 | v2 | 20561 (16595-21569) | 84 (66-116) | 754 (734-947) | 23666 (21891-26034) | 68.4 (58.2-76.2) | 2634 (2414-2832) |
| .NET 8 | v3 | 20882 (16318-21307) | 89.5 (85.5-116) | 752 (738-950) | 24481 (23275-26594) | 62.1 (53.5-66) | 2523 (2325-2703) |
| .NET 8 | native | 16769 (15144-21335) | 106 (73.8-128) | 935 (740-961) | 23218 (21591-26319) | 60.9 (54.7-66) | 2672 (2307-2806) |
| .NET 8 AOT | v1 | 16460 (15478-19420) | 70.3 (49.6-79.7) | 926 (786-965) | 20806 (20558-21719) | 67.6 (56.2-74.6) | 2885 (2795-2904) |
| .NET 8 AOT | v2 | 20283 (16773-21403) | 54.3 (52.7-63.7) | 762 (745-927) | 22148 (20620-27416) | 61.3 (40.6-74.2) | 2784 (2289-2928) |
| .NET 8 AOT | v3 | 18363 (14760-21486) | 64.8 (45.3-82.8) | 860 (740-958) | 23190 (20978-24527) | 57.4 (54.7-62.5) | 2716 (2514-2933) |
| .NET 8 AOT | native | 20615 (16373-21487) | 55.5 (51.2-71.9) | 757 (734-930) | 24192 (20085-27813) | 53.9 (36.3-71.1) | 2455 (2260-2880) |
| .NET Framework x86 | i686 v1 | 11517 (10492-12831) | 433 (378-451) | 1281 (1130-1412) | 14635 (14050-14662) | 329 (322-340) | 4222 (4163-4360) |
| .NET Framework x86 | i686 v3 | 11006 (10821-13117) | 443 (359-457) | 1348 (1111-1354) | 14094 (12911-15587) | 330 (303-350) | 4335 (3970-4784) |
| .NET 8 | managed | 24452 (19716-24977) | 165 (140-207) | 610 (599-760) | 38162 (30584-41430) | 94.5 (93.4-138) | 1497 (1338-1881) |

Large messages, over TLS:

| Host | Engine | Up 4 MiB (MiB/s) | CPU per MiB (us) | Up 16 MiB (MiB/s) | CPU per MiB (us) | Down 4 MiB (MiB/s) | CPU per MiB (us) | Down 16 MiB (MiB/s) | CPU per MiB (us) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| .NET 8 | v1 | 272 (204-367) | 2502 (2075-3357) | 369 (354-446) | 1862 (1495-1923) | 196 (172-213) | 3662 (3418-3937) | 230 (181-274) | 2960 (2502-3387) |
| .NET 8 | v2 | 299 (179-320) | 2350 (2075-3510) | 374 (327-482) | 1831 (1526-2136) | 207 (152-221) | 3357 (2930-4150) | 231 (203-235) | 2991 (2686-3113) |
| .NET 8 | v3 | 319 (264-355) | 2380 (2075-2502) | 367 (328-412) | 1923 (1556-2106) | 207 (171-223) | 3510 (3113-4425) | 229 (216-231) | 3052 (2777-3174) |
| .NET 8 | native | 330 (186-356) | 2167 (2075-3601) | 401 (335-461) | 1801 (1282-1923) | 209 (162-223) | 3448 (2930-4456) | 219 (212-241) | 2930 (2777-3326) |
| .NET 8 AOT | v1 | 343 (256-359) | 1434 (1282-2045) | 380 (366-394) | 1923 (1556-1984) | 207 (170-215) | 3052 (2838-3937) | 233 (184-246) | 3113 (2747-3784) |
| .NET 8 AOT | v2 | 304 (243-370) | 1495 (1434-1770) | 387 (339-441) | 1862 (1465-2106) | 218 (200-226) | 3113 (2533-3357) | 223 (216-231) | 3052 (2991-3204) |
| .NET 8 AOT | v3 | 289 (248-357) | 1648 (1587-1892) | 382 (297-431) | 1862 (1556-2258) | 210 (202-217) | 3113 (2808-3571) | 228 (211-254) | 2869 (2563-3143) |
| .NET 8 AOT | native | 342 (174-347) | 1495 (1434-2686) | 408 (342-521) | 1373 (1282-2167) | 224 (207-230) | 2960 (2716-3662) | 232 (199-264) | 2991 (2563-3143) |
| .NET Framework x86 | i686 v1 | 303 (216-335) | 2380 (2197-3296) | 353 (287-364) | 2563 (2228-2991) | 154 (149-162) | 4791 (4395-5127) | 169 (160-186) | 3876 (3723-4120) |
| .NET Framework x86 | i686 v3 | 308 (223-352) | 2380 (2136-3052) | 348 (241-356) | 2411 (2197-3540) | 170 (152-174) | 4517 (4150-5280) | 161 (150-174) | 4150 (3845-4822) |
| .NET 8 | managed | 100 (88-109) | 12878 (12299-15289) | 100 (91-104) | 10010 (9308-10803) | - | - | - | - |

The full measurement, in the clear:

| Host | Engine | P50 (us) | CPU per call (us) | Download (MiB/s) | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| .NET 8 | v1 | 406 (393-413) | 325 (297-362) | 418 (398-494) | 345 (328-385) | 2250 (1750-2354) | 142 (125-149) | 938 (875-1250) |
| .NET 8 | v2 | 408 (399-412) | 336 (312-364) | 428 (386-470) | 360 (301-383) | 2312 (2000-3104) | 148 (136-170) | 1146 (1062-1229) |
| .NET 8 | v3 | 410 (406-417) | 342 (303-345) | 455 (391-484) | 353 (336-441) | 2417 (2188-2562) | 147 (139-177) | 1125 (917-1208) |
| .NET 8 | native | 408 (399-419) | 334 (298-361) | 439 (331-470) | 376 (289-443) | 2292 (1667-3312) | 147 (134-162) | 1062 (792-1167) |
| .NET 8 AOT | v1 | 403 (398-407) | 241 (216-245) | 424 (401-543) | 389 (354-459) | 1625 (1438-1708) | 157 (136-167) | 958 (854-1083) |
| .NET 8 AOT | v2 | 405 (394-407) | 228 (217-239) | 431 (289-481) | 392 (347-459) | 1771 (1479-2417) | 143 (116-159) | 1042 (896-1250) |
| .NET 8 AOT | v3 | 404 (399-407) | 242 (231-262) | 438 (412-478) | 371 (302-427) | 1500 (1292-1583) | 127 (122-134) | 979 (875-1333) |
| .NET 8 AOT | native | 407 (398-409) | 252 (230-275) | 463 (422-475) | 429 (343-500) | 1667 (1417-1708) | 143 (127-148) | 1125 (896-1146) |
| .NET Framework x86 | i686 v1 | 424 (422-428) | 1022 (995-1094) | 324 (296-363) | 259 (238-267) | 5292 (4875-6625) | 139 (131-164) | 1146 (1063-1208) |
| .NET Framework x86 | i686 v3 | 420 (403-423) | 988 (906-1031) | 309 (303-362) | 282 (253-294) | 4813 (4500-4938) | 136 (134-156) | 1208 (979-1271) |
| .NET 8 | managed | 378 (366-405) | 570 (517-686) | 455 (439-493) | 188 (140-214) | 6750 (6667-8792) | 141 (116-153) | 6500 (5750-8146) |

The `metrics` build, the full measurement over TLS:

| Host | Engine | P50 (us) | CPU per call (us) | Download (MiB/s) | Upload, stream (MiB/s) | CPU per MiB, stream (us) | Upload, unary (MiB/s) | CPU per MiB, unary (us) |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| .NET 8 | metrics v1 | 442 (438-448) | 309 (272-359) | 402 (264-483) | 457 (363-474) | 3375 (2583-3833) | 375 (367-387) | 1833 (1667-1875) |
| .NET 8 | metrics v2 | 455 (441-475) | 291 (273-355) | 384 (330-431) | 543 (395-553) | 2792 (2146-3583) | 389 (346-481) | 1938 (1542-2062) |
| .NET 8 | metrics v3 | 445 (441-455) | 345 (302-355) | 407 (330-444) | 460 (347-569) | 2979 (2688-3250) | 354 (316-402) | 1875 (1729-2021) |
| .NET 8 | metrics native | 454 (438-462) | 336 (311-348) | 354 (325-380) | 564 (390-626) | 2729 (2375-3250) | 371 (345-424) | 1792 (1708-2104) |

The `metrics` build, calls in flight over TLS:

| Host | Engine | 16: calls/s | 16: CPU per call (us) | 16: P50 (us) | 64: calls/s | 64: CPU per call (us) | 64: P50 (us) |
|---|---|---:|---:|---:|---:|---:|---:|
| .NET 8 | metrics v1 | 15740 (15436-19647) | 106 (80.9-110) | 1006 (801-1026) | 20943 (19501-27591) | 58.2 (58.2-69.5) | 2967 (2251-3091) |
| .NET 8 | metrics v2 | 15506 (15035-15861) | 103 (98.4-112) | 1017 (993-1039) | 20975 (19922-22143) | 71.5 (63.3-77) | 2976 (2835-3092) |
| .NET 8 | metrics v3 | 15509 (15434-15642) | 97.7 (90.2-120) | 1015 (1009-1021) | 21273 (19504-22257) | 60.2 (56.2-71.5) | 2927 (2812-3096) |
| .NET 8 | metrics native | 15654 (15601-16057) | 107 (94.9-110) | 1007 (990-1013) | 20902 (20198-22080) | 65.2 (62.9-79.3) | 2989 (2844-3040) |

Each level was also paired with the default build of its pass, per figure: 312 comparisons. In 19
of them every pass moved the same way, about what chance gives for five passes (6.25 %, 19.5).
In 8 the level's range and the default build's do not overlap, four for the level and four
against it:

| Workload | Host | Engine | Figure | Paired change (range) |
|---|---|---|---|---:|
| concurrent | .NET 8 | native | 16 in flight, calls a second | +1.9 % (+1.1 to +16.6) |
| concurrent | .NET 8 | native | 16 in flight, P50 | -1.6 % (-10.9 to -1.1) |
| concurrent | .NET Framework x86 | i686 v3 | 64 in flight, calls a second | +7.4 % (+0.1 to +10.3) |
| concurrent | .NET Framework x86 | i686 v3 | 64 in flight, P50 | -7.8 % (-11.2 to -1.5) |
| gzip | .NET 8 | v3 | download, MiB/s | -6.6 % (-8.5 to -0.9) |
| gzip | .NET 8 AOT | v2 | upload, unary, CPU per MiB | +3.0 % (+2.2 to +4.0) |
| h2c | .NET 8 AOT | v3 | upload, unary, MiB/s | -20.1 % (-24.0 to -1.5) |
| zstd | .NET 8 | native | upload, unary, MiB/s | -7.0 % (-13.0 to -2.1) |

What the numbers say:

- **No level is worth shipping on this processor, in any of these workloads.** No level moves a
  figure beyond the spread in the same direction on two hosts, or with TLS and in the clear; the
  cells beyond it go both ways, by 2 to 8 % for a level and up to 20 % against.
- **Where the engine's own code is most of the work, the levels still change nothing.**
  Compressing with gzip costs 22 to 25 ms of CPU per MiB sent as one message, against about 1 ms
  for the same upload uncompressed in the clear, so compression is most of it; that it is
  miniz_oxide's deflate, which the level recompiles, is inferred, not profiled. That CPU per MiB
  moves by -1.5 % (-3.8 to +0.4) at v3 on .NET 8, -1.3 % (-3.7 to +0.7) ahead-of-time and +0.6 %
  (-0.8 to +1.4) for i686 v3, with ranges of 1 to 3 %; the one cell beyond the spread is against
  a level, v2 ahead-of-time, +3.0 % (+2.2 to +4.0).
- **zstd is the control.** Its C is the same in every build; its figures move within the spread
  but for `native`'s unary upload, against it.
- **Many calls in flight, and large messages, stay within the spread**, every run kept, but for
  `native` at 16 in flight on .NET 8 (2 %), and i686 v3 at 64 with TLS (7 %), which the same
  engine in the clear does not repeat (-1.6 %, -9.0 to +6.5).
- **In the clear, the one cell beyond the spread is against a level**: one message uploaded at v3
  ahead-of-time, -20.1 % (-24.0 to -1.5).
- **No level changes anything on the `metrics` build either**: with every run kept, each of its
  cells is within the spread.
- **Beside the levels**: grpc-dotnet uploads gzip faster (58 MiB/s streamed against 36) for less
  CPU per MiB (21 against 30 ms); no run varies either side's compression level. At 64 calls in
  flight it makes 1.37 times as many calls a second as the engine over TLS and 1.61 times in the
  clear.

## Large uploads beside unary calls, over mutual TLS

The workload ArmoniK's clients weigh most is a client that presents a certificate, uploads
payloads of 100 MiB and more as client streams, and makes many small unary calls, all on one
channel and at the same time. The benchmark's `mixed` scenario runs it:

- **Mutual TLS.** The test server, started with `--mtls`, requires a client certificate and accepts
  one only when it chains to the authority that issued its own certificate; a certificate from
  another authority was refused in a manual check when the option was added. It issues that client
  certificate at start, writes it and its key, PKCS#8, as PEM files it names on stdout
  (`CLIENT_CERT` and `CLIENT_KEY`, beside `CA`), and removes them when it stops. The native channel
  presents them through `Transport.Tls.ClientCertificate.Pem`, grpc-dotnet through
  `GrpcClient.CertPem` and `KeyPem`, which `GrpcChannelFactory` hands to its handler. The `-h2c`
  transports run the scenario in the clear.
- **The uploads**: K workers (`k=`, 1 by default) take client-streaming `Upload` calls of `size`
  MiB, 128 by default, in chunks of `chunk` KiB, 64 by default, from one count, each starting the
  next when its last ends. The count is what it takes for the run to send at least 2 GiB, or eight
  uploads' worth when that is more, and at least K: 16 uploads of 128 MiB, 8 of 512 MiB.
- **The unary calls**: M workers (`m=`, 16 by default), each calling `Say` with 256 characters,
  which come back, and starting the next once the last ends.
- **One channel** carries both. `pool=N` sets the native channel's
  `Http2.SimultaneousCallsPerConnection` to `{"Limit": N}`: a call goes on the fullest connection
  with room, a new one when none has. `pool=server`, the default, leaves it at `"FromServer"`, as
  many calls on a connection as the server allows, 100 for this one. grpc-dotnet has no such
  option through `GrpcChannelFactory`, and the two do not compare as pools: on .NET its
  `HttpClientHandler` sets neither `EnableMultipleHttp2Connections` nor `MaxConnectionsPerServer`,
  so it keeps to one connection and queues the calls past the server's 100 streams, while on .NET
  Framework `WinHttpHandler` opens connections of its own accord; `conns` says how many.
- **The warm-up.** The server first, on a channel of its own that closes before the run's opens:
  50 000 `Say` at 64 in flight, and four uploads of 64 MiB at once in the run's chunks. The
  server's just-in-time compiler moves a method to optimized code only once it has run a while,
  and a server warmed by a few thousand calls is still compiling; its CPU per call over the last
  10 000 of those calls (`srv_warm_us_call`) shows one that is. Then the run's channel: 2 000
  calls with M in flight and K uploads of 16 MiB at once, which open the connections the run uses
  and no others.
- **The window.** The calls run alone for 500 ms, and the window opens as the uploads start. It
  closes when the last upload ends, or after 5 s when K is 0. M = 0 runs the uploads alone.

A run's line names its parameters (`mixed-k4-m64-s128-c64-p16`, `pS` for `pool=server`), then gives,
over the window (`window_s` long): the uploads' MiB per second and their median and longest duration
(`l_*`); the unary calls that end in the window, a call held across its opening included, per second
and their P50, P99, P99.9 and longest (`u_*`); the CPU of the client and of the server per MiB and
per call, both tasks' together (`*_both`), the upload figures and the CPU per MiB absent when K is
0, the unary figures and the CPU per call when M is 0, and the latencies and the CPU per call also
when no call ends in the window; the CPU in cores (`cpu_cores`, `srv_cpu_cores`), the server's
telling a run it held back; and the client's busiest thread in cores (`busiest_thread_cores`), a
native channel's engine work running on one thread. Sampled every 250 ms, since the process's own
peak would count the warm-up's: the most connections established to the server (`conns`) and the
most private bytes above the client's before its channel opened (`peak_above_mib`, above
`base_private_mib`), a run none of whose samples succeeds failing, and the failed ones counted
(`samples_failed`, which `campaign.ps1` reports for the worst pass rather than the median).

## The machine

Windows 11 Pro 10.0.26100, 13th Gen Intel Core i7-1360P, 16 GB, with the desktop's usual
applications open and no other build or test running. The i7-1360P is a hybrid processor: four
performance cores, logical processors 0 to 7, and eight efficiency cores. A unary call takes about
twice as long on the second kind, and a run the scheduler spreads over both measures where its
threads landed, so every run is held to logical processors 0 to 7, the test server with it, at
AboveNormal priority.

Built with .NET SDK 11.0.100-rc.1; run on .NET 8.0.19, .NET 10.0.12 and .NET Framework 4.8,
ahead-of-time with the 8.0.31 and 10.0.12 ILCompiler packages; rustc 1.94.1, the engine built
with Cargo's default release profile, LTO within each crate only, unless a section says
otherwise: the workspace's own, fat LTO, dates from 2026-10-17. The client and the server share
the machine. The runs of `bf86fece3`, and those of 2026-10-15, used the same SDK, runtimes,
ILCompiler packages and compiler.

## Running it again

On Windows 11 or Windows Server 2022 or later: on .NET Framework the managed side runs over
`WinHttpHandler`, which carries gRPC there only. From a Visual Studio x64 developer PowerShell, in
`packages/csharp/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks`, `campaign.ps1` builds, publishes
the ahead-of-time variants, runs the passes interleaved and prints the medians:

```powershell
.\campaign.ps1 -Frameworks net4.8,net8.0,net10.0 -Aot -Affinity 0xFF -Priority AboveNormal
```

`-Affinity` takes a mask of logical processors, the performance cores on a hybrid processor.
`-ServerAffinity` gives the server a mask of its own, which shares no processor with `-Affinity`,
so that client and server never compete for a core; it needs `-Affinity`. Without
`-ServerAffinity` the server takes `-Affinity`.
`-Dotnet` names the SDK that builds, and `-DotnetRoot` the install that holds the .NET 10 and .NET
11 runtimes when the system-wide one does not, which `-DotnetRootFor` lists. A developer prompt's
linker is passed to the ahead-of-time publish with `-PublishArgs -p:IlcUseEnvironmentalTools=true`.
`-Transports` and `-Scenarios` choose what runs, `native` and `managed`, idle and busy, by default;
`native-batch` needs the default build, against h2-batch's patch, which takes Git Bash first on the
PATH on Windows; `-BuildArgs -p:NativeEngineH2Batch=false` builds the engine on stock h2, which
refuses it. `native-metrics` loads the engine's build with its counters, which the benchmark's
build copies beside it unless `-p:NativeEngineMetrics=false`, and which it selects with
`NativeLibrarySelection` before the first runtime exists; the published ahead-of-time benchmarks do
not carry that build, so it does not run there. `native-spin` runs the engine with `BeforeSleep`
`{"Spin": {}}`, a channel's thread polling for work for the default window, 100 us, after each
callback into the host before it sleeps. `native-h2c` and `managed-h2c` speak HTTP/2 in the clear,
prior knowledge, to the test server started without TLS, which tells what TLS costs from the rest;
`managed-h2c` does not run on .NET Framework, whose `WinHttpHandler` speaks HTTP/2 only over TLS.
Either build writes its engine over the other's, so a run of both builds takes a copy of the
output directory before the second build. A run that fails is reported and left out, and a cell
left with fewer passes says so. `pinned` and `shared`, in `-Scenarios`, measure the unary latency
alone, on a native channel held to one connection, without and with a streamed upload beside it; the
managed transports skip them. `gzip`, `zstd`, `concurrent` and `large` run the workloads of "The
processor levels, on LTO engines", each summed up in a table of its own; the managed transports
skip `zstd`, and the test server built here refuses it: that section says how to build one that
takes it. A `gzip` or `zstd` run fails if a leg's stream does not announce the encoding; a
message the engine leaves uncompressed under that announcement, as it may, is not detected.
`mixed` runs the scenario of "Large uploads beside unary calls, over mutual TLS", its parameters
after it: `k=` (K, at most 64), `m=` (M, at most 1 024, K + M above 0), `size=` in MiB (at most
4 096) and `chunk=` in KiB (at most 1 GiB, dividing the size), `pool=`, `server` or a number, and
`fpw=`, the native channel's `Http2.Send.FramesPerWrite` in place of the build's, which names the
run with `-fN` after its pool; a numbered pool and `fpw=` are refused by the managed transports. A
value outside these, or an unknown key, has the benchmark print its usage and exit 2. In
`-Scenarios` the parameters follow it after spaces, as in `"mixed k=4 m=64 pool=16"`; the managed
transports skip a numbered pool and `fpw=`, and the mixed runs share one table, a row per set of
parameters, which each run's line names (`mixed-k4-m64-s128-c64-p16`).

The DATA after a cancel and the PINGs' waits come from the test `h2_batch_costs`, run in the
stock build and in the build against h2-batch's patch:

```bash
cargo test -p armonik-transport --features test-hooks --release --test h2_batch_costs -- --include-ignored --nocapture --test-threads 1
```

```bash
packages/rust/patches/h2-batch/build.sh test -p armonik-transport --features test-hooks --release --test h2_batch_costs -- --include-ignored --nocapture --test-threads 1
```

A single run is the benchmark itself, with the transport and `busy` for the busy scenario, or
`pinned` or `shared` for the unary latency alone on one connection:

```bash
bin/Release/net8.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native busy
```

It prints one line: the latencies, the download and the two uploads, the allocation per call and
on .NET the thread pool work items, and for an idle run the CPU per call and per MiB uploaded and
the memory before and after the uploads. `ARMONIK_BENCH_AFFINITY` and `ARMONIK_BENCH_PRIORITY` set
where it and its server run; `ARMONIK_BENCH_SERVER_AFFINITY` pins the server to a mask of its
own, which shares no processor with `ARMONIK_BENCH_AFFINITY`, which pins the benchmark itself.
The benchmark starts its test server, a `net8.0` program, with `--plain-say` and the `dotnet` the
system finds first, which has to be an install with a .NET 8 runtime; `ARMONIK_BENCH_SERVER`
names another build of the server instead, and `ARMONIK_BENCH_SERVER_HOST` the `dotnet` that
runs it. Every run's line ends with the two calibrations.
