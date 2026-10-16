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

namespace ArmoniK.Api.Client.RustGrpcChannel.Calls;

/// <summary>The request stream of a client streaming or duplex call.</summary>
/// <remarks><c>IClientStreamWriter</c> promises one writer and no <c>CompleteAsync</c> beside a
/// pending write, so this holds no state of its own beyond having been closed - the call is where
/// a write waits for its acquittal.</remarks>
internal sealed class NativeRequestStream<TRequest, TResponse> : IClientStreamWriter<TRequest>
  where TRequest : class
  where TResponse : class
{
  private readonly NativeCall<TResponse> call_;
  private readonly Marshaller<TRequest>  marshaller_;

  private int closed_;

  internal NativeRequestStream(NativeCall<TResponse> call,
                               Marshaller<TRequest>  marshaller)
  {
    call_       = call;
    marshaller_ = marshaller;
  }

  /// <summary>Accepted and ignored: the ABI carries no per-write flag.</summary>
  public WriteOptions? WriteOptions { get; set; }

  public Task WriteAsync(TRequest message)
  {
    if (Volatile.Read(ref closed_) != 0)
    {
      throw new InvalidOperationException("the request stream is closed");
    }

    // The guard above stays synchronous - it names a rule the caller broke, and an `async` method
    // would deliver it through the task instead. The write is started here for the same reason:
    // the call refuses a second writer synchronously too, and wrapping the start would hand that
    // refusal to a task the caller who overlapped two writes may not be awaiting.
    return Written(call_.WriteAsync(marshaller_,
                                    message));
  }

  private async Task Written(Task written)
  {
    try
    {
      await written.ConfigureAwait(false);
    }
    catch (CallEnded)
    {
      // The engine refused the write because the call had already ended, and the caller hears the
      // status grpc-dotnet's call holds by then: the terminal a read consumed, Cancelled for a call
      // cancelled or disposed this side, otherwise the code of the terminal the engine published,
      // which it does for every call that ends whether or not anyone reads it.
      var terminal = call_.TerminalAsync;
      if (terminal.IsCompleted)
      {
        throw new RpcException(terminal.Result,
                               call_.Trailers);
      }

      if (call_.CancelledHere)
      {
        throw new RpcException(new Status(StatusCode.Cancelled,
                                          "the call was cancelled"));
      }

      var code = await call_.EndedWith.ConfigureAwait(false);
      throw new RpcException(new Status(code,
                                        "the call ended before the write"));
    }
  }

  /// <summary>Idempotent, because a caller disposing after completing is the ordinary path.</summary>
  public Task CompleteAsync()
  {
    if (Interlocked.Exchange(ref closed_,
                             1) == 0)
    {
      call_.HalfClose();
    }

    return Task.CompletedTask;
  }
}
