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
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   One generation of the native runtime, and the trampoline every event of it arrives on.
/// </summary>
/// <remarks>
///   Materialized and retired by <see cref="NativeRuntimeFactory" />, which is what holds the
///   leases. The teardown steps are separate because the model separates them, and because the
///   root may only be released once <c>ak_runtime_destroy</c> has returned.
/// </remarks>
internal sealed class NativeRuntime
{
  /// <summary>How long the teardown gives the runtime to stop before giving up on it.</summary>
  private static readonly TimeSpan ShutdownTimeout = TimeSpan.FromSeconds(30);

  private static readonly TimeSpan RoomPollInterval = TimeSpan.FromMilliseconds(2);

  /// <summary>
  ///   How long to wait between reads of the runtime's state once its last event has arrived.
  /// </summary>
  /// <remarks>
  ///   The gap it covers is a few instructions wide, and the state is read before the first wait,
  ///   so the ordinary teardown never sleeps here at all.
  /// </remarks>
  private static readonly TimeSpan QuiescePollInterval = TimeSpan.FromMilliseconds(1);

  /// <summary>
  ///   Rooted for the library's lifetime. A delegate marshalled to a function pointer is not kept
  ///   alive by the native side holding that pointer, so letting this be collected would leave the
  ///   library calling into a freed thunk.
  /// </summary>
  private static readonly NativeMethods.AkCallback Trampoline = OnEvent;

  private readonly TaskCompletionSource<bool> released_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private GCHandle self_;
  private readonly ulong handle_;

  private NativeRuntime(uint workerThreads,
                        ulong memoryCeiling)
  {
    self_ = GCHandle.Alloc(this);

    var config = new NativeMethods.AkRuntimeConfig
                 {
                   StructSize    = (uint)Marshal.SizeOf<NativeMethods.AkRuntimeConfig>(),
                   WorkerThreads = workerThreads,
                   MemoryCeiling = memoryCeiling,
                 };

    var status = NativeMethods.ak_runtime_create(ref config,
                                                 Trampoline,
                                                 GCHandle.ToIntPtr(self_),
                                                 out handle_);
    if (status != NativeMethods.AkStatus.Ok)
    {
      self_.Free();
      throw new InvalidOperationException($"the native runtime could not be created ({status})");
    }
  }

  internal ulong Handle
    => handle_;

  /// <summary>Creates a generation, after checking the library speaks the ABI this was built against.</summary>
  internal static NativeRuntime Create(uint workerThreads,
                                       ulong memoryCeiling)
  {
    var found = NativeMethods.ak_abi_version();
    if (found != NativeMethods.AbiVersion)
    {
      throw new InvalidOperationException($"the native library speaks ABI {found}, this binding speaks {NativeMethods.AbiVersion}");
    }

    return new NativeRuntime(workerThreads,
                             memoryCeiling);
  }

  /// <summary>
  ///   Waits until the ceiling admits another lend.
  /// </summary>
  /// <remarks>
  ///   The ceiling is the runtime's, so the wait is too: a call refused a buffer has no idea what
  ///   else is holding one. There is no event announcing room either, which is why this polls the
  ///   figure the header names for it.
  /// </remarks>
  internal async Task WaitForRoomAsync(CancellationToken token)
  {
    while (true)
    {
      token.ThrowIfCancellationRequested();
      await Task.Delay(RoomPollInterval,
                       token)
                .ConfigureAwait(false);

      if (NativeMethods.ak_runtime_memory_usage(handle_,
                                                out var usage) != NativeMethods.AkStatus.Ok
          || usage.Ceiling == 0
          || usage.BytesUsed < usage.Ceiling)
      {
        return;
      }
    }
  }

  /// <summary>
  ///   Stops, waits for quiescence, destroys, and only then releases the root.
  /// </summary>
  /// <remarks>
  ///   In that order, and here rather than at the caller: the root may be released only once
  ///   `ak_runtime_destroy` has returned, since until then a callback can still be in flight
  ///   carrying it. An ordering documented at two sites and enforced at neither is what this
  ///   replaces.
  /// </remarks>
  internal async Task RetireAsync()
  {
    NativeMethods.ak_runtime_begin_shutdown(handle_);
    await QuiescentAsync()
      .ConfigureAwait(false);
    Destroy();
    FreeRoot();
  }

