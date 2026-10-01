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
using System.Collections.Concurrent;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

using ArmoniK.Api.Client.RustGrpcChannel.Interop;
using ArmoniK.Api.Client.RustGrpcChannel.Calls;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal enum ChannelDisposeState
{
  Active,
  Disposing,

  Released,
  Disposed,
}

/// <summary>A gRPC channel served by the native engine.</summary>
public sealed class NativeChannel : ChannelBase, IAsyncDisposable
{
  private readonly NativeRuntime runtime_;
  private readonly ulong handle_;
  private readonly int deliveryCredits_;

  private readonly ConcurrentDictionary<ICallSink, Task> live_ = new();

  private readonly TaskCompletionSource<bool> disposed_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private int disposing_;
  private volatile ChannelDisposeState state_ = ChannelDisposeState.Active;

  internal NativeChannel(NativeRuntime runtime,
                         string endpoint,
                         ChannelOptions options)
    : base(endpoint)
  {
    runtime_ = runtime;
    // Resolved by the runtime, so this and the engine size from one number.
    deliveryCredits_ = options.DeliveryCredits!.Value;

    // The endpoint is its own argument and never an option: it is the one value a channel cannot
    // be created without, so every option of the document has a default and `{}` would do.
    var named = Encoding.UTF8.GetBytes(endpoint);
    var json = options.Encode();

    unsafe
    {
      fixed (byte* pinnedEndpoint = named)
      fixed (byte* pinned = json)
      {
        var where = ak_bytes_in.Borrow(pinnedEndpoint,
                                       named.Length);
        var config = ak_bytes_in.Borrow(pinned,
                                        json.Length);

        ak_status status;
        fixed (ulong* created = &handle_)
        {
          status = NativeMethods.ak_channel_create(runtime.Handle,
                                                   where,
                                                   config,
                                                   created,
                                                   null);
        }

        if (status != ak_status.AK_STATUS_OK)
        {
          // Three answers, because this door means three things by a refusal, and the caller can
          // act on which. `InvalidArg` is what was handed over: an endpoint that is not UTF-8,
          // not a URI, or not one this engine dials, or a document it re-checks and refuses - a
          // different endpoint is worth trying. A stale or closed runtime is the runtime going
          // away under a creation that passed `RefuseIfGoingAway`, since the engine also stops
          // for reasons of its own, and retrying that is worth nothing. The status is named in
          // every message for whoever reads the trace rather than for the caller, who has the
          // type: a gate closed by a shutdown and a handle spent by a destroy are one answer here
          // and two different bugs to go and find.
          throw status switch
                {
                  ak_status.AK_STATUS_INVALID_ARG => new ArgumentException($"`{Safely(endpoint)}` or an option given with it was refused ({status})"),
                  ak_status.AK_STATUS_INVALID_STATE or ak_status.AK_STATUS_HANDLE_STALE =>
                    new ObjectDisposedException(nameof(NativeRuntime),
                                                $"the runtime was gone before `{Safely(endpoint)}` could be opened ({status})"),
                  _ => new InvalidOperationException($"`{Safely(endpoint)}` was refused ({status})"),
                };
        }
      }
    }
  }

  /// <summary>An endpoint as a message may carry it: its scheme, host and port, as written.</summary>
  /// <remarks>
  ///   An endpoint may carry `user:password@`, and the engine refusing one is exactly the case
  ///   that reaches the message above - so the string as given would put a password in the
  ///   caller's log. Read as text rather than by `Uri`, which takes the `localhost` of
  ///   `localhost:5000` for a scheme and, on .NET Framework, reads a port out of a trailing
  ///   newline. Everything up to the last `@` goes, because a password may contain a slash, so
  ///   when that `@` is in a path or a query, which the engine refuses anyway, what follows it is
  ///   shown as the host. A string with whitespace or a control character in it, or a port that
  ///   is not digits, is named rather than echoed.
  /// </remarks>
  internal static string Safely(string endpoint)
  {
    const string notAUri = "an endpoint that is not a URI";

    foreach (var character in endpoint)
    {
      if (char.IsWhiteSpace(character) || char.IsControl(character))
      {
        return notAUri;
      }
    }

    var separator = endpoint.IndexOf("://",
                                     StringComparison.Ordinal);
    var scheme = separator > 0 && IsScheme(endpoint.Substring(0,
                                                              separator))
                   ? endpoint.Substring(0,
                                        separator)
                   : null;
    var rest = scheme is null
                 ? endpoint
                 : endpoint.Substring(separator + 3);

    rest = rest.Substring(rest.LastIndexOf('@') + 1);
    var end = rest.IndexOfAny(AuthorityEnds);
    var authority = end < 0
                      ? rest
                      : rest.Substring(0,
                                       end);

    // After the last `]`, so an IPv6 host's own colons are not taken for a port's.
    var colon = authority.LastIndexOf(':');
    if (colon > authority.LastIndexOf(']') && !IsDigits(authority.Substring(colon + 1)))
    {
      return notAUri;
    }

    if (scheme is not null)
    {
      return $"{scheme}://{authority}";
    }

    return authority.Length > 0
             ? authority
             : notAUri;
  }

  private static bool IsDigits(string candidate)
  {
    foreach (var character in candidate)
    {
      if (character is < '0' or > '9')
      {
        return false;
      }
    }

    return candidate.Length > 0;
  }

