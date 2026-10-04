# SPEC - ArmoniK .NET Client on Native Rust gRPC Channel

## Objectives

1. Replace the managed gRPC channel (`Grpc.Net.Client` / `WinHttpHandler`) with a native Rust
   gRPC channel, exposed to .NET through a thin C ABI.
2. **Unify client behavior** whatever the .NET runtime (Framework 4.7.2 and 4.8, .NET 8): one
   transport removes the divergences between `WinHttpHandler`, `SocketsHttpHandler`,
   `GrpcWebHandler` and their conditional fallbacks.
3. Make the options that are inoperative today (`RequestTimeout`, `OverrideTargetName`)
   effective.
4. Preserve the client's public API: the same generated stubs, over another `CallInvoker`.

---

## Layered architecture

Five layers, each a component of the implementation. They are not the TLA+ levels: those count
abstraction, and a level is not a layer.

```text
+------------------------------------------------------------+
| 5. ArmoniK.Api.Client                   C#, netstandard2.0 |
|    Generated stubs over a CallInvoker the caller supplies  |
+------------------------------------------------------------+
| 4. ArmoniK.Api.Client.RustGrpcChannel   C#, netstandard2.0 |
|    A native CallInvoker; the runtime, its channels and     |  and net8.0
|    their calls; the callback trampoline                    |
+------------------------------------------------------------+
| 3. armonik-transport-ffi                      Rust, cdylib |
|    The C ABI: an owned tokio runtime, handles, callbacks,  |
|    the memory ceiling, shutdown and quiescence             |
+------------------------------------------------------------+
| 2. armonik-transport :: grpc               Rust, lib crate |
|    gRPC calls, the four cardinalities, metadata, status -  |  layers 1 and 2 are
|    tonic's client over the channel's own HTTP/2 session    |  two modules of one
+------------------------------------------------------------+  crate
| 1. armonik-transport :: http2              Rust, lib crate |
|    The connector: endpoint, TCP, the HTTP/2 handshake,     |
|    the connect timeout                                     |
+------------------------------------------------------------+
```

**Layer 1** dials one endpoint and hands the gRPC layer an HTTP/2 connection. It knows no gRPC
notion - no status, no retry, no deadline. TLS and the proxy arrive in phases 4 and 5.

**Layer 2** turns the connection into calls: a pooled session per channel, tonic's client for
the framing and the status, and a small set of checks tonic does not make. It owns no runtime:
it spawns on the one it is given. Retry and deadlines arrive in phase 6.

**Layer 3** projects layer 2 onto a C ABI for any host. It owns what a foreign caller cannot:
the tokio runtime, the handles and their validation, the callbacks and their serialization per
call, the memory it lends and its ceiling, and a shutdown that ends in a state certifying the
library may be unloaded.

**Layer 4** implements `Grpc.Core.CallInvoker` over the ABI, for .NET Framework and .NET alike.
The runtime is an object its caller creates and disposes; channels are made from it and swept
by its disposal; each call publishes its events into a ring its reader drains.

**Layer 5** is the existing client. It takes the `CallInvoker` its caller chooses, so the native
channel reaches it without a selector inside it.

The package multi-targets `netstandard2.0` and `net8.0`. Behavior is identical on both, which is
the point of objective 2; the second target is there for the APIs it adds, not for a different
path.

---

## Design decisions that hold across the layers

1. **A callback per event, MsQuic-style** - no completion queue at the ABI.
2. **One gRPC engine** - no framing or retry duplicated in the binding.
3. **`CallInvoker` as the .NET integration point**, not `HttpMessageHandler`, so the transport
   does not depend on `Grpc.Net.Client`.
4. **The call's root is published before `ak_call_start`**, which closes the race with a callback
   that arrives before the start returns.
5. **The FFI owns its tokio runtime, and one exists per process.**
6. **Handles are drawn from a counter per kind, over disjoint ranges, and never reused**, so a
   stale handle is refused rather than dereferenced.
7. **Configuration is immutable once a channel is created.**
8. **`AK_RUNTIME_QUIESCENT` is the only condition for unloading the library** - safety before
   liveness.
9. **Releasing a channel is synchronous and cancels its calls.**
10. **The JSON schema is the source of truth for the options**: the C# class is generated from
    it, never the reverse.
11. **Connecting is lazy**: a channel dials on its first call, unless `ConnectEagerly` asks
    `ak_channel_create` to dial as it registers the channel.
12. **`end_send`, not `half_close`**, for the call that says nothing more will be sent.

---

## The documents, and which one wins

| Document | What it holds |
|----------|---------------|
| [requirements.md](requirements.md) | What the transport must do, as acceptance criteria. A criterion not yet met names the task that meets it. |
| [design.md](design.md) | How, in five documents it indexes: [contract.md](contract.md), each layer's promises; [abi.md](abi.md), the ABI's semantics; [architecture.md](architecture.md), the implementation, whose sketches are normative; [formal-model.md](formal-model.md), the models' structure, the implementation's risk register and the models' mapping to the code; [decisions.md](decisions.md), the decision log. |
| [tasks.md](tasks.md) | The plan, in phases. Each task's status records what shipped and where the plan did not survive. |
| [tla/](tla/) | The TLA+ models, at three levels each refining the one before - gRPC in the abstract, the C ABI, the .NET binding - proved with TLAPS. |
| `packages/rust/armonik-transport-ffi/include/armonik_transport_ffi.h` | The ABI as a C host reads it, rendered from the crate's Rust, which is its reference. |
| [call-shapes.md](call-shapes.md) | Calls that declare a single request or a single response, and what the ABI, the engine and the binding do with it: decided, steps 1 and 2 of 5 built. Until each of its steps lands, every other document of the specification wins over it on what that step changes, but for `DotNetBinding.tla`, which follows at step 5. |

Where they disagree on how, the TLA+ models win over the design, and the design over tasks.md.
What V1 includes is requirements.md's criteria, and what it excludes is that document's
"Out of scope for V1".
