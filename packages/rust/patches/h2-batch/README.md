# h2-batch

A patch of h2 0.4.19 that writes a stream's DATA frames in fewer, larger writes, and the recipe
that builds the engine against it. Only the patch is in this repository: the build takes h2 from
crates.io, patches a copy of it in the target directory, and points cargo at that copy for that
build alone. Every other build uses h2 as crates.io ships it.

## What the patch does

`h2-batch.patch` is three changes to h2's `src/`:

1. [hyperium/h2#903](https://github.com/hyperium/h2/pull/903), ported onto 0.4.19: the
   connection queues several DATA frames and writes them in one vectored write, rather than one
   frame per write. With one fix: a stream reset while one of its frames is half written, and
   released by the time the frame is, has the rest of that frame dropped. h2#903 as
   proposed resolves the released stream there, and the connection panics ("dangling store key").
2. One queued DATA frame may span several frames of the peer's largest size, written one after
   the other in the same vectored write, each with its own head and END_STREAM only on the last.
3. How many it may span is the connection's: `h2::with_frames_per_write(frames, future)` sets
   it for every connection whose codec is made while `future` is polled. A connection made
   outside it spans one frame, which is the behaviour of hyperium/h2#903 alone.

The engine sets it from `Http2.Send.FramesPerWrite`, and refuses a value above 1 from a build
without the patch.

Measured over TCP loopback, 16 MiB uploads at 16 frames per write: 15.3 to about 8.2 ms of
client CPU per call, and 1245 to 286 syscalls; a 4 MiB request goes out in 17 writes at 16
frames against 257 at 1. A part is spread over frames as far as its buffer's first contiguous
chunk reaches, which is the whole message for what tonic encodes: a buffer of several chunks
would take a write per chunk.

## What it changes besides the writes

- **Data after a reset.** A queued frame of N frames is handed to the writer whole, so a reset
  of its stream after that sends up to N - 1 DATA frames before the RST_STREAM, the last of them
  possibly carrying END_STREAM. Stock h2 sends at most the one frame in flight. RFC 9113 allows
  it, and it stays within flow control: the window was taken when the frame was handed over.
- **Control frames behind queued data.** A PING, SETTINGS acknowledgement, WINDOW_UPDATE or
  GOAWAY waits behind the DATA already queued. Stock h2 holds one DATA frame at a time;
  hyperium/h2#903 queues up to 512 parts, one per stream with data to send, each of N frames.

At N = 1 a reset sends no more than with stock h2, but a control frame still waits behind one
frame per active stream.

## Building with it

```sh
packages/rust/patches/h2-batch/build.sh build -p armonik-transport-ffi --release
packages/rust/patches/h2-batch/build.sh test -p armonik-transport --all-features
```

`build.sh` runs cargo with what follows it, after it has:

1. waited for its turn on the target directory, since every run there rewrites the same copy
   and the same Cargo.lock: the .NET binding builds once per target framework, in parallel;
2. read the h2 version and checksum Cargo.lock pins, and refused to go on unless it is 0.4.19,
   the version the patch is made against;
3. found `h2-0.4.19.crate` in cargo's cache, fetching it first if it is not there, and checked
   its sha256 against Cargo.lock;
4. extracted it into `target/h2-batch/h2-0.4.19` and applied the patch there, unless the copy
   there is already that crate with that patch, which spares rebuilding h2 and all above it;
5. run cargo with `--config patch.crates-io.h2.path=...` and `ARMONIK_H2_BATCH=1`, which tells
   armonik-transport's build script to compile what calls into the patch.

A path patch makes cargo rewrite Cargo.lock; `build.sh` puts it back when it exits. It needs
bash, tar, patch and sha256sum, which Git for Windows provides.

The .NET binding builds its native engine with it when the MSBuild property
`NativeEngineH2Batch` is `true`, which needs `bash` on the PATH to be Git Bash on Windows:

```sh
dotnet build -p:NativeEngineH2Batch=true
```

## Moving to another h2

The patch is made against 0.4.19 and `build.sh` refuses any other. For a new h2, apply it to that
version's `src/`, resolve what does not apply, and write the patch again with
`git diff --no-index` between the pristine and the patched `src/`, then change `base` in
`build.sh`.
