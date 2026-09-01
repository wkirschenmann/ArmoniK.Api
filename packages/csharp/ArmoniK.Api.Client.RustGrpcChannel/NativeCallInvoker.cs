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
  private readonly ulong channel_;

  internal NativeCallInvoker(ulong channel)
    => channel_ = channel;

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
    try
    {
      return call.ResponseAsync.GetAwaiter()
                 .GetResult();
    }
    catch (AggregateException aggregate) when (aggregate.InnerException is not null)
    {
      throw aggregate.InnerException;
    }
  }

  /// <inheritdoc />
  public override AsyncUnaryCall<TResponse> AsyncUnaryCall<TRequest, TResponse>(Method<TRequest, TResponse> method,
                                                                                string? host,
                                                                                CallOptions options,
                                                                                TRequest request)
  {
    var call = NativeCall.Start(channel_,
                                method.FullName,
                                options.Headers);

    CancellationTokenRegistration cancellation = default;
    Task<TResponse> response;
    try
    {
      if (options.CancellationToken.CanBeCanceled)
      {
        cancellation = options.CancellationToken.Register(call.Cancel);
      }

      var payload = new SerializedMessage();
      method.RequestMarshaller.ContextualSerializer(request,
                                                    payload);
      call.SendUnary(payload.Bytes);

      response = ReadAsync(call,
                           method);
    }
    catch
    {
      // Nothing was handed to the caller, so nothing else will ever release the call.
      cancellation.Dispose();
      call.Dispose();
      throw;
    }

    return new AsyncUnaryCall<TResponse>(response,
                                         call.ResponseHeadersAsync,
                                         () => Terminal(call),
                                         () => call.Trailers,
                                         () =>
                                         {
                                           cancellation.Dispose();
                                           call.Dispose();
                                         });
  }

  private static async Task<TResponse> ReadAsync<TRequest, TResponse>(NativeCall call,
                                                                      Method<TRequest, TResponse> method)
    where TRequest : class
    where TResponse : class
  {
    var message = await call.ReadUnaryAsync()
                            .ConfigureAwait(false);
    var response = method.ResponseMarshaller.ContextualDeserializer(new ReceivedMessage(message));

    // A unary call's answer is its message and its status together: a message followed by a
    // failing status is not a success, and grpc-dotnet's own callers rely on that.
    var status = await call.TerminalAsync.ConfigureAwait(false);
    if (status.StatusCode != StatusCode.OK)
    {
      throw new RpcException(status,
                             call.Trailers);
    }

    return response;
  }

  private static Status Terminal(NativeCall call)
    => call.TerminalAsync.IsCompleted
         ? call.TerminalAsync.GetAwaiter()
               .GetResult()
         : new Status(StatusCode.Unknown,
                      "the call has not ended yet");

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
