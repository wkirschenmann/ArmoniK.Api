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

    return call_.WriteAsync(marshaller_,
                            message);
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
