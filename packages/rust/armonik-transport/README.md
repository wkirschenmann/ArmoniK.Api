# armonik-transport

Transport layer for the [ArmoniK](https://github.com/aneoconsulting/ArmoniK) Rust client:
configuration parsing, TLS/mTLS, HTTP proxy tunnelling, the optional port-reuse connector for
Windows, and the retry policy for replaying a failed request.

This crate is factored out of [`armonik`](../armonik), which re-exports everything here at the same
paths it always used (`armonik::ClientConfig`, `armonik::client::RetryPolicy`, ...), so depending on
`armonik` directly is unaffected by this split. Depend on `armonik-transport` instead when you need
the connection layer without generated protobuf types or a `protoc`/`tonic-prost-build` build step —
this is what the native half of the `ArmoniK.Api.Client.Legacy` .NET Framework binding
(`armonik-transport-ffi`) does.

## Publishing

**This crate has to be published before `armonik`.** `armonik` depends on it by `path`, and a `path`
dependency cannot be published: `cargo publish` on `armonik` rewrites it into a version requirement
against the registry, so the version it names must already be there.

```sh
cargo publish -p armonik-transport   # first, and wait for the index to pick it up
cargo publish -p armonik
```

`armonik-transport-ffi` is never published — it is `publish = false`, built only as the native half of
a NuGet package.
