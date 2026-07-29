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
