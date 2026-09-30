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

using System.Collections.Concurrent;
using System.Linq;
using System.Threading.Tasks;

using ArmoniK.Api.gRPC.V1;
using ArmoniK.Api.gRPC.V1.Events;

using Grpc.Core;

namespace ArmoniK.Api.Mock.Services;

[Counting]
public class Events : gRPC.V1.Events.Events.EventsBase
{
  /// <summary>A session whose subscriptions are refused before any event, as ArmoniK refuses one
  /// the caller may not make.</summary>
  public const string RefusedSessionId = "refused-session-id";

  /// <summary>A session whose subscriptions each send a NewResult for the first result they ask
  /// about and then fail with Unavailable, except every eighth for that result, which sends it
  /// Completed and ends.</summary>
  /// <remarks>Counted per result across the process, so waits on different results do not share a
  /// cycle.</remarks>
  public const string DroppedSessionId = "dropped-session-id";

  private static readonly ConcurrentDictionary<string, int> DroppedSubscriptions = new();

  /// <inheritdocs />
  [Count]
  public override async Task GetEvents(EventSubscriptionRequest                       request,
                                       IServerStreamWriter<EventSubscriptionResponse> responseStream,
                                       ServerCallContext                              context)
  {
    if (request.SessionId == RefusedSessionId)
    {
      throw new RpcException(new Status(StatusCode.PermissionDenied,
                                        "the subscription is refused"));
    }

    var dropped = request.SessionId == DroppedSessionId;
    var resultId = dropped
                     ? request.ResultsFilters?.Or.SelectMany(and => and.And)
                              .Select(field => field.FilterString?.Value)
                              .FirstOrDefault(value => value is not null) ?? "result-id"
                     : "result-id";
    var completes = dropped && DroppedSubscriptions.AddOrUpdate(resultId,
                                                                 1,
                                                                 (_,
                                                                  count) => count + 1) % 8 == 0;

    await responseStream.WriteAsync(new EventSubscriptionResponse
                                    {
                                      SessionId = "session-id",
                                      NewResult = new EventSubscriptionResponse.Types.NewResult
                                                  {
                                                    ResultId = resultId,
                                                    OwnerId  = "owner-id",
                                                    Status = completes
                                                               ? ResultStatus.Completed
                                                               : ResultStatus.Created,
                                                  },
                                    })
                        .ConfigureAwait(false);

    if (dropped && !completes)
    {
      throw new RpcException(new Status(StatusCode.Unavailable,
                                        "the subscription is dropped"));
    }
  }
}
