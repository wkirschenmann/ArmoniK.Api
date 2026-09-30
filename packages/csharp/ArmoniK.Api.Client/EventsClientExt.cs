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
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.Options;
using ArmoniK.Api.Common.Exceptions;
using ArmoniK.Api.gRPC.V1;
using ArmoniK.Api.gRPC.V1.Events;
using ArmoniK.Api.gRPC.V1.Results;
using ArmoniK.Utils;

using Grpc.Core;

using JetBrains.Annotations;

namespace ArmoniK.Api.Client
{
  /// <summary>
  ///   <see cref="Events.EventsClient" /> extensions methods
  /// </summary>
  [PublicAPI]
  public static class EventsClientExt
  {
    private static FiltersAnd ResultsFilter(string resultId)
      => new()
         {
           And =
           {
             new FilterField
             {
               Field = new ResultField
                       {
                         ResultRawField = new ResultRawField
                                          {
                                            Field = ResultRawEnumField.ResultId,
                                          },
                       },
               FilterString = new FilterString
                              {
                                Operator = FilterStringOperator.Equal,
                                Value    = resultId,
                              },
             },
           },
         };


    /// <summary>
    ///   Wait until the given results are completed
    /// </summary>
    /// <param name="client">gRPC result client</param>
    /// <param name="sessionId">The session ID in which the results are located</param>
    /// <param name="resultIds">A collection of results to wait for</param>
    /// <param name="cancellationToken">Token used to cancel the execution of the method</param>
    /// <exception cref="Exception">if a result is aborted</exception>
    /// <exception cref="RpcException">if six subscriptions in a row fail with no event between them</exception>
    [PublicAPI]
    [Obsolete("Use the overload with the bucket size and the parallelism")]
    public static Task WaitForResultsAsync(this Events.EventsClient client,
                                           string                   sessionId,
                                           ICollection<string>      resultIds,
                                           CancellationToken        cancellationToken = default)
      => client.WaitForResultsAsync(sessionId,
                                    resultIds,
                                    100,
                                    1,
                                    cancellationToken);


    /// <summary>
    ///   Wait until the given results are completed
    /// </summary>
    /// <param name="client">gRPC result client</param>
    /// <param name="sessionId">The session ID in which the results are located</param>
    /// <param name="resultIds">A collection of results to wait for</param>
    /// <param name="parallelism">Number of parallel threads to use. One bucket per thread.</param>
    /// <param name="bucket_size">Number of results Id to use to create the request to the event API</param>
    /// <param name="cancellationToken">Token used to cancel the execution of the method</param>
    /// <exception cref="Exception">if a result is aborted</exception>
    /// <exception cref="RpcException">if six subscriptions in a row fail with no event between them</exception>
    [PublicAPI]
    public static Task WaitForResultsAsync(this Events.EventsClient client,
                                           string                   sessionId,
                                           ICollection<string>      resultIds,
                                           int                      bucket_size       = 100,
                                           int                      parallelism       = 1,
                                           CancellationToken        cancellationToken = default)
      => client.WaitForResultsAsync(sessionId,
                                    resultIds,
                                    new SubscriptionRetry(),
                                    bucket_size,
                                    parallelism,
                                    cancellationToken);


    /// <summary>
    ///   Wait until the given results are completed, subscribing again after a failure as <paramref name="retry" />
    ///   says
    /// </summary>
    /// <param name="client">gRPC result client</param>
    /// <param name="sessionId">The session ID in which the results are located</param>
    /// <param name="resultIds">A collection of results to wait for</param>
    /// <param name="retry">How long to wait before subscribing again after a failure, and how many failures in a row end the wait</param>
    /// <param name="parallelism">Number of parallel threads to use. One bucket per thread.</param>
    /// <param name="bucket_size">Number of results Id to use to create the request to the event API</param>
    /// <param name="cancellationToken">Token used to cancel the execution of the method</param>
    /// <exception cref="Exception">if a result is aborted</exception>
    /// <exception cref="RpcException">
    ///   if <see cref="SubscriptionRetry.MaxAttempts" /> subscriptions in a row fail with no event between them
    /// </exception>
    /// <exception cref="ArgumentNullException"><paramref name="retry" /> is null</exception>
    /// <exception cref="ArgumentOutOfRangeException">a value of <paramref name="retry" /> is outside what it admits</exception>
    [PublicAPI]
    public static async Task WaitForResultsAsync(this Events.EventsClient client,
                                                 string                   sessionId,
                                                 ICollection<string>      resultIds,
                                                 SubscriptionRetry        retry,
                                                 int                      bucket_size       = 100,
                                                 int                      parallelism       = 1,
                                                 CancellationToken        cancellationToken = default)
    {
      var backoff = new Backoff(retry);

      var cts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);

