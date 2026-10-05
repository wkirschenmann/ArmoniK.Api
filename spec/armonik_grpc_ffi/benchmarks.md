# Benchmarks: native against managed

The baseline requirement 16 asks for: the native engine through `NativeChannel` against
grpc-dotnet through `GrpcChannelFactory`, on .NET Framework 4.8 and .NET 8, measured with the
engine of the commit that adds these figures. It is a baseline, not a verdict: one machine, one
server, the loopback interface, and numbers to compare the next measurement with.

## What is measured

`ArmoniK.Api.Client.RustGrpcChannel.Benchmarks` measures one transport per process, so that what
one leaves allocated is not counted against the other, against the test server over TLS on
loopback. TLS for both, because grpc-dotnet on .NET Framework speaks HTTP/2 only over it.

- **Unary latency**: 1 000 warm-up calls, then 10 000 sequential `Say` calls of a one-character
  message, each timed alone. P50, P95 and P99 in microseconds.
- **Allocation**: the managed bytes the process allocates over those 10 000 calls, per call, and
  on .NET 8 the thread pool's work items per call.
- **Download**: `Stream` sends 2 000 chunks of 64 KiB, 125 MiB; five runs, the median in MiB/s.
  At 64 KiB a chunk, 16 messages a second per MiB/s.
- **Upload**: 150 MiB sent to the server, five runs of each, the median in MiB/s: as a client
  stream of 64 KiB chunks (`Upload`), and as one unary message (`UploadWhole`).
- **Memory**: what the process holds after the latency and the download, less what it held
  before the channel was opened, each read after a full collection: private bytes, and the
  managed heap. Read with the channel still open, so each stack's one-time costs - loading,
  compiling - are in it. The private bytes are read again after the uploads, the 150 MiB message
  unreachable by then.

Every transport option is left at its default, the runtime's ceilings included, except for
**native-batch**: the native engine built against h2-batch's patch of h2
(`packages/rust/patches/h2-batch`) with `Http2.Send.FramesPerWrite` at 16.

Every measurement runs in two scenarios. **Idle**: the benchmark is all the process does, so the
.NET thread pool sleeps between calls. **Busy**: two pool workers compute for 50 us and yield,
over and over, as in an application whose pool is never idle.

## Results

Three passes, the twelve runs of a pass interleaved; each cell is the median of the three.

| Host | Scenario | Transport | P50 (us) | P95 (us) | P99 (us) | Download (MiB/s) | Upload, stream (MiB/s) | Upload, unary (MiB/s) |
|------|----------|-----------|---------:|---------:|---------:|-----------------:|-----------------------:|----------------------:|
| .NET 8 | idle | native | 681 | 1467 | 1857 | 144 | 234 | 197 |
| .NET 8 | idle | native-batch | 545 | 1303 | 1871 | 173 | 316 | 222 |
| .NET 8 | idle | managed | 573 | 1150 | 1753 | 132 | 71 | 79 |
| .NET 8 | busy | native | 612 | 1095 | 1331 | 200 | 281 | 284 |
| .NET 8 | busy | native-batch | 625 | 943 | 1104 | 182 | 371 | 262 |
| .NET 8 | busy | managed | 656 | 1041 | 1395 | 163 | 74 | 85 |
| .NET Framework 4.8 | idle | native | 595 | 1443 | 1781 | 147 | 326 | 189 |
| .NET Framework 4.8 | idle | native-batch | 573 | 1348 | 1741 | 159 | 365 | 226 |
| .NET Framework 4.8 | idle | managed | 2835 | 5925 | 7513 | 129 | 45 | 133 |
| .NET Framework 4.8 | busy | native | 756 | 1140 | 1340 | 152 | 230 | 176 |
| .NET Framework 4.8 | busy | native-batch | 721 | 1034 | 1403 | 141 | 308 | 208 |
| .NET Framework 4.8 | busy | managed | 3193 | 5005 | 6013 | 129 | 46 | 127 |

Allocation per unary call, idle: native 3.6 KB on .NET 8 and 4.7 KB on .NET Framework, managed
8.0 KB and 36 KB. Thread pool work items per call on .NET 8, idle: native 2.0, managed 9.1. A busy
run's allocations and work items are not reported: the load's own are in them.

