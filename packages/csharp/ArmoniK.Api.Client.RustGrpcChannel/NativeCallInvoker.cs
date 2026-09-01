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

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   Routes the generated stubs' calls to the native engine.
/// </summary>
/// <remarks>
///   Unary only for now: the three streaming cardinalities throw rather than pretend, so a caller
///   that reaches one learns it here instead of at the first message that never arrives.
/// </remarks>
public sealed class NativeCallInvoker : CallInvoker
{
  private readonly ulong runtime_;
  private readonly ulong channel_;

  internal NativeCallInvoker(ulong runtime,
                             ulong channel)
  {
    runtime_ = runtime;
    channel_ = channel;
  }

  /// <inheritdoc />
  public override TResponse BlockingUnaryCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                   string? host,
                                                                   CallOptions options,
                                                                   TRequest request)
  {
    using var call = AsyncUnaryCall(method,
                                    host,
                                    options,
                                    request);
    return call.ResponseAsync.GetAwaiter()
               .GetResult();
  }

  /// <inheritdoc />
  public override AsyncUnaryCall<TResponse> AsyncUnaryCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                string? host,
                                                                                CallOptions options,
                                                                                TRequest request)
  {
    var call = NativeCall<TResponse>.Start(runtime_,
                                           channel_,
                                           method.FullName,
                                           options.Headers,
                                           method.ResponseMarshaller);
    call.CancelWith(options.CancellationToken);

    return new AsyncUnaryCall<TResponse>(AnswerAsync(call,
                                                     method.RequestMarshaller,
                                                     request,
                                                     options.CancellationToken),
                                         call.ResponseHeadersAsync,
                                         () => Ended(call)
                                           .Status,
                                         () => Ended(call)
                                           .Trailers,
                                         call.Dispose);
  }

  /// <summary>
  ///   Sends, then answers. The drain starts first and is always awaited: it is what gives the
  ///   library its payloads back, and a call whose payloads never come back is never reclaimed.
  /// </summary>
  private static async Task<TResponse> AnswerAsync<TRequest, TResponse>(NativeCall<TResponse> call,
                                                                        Marshaller<TRequest> marshaller,
                                                                        TRequest request,
                                                                        CancellationToken token)
    where TRequest : class
    where TResponse : class
  {
    var drained = call.RunAsync();
    try
    {
      await call.SendUnaryAsync(marshaller,
                                request,
                                token)
                .ConfigureAwait(false);
    }
    catch
    {
      call.Cancel();
      try
      {
        await drained.ConfigureAwait(false);
      }
      catch
      {
        // The send's failure is the one worth reporting; the terminal only follows from it.
      }

      throw;
    }

    return await drained.ConfigureAwait(false);
  }

  private static (Status Status, Metadata Trailers) Ended<TResponse>(NativeCall<TResponse> call)
    where TResponse : class
  {
    if (!call.TerminalAsync.IsCompleted)
    {
      throw new InvalidOperationException("the call has not ended yet");
    }

    var trailers = new Metadata();
    foreach (var entry in call.Trailers)
    {
      trailers.Add(entry);
    }

    return (call.TerminalAsync.GetAwaiter()
                .GetResult(), trailers);
  }

  /// <inheritdoc />
  public override AsyncServerStreamingCall<TResponse> AsyncServerStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                    string? host,
                                                                                                    CallOptions options,
                                                                                                    TRequest request)
    => throw Unsupported(method.Type);

  /// <inheritdoc />
  public override AsyncClientStreamingCall<TRequest, TResponse> AsyncClientStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                              string? host,
                                                                                                              CallOptions options)
    => throw Unsupported(method.Type);

  /// <inheritdoc />
  public override AsyncDuplexStreamingCall<TRequest, TResponse> AsyncDuplexStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                              string? host,
                                                                                                              CallOptions options)
    => throw Unsupported(method.Type);

  private static RpcException Unsupported(MethodType type)
    => new(new Status(StatusCode.Unimplemented,
                      $"this invoker carries unary calls; {type} is not implemented yet"));
}