      try
      {
        await resultIds.ToChunks(bucket_size)
                       .ParallelForEach(new ParallelTaskOptions
                                        {
                                          ParallelismLimit  = parallelism,
                                          CancellationToken = cts.Token,
                                        },
                                        async results =>
                                        {
                                          var resultsCompleted = new List<string>();
                                          var resultsNotFound  = new HashSet<string>(results);
                                          var retryCount       = 0;
                                          var bound            = backoff.First;
                                          var delay            = TimeSpan.Zero;
                                          while (resultsNotFound.Any() && !cts.IsCancellationRequested)
                                          {
                                            try
                                            {
                                              if (delay > TimeSpan.Zero)
                                              {
                                                await Task.Delay(delay,
                                                                 cts.Token)
                                                          .ConfigureAwait(false);
                                                delay = TimeSpan.Zero;
                                              }

                                              using var streamingCall = client.GetEvents(new EventSubscriptionRequest
                                                                                         {
                                                                                           SessionId = sessionId,
                                                                                           ReturnedEvents =
                                                                                           {
                                                                                             EventsEnum.ResultStatusUpdate,
                                                                                             EventsEnum.NewResult,
                                                                                           },
                                                                                           ResultsFilters = new Filters
                                                                                                            {
                                                                                                              Or =
                                                                                                              {
                                                                                                                resultsNotFound.Select(ResultsFilter),
                                                                                                              },
                                                                                                            },
                                                                                         },
                                                                                         cancellationToken: cancellationToken);
                                              await streamingCall.ResponseHeadersAsync.ConfigureAwait(false);

                                              while (await streamingCall.ResponseStream.MoveNext(cancellationToken))
                                              {
                                                // Only an event shows the subscription holds: a refused one answers its headers too.
                                                retryCount = 0;
                                                bound      = backoff.First;

                                                var resp = streamingCall.ResponseStream.Current;
                                                if (resp.UpdateCase == EventSubscriptionResponse.UpdateOneofCase.ResultStatusUpdate &&
                                                    resultsNotFound.Contains(resp.ResultStatusUpdate.ResultId))
                                                {
                                                  if (resp.ResultStatusUpdate.Status == ResultStatus.Completed)
                                                  {
                                                    resultsCompleted.Add(resp.ResultStatusUpdate.ResultId);
                                                    resultsNotFound.Remove(resp.ResultStatusUpdate.ResultId);
                                                    if (!resultsNotFound.Any())
                                                    {
                                                      break;
                                                    }
                                                  }
                                                  else if (resp.ResultStatusUpdate.Status == ResultStatus.Aborted)
                                                  {
                                                    throw new ResultAbortedException($"Result {resp.ResultStatusUpdate.ResultId} has been aborted",
                                                                                     resp.ResultStatusUpdate.ResultId,
                                                                                     resultsCompleted,
                                                                                     resultsNotFound);
                                                  }
                                                }

                                                if (resp.UpdateCase == EventSubscriptionResponse.UpdateOneofCase.NewResult &&
                                                    resultsNotFound.Contains(resp.NewResult.ResultId))
                                                {
                                                  if (resp.NewResult.Status == ResultStatus.Completed)
                                                  {
                                                    resultsCompleted.Add(resp.NewResult.ResultId);
                                                    resultsNotFound.Remove(resp.NewResult.ResultId);
                                                    if (!resultsNotFound.Any())
                                                    {
                                                      break;
                                                    }
                                                  }
                                                  else if (resp.NewResult.Status == ResultStatus.Aborted)
                                                  {
                                                    throw new ResultAbortedException($"Result {resp.NewResult.ResultId} has been aborted",
                                                                                     resp.NewResult.ResultId,
                                                                                     resultsCompleted,
                                                                                     resultsNotFound);
                                                  }
                                                }
                                              }
                                            }
                                            catch (OperationCanceledException) when (cts.Token.IsCancellationRequested)
                                            {
                                              break;
                                            }
                                            catch (RpcException)
                                            {
                                              retryCount += 1;
                                              if (retryCount >= backoff.MaxAttempts)
                                              {
                                                throw;
                                              }

                                              delay = backoff.Drawn(bound);
                                              bound = backoff.Grown(bound);
                                            }
                                          }
                                        });
        cts.Dispose();
      }
      catch
      {
        cts.Cancel();
        throw;
      }
    }

