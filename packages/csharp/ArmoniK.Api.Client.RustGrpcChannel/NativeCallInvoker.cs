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

public sealed class NativeCallInvoker : CallInvoker
{
  private readonly NativeChannel channel_;

  internal NativeCallInvoker(NativeChannel channel)
    => channel_ = channel;

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

  public override AsyncUnaryCall<TResponse> AsyncUnaryCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                string? host,
                                                                                CallOptions options,
                                                                                TRequest request)
  {
    MustCarryNoDeadline(options);

    var call = channel_.StartCall(method.FullName,
                                 options.Headers,
                                 method.ResponseMarshaller);
    call.CancelWith(options.CancellationToken);

    var answered = AnswerAsync(call,
                               method.RequestMarshaller,
                               request);

    return new AsyncUnaryCall<TResponse>(answered,
                                         static state => ((NativeCall<TResponse>)state).ResponseHeadersAsync,
                                         static state => EndedStatus((NativeCall<TResponse>)state),
                                         static state => EndedTrailers((NativeCall<TResponse>)state),
                                         static state => ((NativeCall<TResponse>)state).Cancel(),
                                         call);
  }

  private static async Task<TResponse> AnswerAsync<TRequest, TResponse>(NativeCall<TResponse> call,
                                                                        Marshaller<TRequest> marshaller,
                                                                        TRequest request)
    where TRequest : class
    where TResponse : class
  {
    var drained = call.Drained;
    try
    {
      await call.SendUnaryAsync(marshaller,
                                request)
                .ConfigureAwait(false);
    }
    catch
    {
      try
      {
        await drained.ConfigureAwait(false);
      }
      catch
      {
      }

      throw;
    }

    return await drained.ConfigureAwait(false);
  }

  private static Status EndedStatus<TResponse>(NativeCall<TResponse> call)
    where TResponse : class
  {
    MustHaveEnded(call);
    return call.TerminalAsync.GetAwaiter()
               .GetResult();
  }

  private static Metadata EndedTrailers<TResponse>(NativeCall<TResponse> call)
    where TResponse : class
  {
    MustHaveEnded(call);

    var trailers = new Metadata();
    foreach (var entry in call.Trailers)
    {
      trailers.Add(entry);
    }

    return trailers;
  }

  private static void MustHaveEnded<TResponse>(NativeCall<TResponse> call)
    where TResponse : class
  {
    if (!call.TerminalAsync.IsCompleted)
    {
      throw new InvalidOperationException("the call has not ended yet");
    }
  }

  public override AsyncServerStreamingCall<TResponse> AsyncServerStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                    string? host,
                                                                                                    CallOptions options,
                                                                                                    TRequest request)
    => throw Unsupported(method.Type);

  public override AsyncClientStreamingCall<TRequest, TResponse> AsyncClientStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                              string? host,
                                                                                                              CallOptions options)
    => throw Unsupported(method.Type);

  public override AsyncDuplexStreamingCall<TRequest, TResponse> AsyncDuplexStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                              string? host,
                                                                                                              CallOptions options)
    => throw Unsupported(method.Type);

  private static void MustCarryNoDeadline(in CallOptions options)
  {
    if (options.Deadline is { } deadline && deadline != DateTime.MaxValue)
    {
      throw new RpcException(new Status(StatusCode.Unimplemented,
                                        "this invoker carries no deadline: the C ABI has no field for one, so it could be honoured here and never reach the server"));
    }
  }

  private static RpcException Unsupported(MethodType type)
    => new(new Status(StatusCode.Unimplemented,
                      $"this invoker carries unary calls; {type} is not implemented yet"));
}
