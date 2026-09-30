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
using System.Threading;
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

  /// <remarks>
  ///   A waiter that takes its wait, looks at a counter and awaits only while it has not moved,
  ///   against a thread that moves it, sets, and waits for the waiter to have seen it before the
  ///   next round. Each set is the only one for its count, so a single lost wake-up leaves both
  ///   waiting on each other.
  /// </remarks>
  [Test]
  public async Task AWaiterThatLooksAfterItsWaitSeesEverySet()
  {
    const int rounds = 20_000;
    var       signal = new ArrivalSignal();
    // Not disposed: a waiter a lost wake-up left parked may still release it once the test has
    // failed.
    var       seen   = new SemaphoreSlim(0);
    var       count  = 0;

    var waiter = Task.Run(async () =>
                          {
                            var last = 0;
                            while (last < rounds)
                            {
                              var arrival = signal.Next();
                              var now     = Volatile.Read(ref count);
                              if (now == last)
                              {
                                await arrival.ConfigureAwait(false);
                                continue;
                              }

                              last = now;
                              seen.Release();
                            }
                          });

    var lost = await Task.Run(() =>
                              {
                                for (var round = 1; round <= rounds; round++)
                                {
                                  Interlocked.Increment(ref count);
                                  signal.Set();
                                  if (!seen.Wait(TimeSpan.FromSeconds(10)))
                                  {
                                    return round;
                                  }
                                }

                                return 0;
                              })
                         .ConfigureAwait(false);

    Assert.That(lost,
                Is.Zero,
                "the waiter missed the set of this round");
    await waiter.ConfigureAwait(false);
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