Memory, idle: native holds 12.6 to 14.7 MiB of private bytes on .NET 8 and 11.2 to 11.7 on .NET
Framework, managed 16.9 to 19.4 and 17.2 to 17.3; the managed heap grows by 0.3 MiB native, 1.0 to 1.2
and 0.7 managed. After the uploads, the private bytes stand 466 to 468 MiB above the start native and
842 to 1145 managed on .NET 8, and 314 to 318 native and 175 to 185 managed on .NET Framework.
A busy run does not report memory: the load's own is in it.

What the numbers say:

- **Uploads are where the native engine is furthest ahead.** A streamed upload runs 3 to 4 times
  faster than grpc-dotnet's on .NET 8 and 5 to 7 times on .NET Framework; a unary one 2.5 to 3.3
  times on .NET 8 and about 1.4 times on .NET Framework, where `WinHttpHandler` sends a whole
  message better than a stream. Each pass of each scenario shows it.
- **h2-batch raises the streamed upload by 12 to 35 % in the medians.** On .NET 8 idle it is
  above the stock engine in every pass, 234 to 316 MiB/s in the medians; in the three other
  scenarios one pass has it below. What it changes, fewer and larger writes, shows first in the
  CPU and the syscalls a send costs, which this benchmark does not measure. It moves neither the
  latency nor the download beyond the spread.
- **On .NET Framework the native engine is the faster transport for latency**, about 4.8 times at
  P50 and 4.2 times at P99 idle, 4.2 and 4.5 times busy.
- **On .NET 8 the latencies are within the spread idle; busy, the native P50 is below
  grpc-dotnet's in every pass**, 612 against 656 us. Idle, the native P50 ran from 446 to 785 us
  and grpc-dotnet's from 420 to 693 across the passes.
- **The native engine downloads at least as fast**, faster in every pass busy on both hosts and
  idle on .NET Framework, within the spread idle on .NET 8.
- **The native process allocates half as much per call on .NET 8 and an eighth on .NET
  Framework**, and on .NET 8 idle runs a fifth of the thread pool work items: what it receives
  lives in native memory until the host gives it back.
- **After a 150 MiB send, the native channel keeps the 150 MiB arena** as a spare for the next
  send of that size, counted against the runtime's memory ceiling and freed first when the count
  nears it, which with the default ceiling is not reached here: the private bytes after the
  uploads hold it.
- **The passes spread.** The machine ran the desktop's usual applications, and a cell is a median
  of three for that reason; a difference smaller than the spread is reported only where every
  pass shows it.

What this baseline does not settle: the HTTP/2 window default, which the audit response leaves to
these benchmarks, needs a link with latency. Loopback has none, so no window size shows here.

## The machine

Windows 11 Pro 10.0.26100, 13th Gen Intel Core i7-1360P, 16 GB, with no other build or test
running. Built with .NET SDK 11.0.100-rc.1, run on .NET 8.0.19 and .NET Framework 4.8, rustc
1.94.1, the engine built with the release profile. The client and the server share the machine.

## Running it again

On Windows 11 or Windows Server 2022 or later: on .NET Framework the managed side runs over
`WinHttpHandler`, which carries gRPC there only. From a Visual Studio x64 developer prompt, in
`packages/csharp`:

```bash
dotnet build ArmoniK.Api.Client.RustGrpcChannel.Benchmarks -c Release
```

then, for each of `net4.8`, `net8.0` and `net10.0` and each of `native`, `native-batch` and
`managed`, from
`ArmoniK.Api.Client.RustGrpcChannel.Benchmarks/bin/Release`:

```bash
dotnet net8.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.dll native
```

```bash
net4.8/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native
```

and the same with `busy` after the transport for the busy scenario. Each run prints one line: the
latencies, the download and the two uploads, the allocation per call and on .NET the thread pool
work items, and for an idle run the memory before and after the uploads.

`native-batch` needs the engine built against h2-batch's patch, which takes Git Bash first on
the PATH; a build without it refuses the option. The two builds write the same engine, so the
output of one is copied aside before the other:

```bash
dotnet build ArmoniK.Api.Client.RustGrpcChannel.Benchmarks -c Release -p:NativeEngineH2Batch=true
```

Built with a .NET 11 SDK, the project also targets `net11.0`. The benchmark starts its test
server, a `net8.0` program, with the `dotnet` the system finds first, which has to be an install
with a .NET 8 runtime. A run on an SDK installed apart from that one therefore goes through the
apphost, with `DOTNET_ROOT` naming the other install:

```bash
DOTNET_ROOT=<the other install> net11.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native
```
