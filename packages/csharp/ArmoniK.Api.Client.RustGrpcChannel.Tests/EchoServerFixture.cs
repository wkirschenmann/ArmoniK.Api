// This file is part of the ArmoniK project
//
// Copyright (C) ANEO, 2021-2026. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License")
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>A fixture whose tests call the echo server, started once for all of them.</summary>
///
/// One server per fixture rather than one per test: starting it costs a process, and every test
/// here opens its own channel over it, on the engine the base fixture gives each test.
public abstract class EchoServerFixture : RuntimeFixture
{
  private EchoServerProcess? server_;

  /// <summary>Where it listens, as this engine needs it: plain HTTP/2.</summary>
  protected string Endpoint { get; private set; } = string.Empty;

  [OneTimeSetUp]
  public void StartTheEchoServer()
  {
    server_  = EchoServerProcess.Start();
    Endpoint = server_.Endpoint;
  }

  [OneTimeTearDown]
  public void StopTheEchoServer()
    => server_?.Dispose();

  protected static Echo.EchoClient Client(NativeChannel channel)
    => new(channel.CreateCallInvoker());
}