  /// <summary>
  ///   Waits for the runtime's last event, and then for the runtime to publish that it has
  ///   stopped.
  /// </summary>
  /// <remarks>
  ///   Quiescence is reached by giving everything back, not by waiting for it, and each call's
  ///   drain is what does that. This waits only for the runtime's own word on it.
  ///   <para>
  ///     Two waits, because the event and the state are two moments and only the second one
  ///     permits the destroy. The runtime publishes AK_RUNTIME_QUIESCENT after its last callback
  ///     has returned - a callback runs on the runtime's own thread, so it can never be the thing
  ///     that reports that thread is finished - and this continuation is queued from inside that
  ///     callback. Destroying on the event alone therefore races the state it needs, and the
  ///     loser is refused with AK_STATUS_INVALID_STATE, which the factory latches for the life of
  ///     the process. The event is the notification; the state is the permission.
  ///   </para>
  /// </remarks>
  private async Task QuiescentAsync()
  {
    var waited = Stopwatch.StartNew();

    // The deadline is cancelled once it is not needed, so a host that opens and closes channels
    // in a loop does not leave one rooted timer per generation behind it.
    using (var deadline = new CancellationTokenSource())
    {
      var answered = await Task.WhenAny(released_.Task,
                                        Task.Delay(ShutdownTimeout,
                                                   deadline.Token))
                               .ConfigureAwait(false);
      deadline.Cancel();

      if (answered != released_.Task)
      {
        throw NotQuiescent();
      }
    }

    while (NativeMethods.ak_runtime_status(handle_) != NativeMethods.AkRuntimeState.Quiescent)
    {
      if (waited.Elapsed >= ShutdownTimeout)
      {
        throw NotQuiescent();
      }

      await Task.Delay(QuiescePollInterval)
                .ConfigureAwait(false);
    }
  }

  private InvalidOperationException NotQuiescent()
    => new($"the runtime did not quiesce within {ShutdownTimeout} ({NativeMethods.ak_runtime_status(handle_)})");

  /// <exception cref="InvalidOperationException">
  ///   Destroying is permitted only from quiescence, so a refusal leaves the threads up. Raised
  ///   rather than swallowed: nothing else would ever report it.
  /// </exception>
  private void Destroy()
  {
    var status = NativeMethods.ak_runtime_destroy(handle_);
    if (status != NativeMethods.AkStatus.Ok)
    {
      throw new InvalidOperationException($"the runtime refused to be destroyed ({status}, {NativeMethods.ak_runtime_status(handle_)})");
    }
  }

  /// <summary>Releases the root. Legal only once <see cref="Destroy" /> has returned.</summary>
  private void FreeRoot()
  {
    if (self_.IsAllocated)
    {
      self_.Free();
    }
  }

  private static unsafe void OnEvent(IntPtr runtimeCtx,
                                     IntPtr callCtx,
                                     IntPtr eventPtr)
  {
    var @event = (NativeMethods.AkEvent*)eventPtr;

    object? target;
    try
    {
      target = GCHandle.FromIntPtr(callCtx != IntPtr.Zero
                                     ? callCtx
                                     : runtimeCtx)
                       .Target;
    }
    catch
    {
      // A token this side no longer roots is a bug on this side, but the payload is the
      // library's to reclaim and dropping it here would strand the call for good.
      NativeMethods.ak_event_consumed(@event->Payload);
      return;
    }

    try
    {
      if (target is ICallSink call)
      {
        call.Publish(@event->Kind,
                     @event->Payload,
                     @event->StatusCode);

        // After the publish and not inside it: the root has to outlive every callback that
        // carries it, and the terminal's is the last one.
        if (@event->Kind == NativeMethods.AkEventKind.Status)
        {
          call.TerminalReturned();
        }

        return;
      }

      (target as NativeRuntime)?.OnRuntimeEvent(@event->Kind,
                                                @event->HostDebt);
    }
    catch
    {
      // Publishing a slot cannot fail, so only a bug reaches here - and unwinding into C is a
      // worse answer to a bug than dropping one event.
    }
  }

  private void OnRuntimeEvent(NativeMethods.AkEventKind kind,
                              NativeMethods.AkHostDebt debt)
  {
    // RESOURCES_RELEASED follows only when the host still owed something; when it owed nothing,
    // SHUTDOWN_COMPLETE is the last word and waiting for a second event would hang.
    if (kind == NativeMethods.AkEventKind.ResourcesReleased
        || (kind == NativeMethods.AkEventKind.ShutdownComplete && debt == NativeMethods.AkHostDebt.NothingToReturn))
    {
      released_.TrySetResult(true);
    }
  }
}
