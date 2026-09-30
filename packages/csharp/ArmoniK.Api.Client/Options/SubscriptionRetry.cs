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

using ArmoniK.Utils.DocAttribute;

using JetBrains.Annotations;

namespace ArmoniK.Api.Client.Options
{
  /// <summary>
  ///   How <see cref="EventsClientExt" />'s WaitForResultsAsync subscribes again after a subscription fails
  /// </summary>
  /// <remarks>
  ///   After a failure, the next subscription waits for a delay no longer than a bound. The bound starts at
  ///   <see cref="InitialBackOff" /> and is multiplied by <see cref="BackoffMultiplier" /> after each failure, never
  ///   past <see cref="MaxBackOff" />, as gRPC's retry policy does. An event resets both the count of failures and the
  ///   bound. While <see cref="InitialBackOff" /> is zero every delay is zero, whatever the other values; the defaults
  ///   subscribe again at once.
  /// </remarks>
  [ExtractDocumentation("Options for the event subscriptions of WaitForResultsAsync")]
  [PublicAPI]
  public class SubscriptionRetry
  {
    /// <summary>
    ///   How many subscriptions in a row that fail with no event between them end the wait. At least 1.
    /// </summary>
    public int MaxAttempts { get; set; } = 6;

    /// <summary>
    ///   The bound on the delay before the first new subscription. Zero subscribes again at once.
    /// </summary>
    public TimeSpan InitialBackOff { get; set; } = TimeSpan.Zero;

    /// <summary>
    ///   What the bound is multiplied by after each failure. Greater than 0.
    /// </summary>
    public double BackoffMultiplier { get; set; } = 1.5;

    /// <summary>
    ///   The largest the bound may be, the first included.
    /// </summary>
    public TimeSpan MaxBackOff { get; set; } = TimeSpan.FromSeconds(5);

    /// <summary>
    ///   The share of each delay that is drawn at random, from 0 to 1. At 0 the delay is the whole bound; at 1 it is drawn
    ///   uniformly up to the bound, as gRPC's retry policy draws it.
    /// </summary>
    public double Jitter { get; set; } = 1;
  }
}
