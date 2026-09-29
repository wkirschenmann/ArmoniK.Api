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

using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class QuiescenceTests
{
  /// <summary>A shutdown that fails without announcing it ends the wait with an exception.</summary>
  /// <remarks>
  ///   The engine stores FAILED_UNQUIESCED and emits nothing, so a wait woken by announcements
  ///   alone would wait for good, and the disposal with it. Driven by hand because the engine
  ///   fails a shutdown only when something inside it breaks.
  /// </remarks>
  [Test]
  public async Task AShutdownThatFailsWithoutAnnouncingItEndsTheWait()
  {
    var reads   = 0;
    var silence = new TaskCompletionSource<bool>().Task;

    var waiting = NativeRuntime.QuiescentAsync(() => ++reads < 3
                                                       ? NativeMethods.AkRuntimeState.GrpcStopping
                                                       : NativeMethods.AkRuntimeState.FailedUnquiesced,
                                               () => silence);

    var ended = await Task.WhenAny(waiting,
                                   Task.Delay(TimeSpan.FromSeconds(10)))
                          .ConfigureAwait(false);

    Assert.That(ended,
                Is.SameAs(waiting),
                "the wait outlived the failure");
    Assert.ThrowsAsync<InvalidOperationException>(async () => await waiting.ConfigureAwait(false));
  }
}
