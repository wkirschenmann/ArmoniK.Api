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

using ArmoniK.Api.Client.RustGrpcChannel.Calls;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal sealed class NativeCallInvoker : CallInvoker
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
    var call = Started(method,
                       host,
                       options);

    // Started before the request is sent, so the reader is draining while the send is in flight
    // and a terminal that arrives first has somebody to collect it.
    var answered = AnswerAsync(call,
                               call.SingleAsync(),
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
                                                                        Task<TResponse> drained,
                                                                        Marshaller<TRequest> marshaller,
                                                                        TRequest request)
    where TRequest : class
    where TResponse : class
  {
    try
    {
      await call.SendUnaryAsync(marshaller,
                                request)
                .ConfigureAwait(false);
    }
    catch (CallEnded)
    {
      // The engine refused the send because the call had already ended, so the send is not what
      // went wrong and its status would say nothing. What ended it is the terminal, and a caller
      // who cancelled has to read `Cancelled` here rather than a fault of the binding.
      return await drained.ConfigureAwait(false);
    }
    catch
    {
      // Awaited so the call settles before this returns, and its failure dropped: what the caller
      // has to see is the send that failed, not the cancellation it caused.
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
  {
    var call = Started(method,
                       host,
                       options);

    // The one request goes without being awaited here: this cardinality hands the reader back to
    // the caller, and a send that fails ends the call itself, so what the caller learns is the
    // terminal that follows rather than a fault from a task nobody holds. Observed all the same,
    // because an unobserved one would surface on the finalizer thread.
    var sent = call.SendUnaryAsync(method.RequestMarshaller,
                                   request);
    _ = sent.ContinueWith(static settled => _ = settled.Exception,
                          TaskContinuationOptions.OnlyOnFaulted);

    return new AsyncServerStreamingCall<TResponse>(new NativeResponseStream<TResponse>(call),
                                                   static state => ((NativeCall<TResponse>)state).ResponseHeadersAsync,
                                                   static state => EndedStatus((NativeCall<TResponse>)state),
                                                   static state => EndedTrailers((NativeCall<TResponse>)state),
                                                   static state => ((NativeCall<TResponse>)state).Cancel(),
                                                   call);
  }

  public override AsyncClientStreamingCall<TRequest, TResponse> AsyncClientStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                              string? host,
                                                                                                              CallOptions options)
  {
    var call = Started(method,
                       host,
                       options);

    // The same reduction the unary path takes, for the same reason: this cardinality answers
    // exactly once, whatever it sent to be answered.
    return new AsyncClientStreamingCall<TRequest, TResponse>(new NativeRequestStream<TRequest, TResponse>(call,
                                                                                                          method.RequestMarshaller),
                                                             call.SingleAsync(),
                                                             static state => ((NativeCall<TResponse>)state).ResponseHeadersAsync,
                                                             static state => EndedStatus((NativeCall<TResponse>)state),
                                                             static state => EndedTrailers((NativeCall<TResponse>)state),
                                                             static state => ((NativeCall<TResponse>)state).Cancel(),
                                                             call);
  }

  public override AsyncDuplexStreamingCall<TRequest, TResponse> AsyncDuplexStreamingCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                                              string? host,
                                                                                                              CallOptions options)
  {
    var call = Started(method,
                       host,
                       options);

    // Both halves of the same call, and nothing between them: a write waits for its own
    // acquittal, which the engine delivers off the ring, and a read takes the ring, so the two
    // touch no shared state and need no order between them.
    return new AsyncDuplexStreamingCall<TRequest, TResponse>(new NativeRequestStream<TRequest, TResponse>(call,
                                                                                                          method.RequestMarshaller),
                                                             new NativeResponseStream<TResponse>(call),
                                                             static state => ((NativeCall<TResponse>)state).ResponseHeadersAsync,
                                                             static state => EndedStatus((NativeCall<TResponse>)state),
                                                             static state => EndedTrailers((NativeCall<TResponse>)state),
                                                             static state => ((NativeCall<TResponse>)state).Cancel(),
                                                             call);
  }

  /// <summary>Starts the call every cardinality starts the same way.</summary>
  /// <remarks>One place for the refusals, because a call option this invoker cannot honour has to
  /// be refused by all four or by none: the one that let it through would be the one a caller
  /// happened to use.</remarks>
  private NativeCall<TResponse> Started<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                             string? host,
                                                             in CallOptions options)
    where TRequest : class
    where TResponse : class
  {
    MustCarryNoHost(host);
    MustCarryNothingElseUnhonoured(options);

    var call = channel_.StartCall(method.FullName,
                                  options.Headers,
                                  method.ResponseMarshaller,
                                  options.Deadline);
    call.CancelWith(options.CancellationToken);
    return call;
  }

  /// <summary>Refuses the call options this invoker cannot act on.</summary>
  /// <remarks>Refused rather than ignored, which is the whole point: credentials the caller
  /// attached and this invoker drops are a request that goes out without the identity the caller
  /// believes it carries, and the answer - Unauthenticated, or worse, served anonymously - names
  /// the server rather than the binding. Grpc.Core itself refuses call credentials on an insecure
  /// channel, which is all this engine speaks.</remarks>
  private static void MustCarryNothingElseUnhonoured(in CallOptions options)
  {
    if (options.Credentials is not null)
    {
      throw new RpcException(new Status(StatusCode.Unimplemented,
                                        "this invoker carries no call credentials: the interceptor would never run, and the call would go out without them"));
    }

    if (options.PropagationToken is not null)
    {
      throw new RpcException(new Status(StatusCode.Unimplemented,
                                        "this invoker carries no propagation token: it holds a parent call's deadline and cancellation, and this invoker reads neither out of it"));
    }
  }

  /// <summary>Refuses a per-call authority, which this invoker cannot send.</summary>
  /// <remarks>The `host` argument every `CallInvoker` override takes overrides the channel's
  /// authority for one call, and nothing here reads it: the endpoint crosses the ABI once, at the
  /// channel. Refused for the same reason as the credentials above - a call sent to an authority
  /// other than the one asked for is answered by the wrong server, and the answer names that
  /// server rather than the binding that redirected it.</remarks>
  private static void MustCarryNoHost(string? host)
  {
    if (!string.IsNullOrEmpty(host))
    {
      throw new RpcException(new Status(StatusCode.Unimplemented,
                                        $"this invoker carries no per-call host: `{host}` would be dropped and the call sent to the channel's endpoint"));
    }
  }
}
