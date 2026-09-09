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
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.RustGrpcChannel.Calls;
using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel;

internal sealed class NativeRuntime
{
  private static readonly TimeSpan RoomPollInterval = TimeSpan.FromMilliseconds(2);

  private static readonly TimeSpan JoinPollInterval = TimeSpan.FromMilliseconds(1);

  // The engine holds this pointer for as long as the runtime lives, and a delegate is only as
  // alive as the reference kept to it.
  private static readonly NativeMethods.AkCallback Trampoline = OnEvent;

  private GCHandle self_;
  private readonly ulong handle_;

  /// <summary>What the runtime says about its own shutdown, as a wake-up and not as news.</summary>
  /// <remarks>The two events a runtime carries rather than a call - SHUTDOWN_COMPLETE and
  /// RESOURCES_RELEASED - and the state is what they mean, read again after the wait.</remarks>
  private readonly ArrivalSignal announced_ = new();

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

    // Only once both have answered. A runtime that refused to be destroyed still holds this
    // pointer, and freeing the root would hand its next callback whatever the slot is reused for.
    self_.Free();
  }

  /// <summary>Waits for the one fact the header names as the guarantee.</summary>
  ///
  /// The runtime's own shutdown has two parts and this waits on each in the way that part admits.
  /// The functional shutdown - every channel closed, every callback returned, every payload given
  /// back - ends with SHUTDOWN_COMPLETE, and the engine stores GRPC_STOPPED before it emits it, so
  /// a wait on the event reads a state that has already moved. What follows is a thread outside
  /// tokio stopping the workers, and QUIESCENT is that thread having finished: no event can
  /// announce it, because whatever emitted the announcement would be running on the thread whose
  /// end it reports. So the long part is waited on and the join is polled.
  ///
  /// <para>
  ///   And there is no deadline, for the reason the engine gives for dropping its own: this state
  ///   is what permits `ak_runtime_destroy` and unloading the library, so patience is the host's
  ///   to spend and no timer can make the promise true early. Giving up on one would call a slow
  ///   shutdown a broken runtime, and since the engine admits one runtime per process that verdict
  ///   is the process's for good. The two failures that are failures answer here: the engine
  ///   saying quiescence is impossible, and a destroy it refuses.
  /// </para>
  private async Task QuiescentAsync()
  {
    while (true)
    {
      var state = NativeMethods.ak_runtime_status(handle_);

      switch (state)
      {
        case NativeMethods.AkRuntimeState.Quiescent:
          return;

        // The engine says it will never quiesce - a teardown thread it could not start - so this
        // is the one wait that ends without the fact it waited for.
        case NativeMethods.AkRuntimeState.FailedUnquiesced:
          throw NotQuiescent(state);

        case NativeMethods.AkRuntimeState.Running:
        case NativeMethods.AkRuntimeState.GrpcStopping:
          // Latched, so an announcement that landed between the read above and this wait is not
          // lost, and the state above is what is believed rather than the event.
          await announced_.WaitAsync()
                          .ConfigureAwait(false);
          break;

        default:
          await Task.Delay(JoinPollInterval)
                    .ConfigureAwait(false);
          break;
      }
    }
  }

  private static InvalidOperationException NotQuiescent(NativeMethods.AkRuntimeState state)
    => new($"the runtime cannot quiesce ({state})");

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

    var taken = false;
    try
    {
      if (target is ICallSink call)
      {
        taken = call.Publish(@event->Kind,
                             @event->Payload,
                             @event->StatusCode);

        if (@event->Kind == NativeMethods.AkEventKind.Status)
        {
          call.TerminalReturned();
        }
      }

      else if (target is NativeRuntime runtime)
      {
        // A runtime-level event carries no payload, and what it carries instead is that the state
        // has moved: the engine stores the new one before it emits the event. So this is a
        // wake-up, and the waiter reads the state for itself.
        runtime.announced_.Set();
      }
    }
    catch
    {
    }
    finally
    {
      // Anything the ring did not take is given back here: a root that no longer names a sink, a
      // publish that threw before storing, an event of the runtime itself. What is owed and never
      // returned is what the shutdown then waits for, forever. A payload the ring did take is the
      // reader's to return, and returning it twice would free it under the reader.
      if (!taken)
      {
        NativeMethods.ak_event_consumed(@event->Payload);
      }
    }
  }
}