    /// <summary>A <see cref="SubscriptionRetry" />'s values, copied once and then checked.</summary>
    /// <remarks>Copied first, so that a caller changing the options during the wait can neither slip a value past the
    ///   checks nor change the wait.</remarks>
    private sealed class Backoff
    {
      // Task.Delay's ceiling.
      private static readonly TimeSpan LongestDelay = TimeSpan.FromMilliseconds(int.MaxValue);

      // Shared by every wait and every bucket, so locked: Random is not thread-safe.
      private static readonly Random Draws = new();

      private readonly double   jitter_;
      private readonly TimeSpan max_;
      private readonly double   multiplier_;

      internal Backoff(SubscriptionRetry retry)
      {
        if (retry is null)
        {
          throw new ArgumentNullException(nameof(retry));
        }

        MaxAttempts = retry.MaxAttempts;
        var initial = retry.InitialBackOff;
        multiplier_ = retry.BackoffMultiplier;
        max_        = retry.MaxBackOff;
        jitter_     = retry.Jitter;

        if (MaxAttempts < 1)
        {
          throw new ArgumentOutOfRangeException(nameof(retry),
                                                MaxAttempts,
                                                "MaxAttempts has to be at least 1");
        }

        if (initial < TimeSpan.Zero || initial > LongestDelay)
        {
          throw new ArgumentOutOfRangeException(nameof(retry),
                                                initial,
                                                $"InitialBackOff has to be between zero and {LongestDelay}");
        }

        if (max_ < TimeSpan.Zero || max_ > LongestDelay)
        {
          throw new ArgumentOutOfRangeException(nameof(retry),
                                                max_,
                                                $"MaxBackOff has to be between zero and {LongestDelay}");
        }

        // Negated, so that NaN, which compares false to everything, is refused here and below.
        if (!(multiplier_ > 0) || double.IsPositiveInfinity(multiplier_))
        {
          throw new ArgumentOutOfRangeException(nameof(retry),
                                                multiplier_,
                                                "BackoffMultiplier has to be greater than 0 and finite");
        }

        if (!(jitter_ >= 0 && jitter_ <= 1))
        {
          throw new ArgumentOutOfRangeException(nameof(retry),
                                                jitter_,
                                                "Jitter has to be between 0 and 1");
        }

        // gRFC A6 caps every bound, the first included.
        First = initial < max_
                  ? initial
                  : max_;
      }

      internal int MaxAttempts { get; }

      internal TimeSpan First { get; }

      internal TimeSpan Drawn(TimeSpan bound)
      {
        double draw;
        lock (Draws)
        {
          draw = Draws.NextDouble();
        }

        return TimeSpan.FromTicks((long)(bound.Ticks * (1 - jitter_ * draw)));
      }

      // Compared as a double, so a bound past what a TimeSpan holds is capped before it is converted.
      internal TimeSpan Grown(TimeSpan bound)
      {
        var grown = bound.Ticks * multiplier_;
        return grown >= max_.Ticks
                 ? max_
                 : TimeSpan.FromTicks((long)grown);
      }
    }
  }
}
