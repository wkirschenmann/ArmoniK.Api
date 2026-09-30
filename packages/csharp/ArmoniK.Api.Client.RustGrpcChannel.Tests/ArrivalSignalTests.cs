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

using System.Threading.Tasks;

using NUnit.Framework;

using ArmoniK.Api.Client.RustGrpcChannel.Calls;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

[TestFixture]
public class ArrivalSignalTests
{
  [Test]
  public void AWaitTakenBeforeASetIsWoken()
  {
    var signal = new ArrivalSignal();
    var wait   = signal.Next();

    signal.Set();

    Assert.That(wait.IsCompleted,
                Is.True);
  }

  [Test]
  public void EveryWaitTakenBeforeASetIsWoken()
  {
    var signal = new ArrivalSignal();
    var first  = signal.Next();
    var second = signal.Next();

    signal.Set();

    Assert.That(first.IsCompleted && second.IsCompleted,
                Is.True);
  }

  /// <remarks>A kept wake-up would go to whichever wait came next, which is how one waiter takes
  /// what another needed.</remarks>
  [Test]
  public void ASetKeepsNothingForAWaitTakenAfterIt()
  {
    var signal = new ArrivalSignal();
    signal.Set();

    Assert.That(signal.Next()
                      .IsCompleted,
                Is.False);
  }

  [Test]
  public void AWaitTakenAfterASetWaitsForTheNextOne()
  {
    var signal = new ArrivalSignal();
    signal.Next();
    signal.Set();
    var wait = signal.Next();

    Assert.That(wait.IsCompleted,
                Is.False);

    signal.Set();

    Assert.That(wait.IsCompleted,
                Is.True);
  }
}
