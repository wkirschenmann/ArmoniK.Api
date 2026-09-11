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


using System;
using System.Threading.Tasks;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>A fixture whose tests run on an engine of their own.</summary>
///
/// One per test rather than one per fixture: the engine admits one runtime per process, so a test
/// that left one behind would be reported by the next test's `Create` rather than by its own, and
/// that is the report nobody can read. Given back in a teardown that asserts nothing, so a test
/// failing on its subject still leaves the process able to run the next one.
public abstract class RuntimeFixture
{
  private NativeRuntime? runtime_;

  /// <summary>This test's engine.</summary>
  protected NativeRuntime Runtime
    => runtime_ ?? throw new InvalidOperationException("this test has no runtime");

  /// <summary>What a fixture whose tests need the engine started differently overrides.</summary>
  protected virtual NativeRuntime Start()
    => NativeRuntime.Create();

  [SetUp]
  public void TakeARuntime()
    => runtime_ = Start();

  [TearDown]
  public Task GiveTheRuntimeBack()
    => RunOn(null);

  /// <summary>Starts this test's engine again with other options.</summary>
  /// <remarks>For a test whose subject is what the engine was started with. One runtime per
  /// process, so the current one goes first - and the fixture disposes whichever is current, so
  /// restarting leaves nothing behind either.</remarks>
  protected async Task<NativeRuntime> RestartAsync(uint workerThreads = 0,
                                                   ulong memoryCeiling = 0)
  {
    await RunOn(() => NativeRuntime.Create(workerThreads,
                                           memoryCeiling))
      .ConfigureAwait(false);
    return Runtime;
  }

  private async Task RunOn(Func<NativeRuntime>? next)
  {
    var going = runtime_;
    runtime_ = null;

    if (going is not null)
    {
      await going.DisposeAsync()
                 .ConfigureAwait(false);
    }

    runtime_ = next?.Invoke();
  }
}
