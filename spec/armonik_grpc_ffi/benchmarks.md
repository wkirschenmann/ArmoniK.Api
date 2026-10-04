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
- **Server-streaming throughput**: `Stream` sends 2 000 chunks of 64 KiB, 125 MiB; five runs, the
  median in MiB/s. At 64 KiB a chunk, 16 messages a second per MiB/s.
- **Memory**: what the process holds after both, less what it held before the channel was
  opened, each read after a full collection: private bytes, and the managed heap. Read with the
  channel still open, so each stack's one-time costs - loading, compiling - are in it.

Every transport option is left at its default, the runtime's ceilings included.

Every measurement runs in two scenarios. **Idle**: the benchmark is all the process does, so the
.NET thread pool sleeps between calls. **Busy**: two pool workers compute for 50 us and yield,
over and over, as in an application whose pool is never idle.

## Results

Three passes, the eight runs of a pass interleaved; each cell is the median of the three.

| Host | Scenario | Transport | P50 (us) | P95 (us) | P99 (us) | Stream (MiB/s) |
|------|----------|-----------|---------:|---------:|---------:|---------------:|
| .NET 8 | idle | native | 517 | 1590 | 2087 | 161 |
| .NET 8 | idle | managed | 400 | 1274 | 1869 | 189 |
| .NET 8 | busy | native | 785 | 1490 | 1880 | 186 |
| .NET 8 | busy | managed | 657 | 1169 | 1629 | 181 |
| .NET Framework 4.8 | idle | native | 708 | 1414 | 1888 | 150 |
| .NET Framework 4.8 | idle | managed | 3395 | 5491 | 7602 | 133 |
| .NET Framework 4.8 | busy | native | 717 | 1021 | 1254 | 177 |
| .NET Framework 4.8 | busy | managed | 3243 | 5335 | 6515 | 128 |

Memory, idle: native holds 14.3 MiB of private bytes on .NET 8 and 13.2 on .NET Framework,
managed 17.7 and 17.4; the managed heap grows by 0.2 and 0.3 MiB native, 1.1 and 0.7 managed.
A busy run does not report memory: the load's own is in it.

What the numbers say:

- **On .NET Framework the native engine is the faster transport**, about 4.8 times at P50 and 4
  times at P99 idle, 4.5 and 5.2 times busy, and it streams faster in each pass of both
  scenarios. grpc-dotnet there runs over `WinHttpHandler`.
- **On .NET 8 it is the slower one at the median**, by about 120 us idle and 130 us busy, and so
  in each pass of both. Idle, its P99 is above grpc-dotnet's in each pass too; busy, the passes
  disagree. Streaming is within the spread, the passes of the two overlapping.
- **A busy pool lowers both stacks' P99 and leaves the gap at the median.** Native's P99 falls
  from 2087 to 1880 us on .NET 8 and from 1888 to 1254 on .NET Framework, grpc-dotnet's from 1869
  to 1629 and from 7602 to 6515. On .NET 8 both medians rise, native's from 517 to 785 us and
  grpc-dotnet's from 400 to 657, the load taking CPU from both; on .NET Framework they barely
  move. Why the tail falls is not measured here; a pool worker found awake rather than woken is
  the likely part.
- **The native process holds 3 to 4 MiB less**, and its managed heap barely moves, as the design
  has it: what it receives lives in native memory until the host gives it back.
- **The passes spread.** The idle P50 of native on .NET 8 ran from 517 to 885 us, which is why
  each cell is a median, and why a difference smaller than that is reported only where every pass
  shows it.

What this baseline does not settle: the HTTP/2 window default, which the audit response leaves to
these benchmarks, needs a link with latency. Loopback has none, so no window size shows here.

## The machine

Windows 11 Pro 10.0.26100, 13th Gen Intel Core i7-1360P, 16 GB, with no other build or test
running. .NET SDK 9.0.304, rustc 1.94.1, the engine built with the release profile.
The client and the server share the machine.

## Running it again

On Windows 11 or Windows Server 2022 or later: on .NET Framework the managed side runs over
`WinHttpHandler`, which carries gRPC there only. From a Visual Studio x64 developer prompt, in
`packages/csharp`:

```bash
dotnet build ArmoniK.Api.Client.RustGrpcChannel.Benchmarks -c Release
```

then, for each of `net4.8`, `net8.0` and `net10.0` and each of `native` and `managed`, from
`ArmoniK.Api.Client.RustGrpcChannel.Benchmarks/bin/Release`:

```bash
dotnet net8.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.dll native
```

```bash
net4.8/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native
```

and the same with `busy` after the transport for the busy scenario. Each run prints one line: the
latencies and the throughput, and for an idle run the two memory figures. Built with a .NET 11
SDK, the project also targets `net11.0`. The benchmark starts its test server, a `net8.0`
program, with the `dotnet` the system finds first, which has to be an install with a .NET 8
runtime. A run on an SDK installed apart from that one therefore goes through the apphost, with
`DOTNET_ROOT` naming the other install:

```bash
DOTNET_ROOT=<the other install> net11.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native
```