  // `\` as well, which .NET's own parser takes for a `/` in an http URI.
  private static readonly char[] AuthorityEnds =
  {
    '/',
    '?',
    '#',
    '\\',
  };

  // RFC 3986: ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ).
  private static bool IsScheme(string candidate)
  {
    if (!IsAsciiLetter(candidate[0]))
    {
      return false;
    }

    foreach (var character in candidate)
    {
      if (!(IsAsciiLetter(character) || character is >= '0' and <= '9' or '+' or '-' or '.'))
      {
        return false;
      }
    }

    return true;
  }

  private static bool IsAsciiLetter(char character)
    => character is >= 'a' and <= 'z' or >= 'A' and <= 'Z';

  internal ChannelDisposeState DisposeState
    => state_;

  internal ak_channel_state NativeState
    => NativeMethods.ak_channel_status(handle_);

  /// <summary>The invoker a generated client calls through.</summary>
  public override CallInvoker CreateCallInvoker()
    => new NativeCallInvoker(this);

  internal NativeCall<TResponse> StartCall<TResponse>(string method,
                                                      Metadata? metadata,
                                                      Marshaller<TResponse> marshaller)
    where TResponse : class
  {
    if (Volatile.Read(ref disposing_) != 0)
    {
      throw new RpcException(new Status(StatusCode.Unavailable,
                                        "the channel is being disposed and takes no new calls"));
    }

    var call = NativeCall<TResponse>.Start(runtime_,
                                           handle_,
                                           deliveryCredits_,
                                           method,
                                           metadata,
                                           marshaller);
    // The settlement and not the response: a server stream has no single response, and what the
    // drain has to wait for is the terminal consumed with nothing owed either way.
    Track(call,
          call.Settled);
    return call;
  }

  private void Track(ICallSink call,
                     Task settled)
  {
    live_[call] = settled;
    _ = settled.ContinueWith(static (_,
                                     state) =>
                             {
                               var (tracked, key) = ((ConcurrentDictionary<ICallSink, Task>, ICallSink))state!;
                               tracked.TryRemove(key,
                                                 out Task? _);
                             },
                             (live_, call),
                             TaskContinuationOptions.ExecuteSynchronously);

    // Read again, because `StartCall`'s read and this line straddle the start: a disposal that
    // began between them drained a `live_` this call was not in yet.
    if (Volatile.Read(ref disposing_) != 0)
    {
      call.Cancel();
    }
  }

  /// <summary>Cancels what is still running, then releases this channel's handle.</summary>
  /// <remarks>It does not stop the engine: the runtime is the caller's own object and outlives
  /// every channel it made, so what ends here ends here.</remarks>
  public async ValueTask DisposeAsync()
  {
    if (Interlocked.Exchange(ref disposing_,
                             1) != 0)
    {
      await disposed_.Task.ConfigureAwait(false);
      return;
    }

    state_ = ChannelDisposeState.Disposing;

    try
    {
      while (!live_.IsEmpty)
      {
        // One snapshot, because `Keys` and `Values` are two of them and a call may leave between.
        var live = live_.ToArray();
        var settling = new Task[live.Length];
        for (var index = 0; index < live.Length; index++)
        {
          live[index]
            .Key.Cancel();
          settling[index] = live[index]
            .Value;
        }

        try
        {
          await Task.WhenAll(settling)
                    .ConfigureAwait(false);
        }
        catch
        {
        }

        // Removed here rather than left to the continuation `Track` registered. A task runs its
        // continuations in order, on the thread that completed it; when `Track` was preempted
        // between the add and the ContinueWith, the removal is registered behind this loop's own
        // await and cannot run while this loop is on that thread - and the loop would spin on a
        // `live_` nothing can empty. Exact because a key is added once and its task never
        // changes: what was just awaited is what is being removed.
        foreach (var entry in live)
        {
          live_.TryRemove(entry.Key,
                          out Task? _);
        }
      }

      NativeMethods.ak_channel_release(handle_);
      state_ = ChannelDisposeState.Released;

      runtime_.Forget(this);
      state_ = ChannelDisposeState.Disposed;
      disposed_.TrySetResult(true);
    }
    catch (Exception raised)
    {
      disposed_.TrySetException(raised);
      throw;
    }
  }

  /// <summary><see cref="ChannelBase" />'s shutdown, which is this channel's disposal.</summary>
  /// <remarks>
  ///   Two things `ChannelBase` tells an implementor it need not do, and this one does: it cancels
  ///   the calls still running, and it waits for them. Both are the engine's debt model rather
  ///   than a preference. A channel that let go of its handle while its calls still held payloads
  ///   and lent buffers would leave the runtime owed them, and a runtime owed anything never
  ///   reaches QUIESCENT - so there is no shutdown here that is cheaper than a disposal, only one
  ///   that would hide the cost until the runtime refused to go.
  ///   <para>
  ///     It costs a caller nothing the contract promised. That contract makes finishing the calls
  ///     the caller's own responsibility and says outright that shutting down with calls in flight
  ///     may change their outcome, so doing it for them narrows the ways to be surprised rather
  ///     than widening them.
  ///   </para>
  /// </remarks>
  protected override Task ShutdownAsyncCore()
    => DisposeAsync()
      .AsTask();
}
