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

using Grpc.Core;

using NUnit.Framework;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>
///   <c>CallOptions.WithWaitForReady</c>: a call that waits for a server that is down rather than
///   fail <c>Unavailable</c>, as grpc-dotnet's does.
/// </summary>
[TestFixture]
public class WaitForReadyTests : RuntimeFixture
{
  /// <summary>One attempt, so that a call that fails does so at once rather than after a retry's
  /// backoff.</summary>
  private static ChannelOptions OneAttempt()
    => new()
       {
         Grpc = new GrpcOptions
                {
                  Retry = new RetryOptions.None(),
                },
       };

  private static Echo.EchoClient Client(NativeChannel channel)
    => new(channel.CreateCallInvoker());

  private static int PortOf(string endpoint)
    => new Uri(endpoint).Port;

  /// <summary>The call's outcome, bounded so that a wait that never ends fails the test rather
  /// than hanging it.</summary>
  private static async Task<T> Within<T>(Task<T> task)
  {
    var limit = Task.Delay(TimeSpan.FromSeconds(60));
    if (await Task.WhenAny(task,
                           limit)
                  .ConfigureAwait(false) != task)
    {
      throw new TimeoutException("the call did not end");
    }

    return await task.ConfigureAwait(false);
  }

  [Test]
  public async Task ACallThatWaitsGoesOutOnceTheServerIsUp()
  {
    var endpoint = ClosedPort.Endpoint();
    await using var channel = Runtime.Channel(endpoint,
                                              OneAttempt());
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "late",
                },
                new CallOptions().WithWaitForReady());

    await Task.Delay(300)
              .ConfigureAwait(false);
    Assert.That(call.ResponseAsync.IsCompleted,
                Is.False,
                "the call waits where it would fail");

    using var server = EchoServerProcess.Start(PortOf(endpoint));

    var reply = await Within(call.ResponseAsync)
                  .ConfigureAwait(false);
    Assert.That(reply.Text,
                Is.EqualTo("late"));
  }

  [Test]
  public async Task ACallThatDoesNotWaitFailsUnavailable()
  {
    await using var channel = Runtime.Channel(ClosedPort.Endpoint(),
                                              OneAttempt());
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "nobody",
                });

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await Within(call.ResponseAsync)
                                                                 .ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.Unavailable));
  }

  [Test]
  public async Task ACallThatWaitsEndsDeadlineExceededAtItsDeadline()
  {
    await using var channel = Runtime.Channel(ClosedPort.Endpoint(),
                                              OneAttempt());
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "nobody",
                },
                new CallOptions(deadline: DateTime.UtcNow.AddMilliseconds(500)).WithWaitForReady());

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await Within(call.ResponseAsync)
                                                                 .ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.DeadlineExceeded));
  }

  [Test]
  public async Task ACallThatWaitsEndsCancelledWhenItIsCancelled()
  {
    await using var channel = Runtime.Channel(ClosedPort.Endpoint(),
                                              OneAttempt());
    using var cancellation = new CancellationTokenSource();
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "nobody",
                },
                new CallOptions(cancellationToken: cancellation.Token).WithWaitForReady());

    await Task.Delay(300)
              .ConfigureAwait(false);
    Assert.That(call.ResponseAsync.IsCompleted,
                Is.False);
    cancellation.Cancel();

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await Within(call.ResponseAsync)
                                                                 .ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }

  /// <summary>The flag is read where every cardinality starts, so a streaming call waits too.</summary>
  [Test]
  public async Task AServerStreamingCallThatWaitsEndsDeadlineExceededAtItsDeadline()
  {
    await using var channel = Runtime.Channel(ClosedPort.Endpoint(),
                                              OneAttempt());
    using var call = Client(channel)
      .Fan(new EchoRequest
           {
             Text = "nobody",
           },
           new CallOptions(deadline: DateTime.UtcNow.AddMilliseconds(500)).WithWaitForReady());

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await Within(call.ResponseStream.MoveNext(CancellationToken.None))
                                                                 .ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.DeadlineExceeded));
  }

  [Test]
  public async Task ACallThatWaitsEndsCancelledWhenItsChannelIsDisposed()
  {
    // Disposed twice, once by the test and once on the way out, so that a failure before the first
    // is the test's and not the teardown's.
    await using var channel = Runtime.Channel(ClosedPort.Endpoint(),
                                              OneAttempt());
    using var call = Client(channel)
      .SayAsync(new EchoRequest
                {
                  Text = "nobody",
                },
                new CallOptions().WithWaitForReady());

    await Task.Delay(300)
              .ConfigureAwait(false);
    await channel.DisposeAsync()
                 .ConfigureAwait(false);

    var thrown = Assert.ThrowsAsync<RpcException>(async () => await Within(call.ResponseAsync)
                                                                 .ConfigureAwait(false));
    Assert.That(thrown!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }
}
