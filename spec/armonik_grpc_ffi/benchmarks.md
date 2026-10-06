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
with the release profile. The client and the server share the machine.

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
option. A run that fails is reported and left out, and a cell left with fewer passes says so.

A single run is the benchmark itself, with the transport and `busy` for the busy scenario:

```bash
bin/Release/net8.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native busy
```

It prints one line: the latencies, the download and the two uploads, the allocation per call and
on .NET the thread pool work items, and for an idle run the CPU per call and per MiB uploaded and
the memory before and after the uploads. `ARMONIK_BENCH_AFFINITY` and `ARMONIK_BENCH_PRIORITY` set
where it and its server run. The benchmark starts its test server, a `net8.0` program, with the
`dotnet` the system finds first, which has to be an install with a .NET 8 runtime.
