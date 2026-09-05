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

internal sealed class NativeRuntime
{
  private static readonly TimeSpan ShutdownTimeout = TimeSpan.FromSeconds(30);

  private static readonly TimeSpan RoomPollInterval = TimeSpan.FromMilliseconds(2);

  private static readonly TimeSpan QuiescePollInterval = TimeSpan.FromMilliseconds(1);

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

  internal async Task WaitForRoomAsync(CancellationToken token)
  {
    while (true)
    {
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

  internal async Task RetireAsync()
  {
    NativeMethods.ak_runtime_begin_shutdown(handle_);
    await QuiescentAsync()
      .ConfigureAwait(false);
    Destroy();
    self_.Free();
  }

  private async Task QuiescentAsync()
  {
    var waited = Stopwatch.StartNew();

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

  private void Destroy()
  {
    var status = NativeMethods.ak_runtime_destroy(handle_);
    if (status != NativeMethods.AkStatus.Ok)
    {
      throw new InvalidOperationException($"the runtime refused to be destroyed ({status}, {NativeMethods.ak_runtime_status(handle_)})");
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
    }
  }

  private void OnRuntimeEvent(NativeMethods.AkEventKind kind,
                              NativeMethods.AkHostDebt debt)
  {
    if (kind == NativeMethods.AkEventKind.ResourcesReleased
        || (kind == NativeMethods.AkEventKind.ShutdownComplete && debt == NativeMethods.AkHostDebt.NothingToReturn))
    {
      released_.TrySetResult(true);
    }
  }
}
