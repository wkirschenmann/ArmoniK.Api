# DESIGN — .NET ArmoniK Client on Native Rust gRPC Channel

## Introduction

These documents detail the technical decisions for each layer of the architecture defined in
[SPEC.MD](SPEC.MD), in response to the requirements in [requirements.md](requirements.md).

They establish the APIs, types, sequences, error contracts, and the foundations of the TLA+
formal model. Implementation choices (specific crates, internal algorithms) remain free as long
as they comply with the contracts described here.

Two numberings run through them, and they count different things. *Layers* 1 to 5 are the
components of the implementation, each with its sections in the documents below. *Levels* 0
to 2 are the TLA+ models, each refining the one before it; `L0!` and `L1!` are their instance
names. A level is not a layer, and "level 1" and "layer 1" do not name the same thing.

---

## Where each part is

The design is written in five documents, one register each, so that what binds the code
and what explains it cannot be mistaken for each other:

| Document | What it holds |
|----------|---------------|
| [contract.md](contract.md) | What layers 1 and 2 promise the code that uses them: the public types of `armonik-transport`, their errors, and the gRPC behaviour a caller relies on |
| [abi.md](abi.md) | The normative C ABI of layer 3: its principles, its entry points, the configuration document, the sequences a host follows |
| [architecture.md](architecture.md) | How the contract and the ABI are met: the engine's internals, the FFI crate's, the .NET binding of layer 4 and the integration of layer 5 |
| [formal-model.md](formal-model.md) | The TLA+ levels 0 to 2, what is proved of them, the implementation's risk register, and the table that maps each level-1 action to the code that performs it |
| [decisions.md](decisions.md) | The questions the design had to settle, decided or still open |

What binds the code is the contract, the ABI - whose declarations' reference is the header
the crate's Rust renders - and the implementation sketches, each of which names the action of
the formal model it realizes. The prose around them says why.

---

## References

- [gRPC over HTTP/2](https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md)
- [gRPC Retry Design](https://github.com/grpc/proposal/blob/master/A6-client-retries.md)
- [tower::Service](https://docs.rs/tower/latest/tower/trait.Service.html)
- [Grpc.Core.CallInvoker](https://grpc.github.io/grpc/csharp-dotnet/api/Grpc.Core.CallInvoker.html)
- [TLA+ Proof System](https://tla.msr-inria.inria.fr/tlaps/content/Home.html)
