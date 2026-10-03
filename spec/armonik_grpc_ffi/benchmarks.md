# Benchmarks: native against managed

The baseline requirement 16 asks for: the native engine through `NativeChannel` against
grpc-dotnet through `GrpcChannelFactory`, on .NET Framework 4.8 and .NET 8, measured with the engine
of the commit that adds this document. It is a baseline, not a verdict: one machine, one server, the loopback
interface, and numbers to compare the next measurement with.

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

## Results

Three passes, interleaved; each cell is the median of the three.

| Host | Transport | P50 (us) | P95 (us) | P99 (us) | Stream (MiB/s) | Private (MiB) | Managed heap (MiB) |
|------|-----------|---------:|---------:|---------:|---------------:|--------------:|-------------------:|
| .NET 8 | native | 839 | 1553 | 1981 | 155 | 13.7 | 0.2 |
| .NET 8 | managed | 704 | 1436 | 1925 | 162 | 18.2 | 1.1 |
| .NET Framework 4.8 | native | 1026 | 1519 | 1879 | 148 | 13.4 | 0.3 |
| .NET Framework 4.8 | managed | 4481 | 7557 | 10193 | 128 | 17.4 | 0.7 |

What the numbers say:

- **On .NET Framework the native engine is the faster transport**, about 4.4 times at P50 and 5.4
  times at P99, and it streams faster in each pass, 148 MiB/s against 128 at the median.
  grpc-dotnet there runs over `WinHttpHandler`.
- **On .NET 8 it is the slower one at the median for small unary calls**, by about 135 us, and so
  in each of the three passes; at P99 the two are level. Where those microseconds go is not
  measured here. The likely place is the boundary: the start, the head, the message and the status
  each cross it, and each completion is a callback into managed code, where the managed stack has
  nothing to cross. grpc-dotnet streams faster there in each pass, 162 MiB/s against 155 at the
  median.
- **The native process holds 4 to 5 MiB less**, and its managed heap barely moves, as the design
  has it: what it receives lives in native memory until the host gives it back.
- **The passes spread.** The P50 of one configuration varied by up to 30 percent from one pass to
  another on this machine - native on .NET Framework ran from 823 to 1068 us - which is why each
  cell is a median, and why a difference smaller than that is reported only where every pass
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

then, for each of `net4.8` and `net8.0` and each of `native` and `managed`, from
`ArmoniK.Api.Client.RustGrpcChannel.Benchmarks/bin/Release`:

```bash
dotnet net8.0/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.dll native
```

```bash
net4.8/ArmoniK.Api.Client.RustGrpcChannel.Benchmarks.exe native
```

Each run prints one line with the six figures above.
