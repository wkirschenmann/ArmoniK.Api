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
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

using Microsoft.Extensions.Configuration;

using ArmoniK.Api.Client.RustGrpcChannel.Calls;
using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>The native engine, and the channels made from it.</summary>
///
/// One per process, because the engine admits one and says so: `ak_runtime_create` refuses while
/// another lives. That is a fact about the library and not a policy of this type, which is why a
/// second <see cref="Create" /> answers at once instead of waiting for the first to go.
///
/// <para>
///   Its lifetime is the caller's, declared: what it makes, it disposes. A channel cannot outlive
///   the engine that serves it, and rather than leave that to an order of disposal this type
///   keeps what it made and takes it down with itself.
/// </para>
public sealed class NativeRuntime : IAsyncDisposable
{
  private static readonly TimeSpan RoomPollInterval = TimeSpan.FromMilliseconds(2);

  private static readonly TimeSpan JoinPollInterval = TimeSpan.FromMilliseconds(1);

  private static readonly TimeSpan FailurePollInterval = TimeSpan.FromMilliseconds(100);

  // The engine holds this pointer for as long as the runtime lives, and a delegate is only as
  // alive as the reference kept to it.
  private static readonly unsafe NativeMethods.ak_runtime_create_callback_delegate Trampoline = OnEvent;

  private GCHandle self_;
  private readonly ulong handle_;

  /// <summary>The channels this runtime made and has not yet seen go.</summary>
  /// <remarks>Behind a lock rather than in a concurrent set, because what has to hold is that a
  /// channel is never added after the disposal has swept: under the lock, either the creation
  /// wins and the sweep finds it, or the creation reads the disposal and refuses. A concurrent
  /// set would need a second read afterwards, and a channel added between the two would hold a
  /// handle nobody closes.</remarks>
  private readonly object gate_ = new();

  private readonly HashSet<NativeChannel> channels_ = new();

  private readonly TaskCompletionSource<bool> disposed_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private bool disposing_;

  /// <summary>What the runtime says about its own shutdown, as a wake-up and not as news.</summary>
  /// <remarks>The two events a runtime carries rather than a call - SHUTDOWN_COMPLETE and
  /// RESOURCES_RELEASED - and the state is what they mean, read again after the wait.</remarks>
  private readonly ArrivalSignal announced_ = new();

  private NativeRuntime(uint workerThreads,
                        ulong memoryCeiling)
  {
    self_ = GCHandle.Alloc(this);

    var config = new ak_runtime_config
                 {
                   struct_size    = (uint)Marshal.SizeOf<ak_runtime_config>(),
                   worker_threads = workerThreads,
                   memory_ceiling = memoryCeiling,
                 };

    ak_status status;
    unsafe
    {
      fixed (ulong* created = &handle_)
      {
        status = NativeMethods.ak_runtime_create(&config,
                                                 Trampoline,
                                                 (void*)GCHandle.ToIntPtr(self_),
                                                 created);
      }
    }

    if (status != ak_status.AK_STATUS_OK)
    {
      self_.Free();
      throw new InvalidOperationException($"the native runtime could not be created ({status})");
    }
  }

  internal ulong Handle
    => handle_;

  /// <summary>The deepest delivery window a channel may ask for.</summary>
  /// <remarks>Every call of the channel allocates a ring of the next power of two above it, so a
  /// window is paid per call in memory whether or not the peer ever fills it: this one is 65536
  /// slots, two megabytes in a 64-bit process. There is no answer here for what a host should
  /// want - the bound exists because the engine imposes none that sizes anything, so this one is
  /// the binding's.</remarks>
  public const int MaxDeliveryCredits = 1 << 15;

  /// <summary>The delivery window a channel gets when its options name none.</summary>
  /// <remarks>
  ///   Resolved into the document a channel sends, so the engine is never left to apply its own -
  ///   which is what keeps the ring this side sizes and the credits that side grants the same
  ///   number. One, the smallest window the option admits: a host that asks for nothing holds at
  ///   most one payload of a call and its terminal status.
  /// </remarks>
  public const int DefaultDeliveryCredits = 1;

  /// <summary>The section a channel's options are read from when a caller names none.</summary>
  public const string SettingSection = "RustGrpcChannel";

  /// <summary>Starts the engine, which the caller owns until it disposes it.</summary>
  /// <param name="workerThreads">How many threads the engine runs on; 0 leaves it its own.</param>
  /// <param name="memoryCeiling">What it may lend for messages at once; 0 leaves it its own.</param>
  /// <exception cref="RustEngineMissingException">The engine could not be loaded.</exception>
  /// <exception cref="InvalidOperationException">
  ///   The library speaks another ABI, or a runtime already lives in this process.
  /// </exception>
  public static NativeRuntime Create(uint workerThreads = 0,
                                     ulong memoryCeiling = 0)
  {
    int found;
    try
    {
      found = NativeMethods.ak_abi_version();
    }
    catch (DllNotFoundException absent)
    {
      throw RustEngineMissingException.For(absent);
    }

    if (found != NativeMethods.AK_ABI_VERSION)
    {
      throw new InvalidOperationException($"the native library speaks ABI {found}, this binding speaks {NativeMethods.AK_ABI_VERSION}");
    }

    return new NativeRuntime(workerThreads,
                             memoryCeiling);
  }

  /// <summary>Opens a channel with the options a configuration carries.</summary>
  /// <param name="endpoint">Where the channel connects, as the engine's own argument.</param>
  /// <param name="configuration">What the options are read from.</param>
  /// <param name="key">The section holding them.</param>
  /// <exception cref="ArgumentNullException"><paramref name="configuration" /> is null.</exception>
  /// <exception cref="InvalidOperationException">
  ///   <paramref name="key" /> names no section, or the section holds a key no option matches.
  /// </exception>
  /// <exception cref="ArgumentOutOfRangeException">An option is outside what is admitted.</exception>
  /// <remarks>
  ///   Required rather than optional: a caller who names a section meant to configure this, and a
  ///   misspelled name that quietly gave the engine's defaults would be a channel nobody
  ///   configured. <see cref="Channel(string,int)" /> is how to ask for the defaults.
  ///
  ///   The same argument one level down is what binds the section strictly. The engine refuses an
  ///   option it does not know in the document it is handed, so a key dropped here would be the
  ///   one door of the two that answers a misspelling with a working channel.
  /// </remarks>
  public NativeChannel Channel(string endpoint,
                               IConfiguration configuration,
                               string key = SettingSection)
  {
    return Channel(endpoint,
                   OptionsFrom(configuration,
                               key));
  }

  /// <summary>The options a configuration's section carries, bound strictly.</summary>
  /// <param name="configuration">What they are read from.</param>
  /// <param name="key">The section holding them.</param>
  /// <exception cref="ArgumentNullException"><paramref name="configuration" /> is null.</exception>
  /// <exception cref="InvalidOperationException">
  ///   <paramref name="key" /> names no section, or the section holds a key no option matches.
  /// </exception>
  /// <remarks>Its own method because reading a document reaches nothing native: a caller may
  /// check what a configuration says without an engine, and a test may too.</remarks>
  public static ChannelOptions OptionsFrom(IConfiguration configuration,
                                           string key = SettingSection)
  {
    if (configuration is null)
    {
      throw new ArgumentNullException(nameof(configuration));
    }

    var options = configuration.GetRequiredSection(key)
                               .Get<ChannelOptions>(binder => binder.ErrorOnUnknownConfiguration = true);

    return options ?? throw new InvalidOperationException($"{key} carries no options");
  }

  /// <summary>Opens a channel with a delivery window, and the engine's defaults elsewhere.</summary>
  /// <param name="endpoint">Where the channel connects.</param>
  /// <param name="deliveryCredits">How many of a call's payloads the host may hold at once, the terminal status aside.</param>
  /// <exception cref="ArgumentOutOfRangeException">The window is outside what is admitted.</exception>
  /// <exception cref="ArgumentException">The engine dials no such endpoint.</exception>
  /// <exception cref="ObjectDisposedException">This runtime is going away.</exception>
  /// <exception cref="InvalidOperationException">The engine refused for a reason of its own.</exception>
  public NativeChannel Channel(string endpoint,
                               int deliveryCredits = DefaultDeliveryCredits)
    => Channel(endpoint,
               new ChannelOptions
               {
                 DeliveryCredits = deliveryCredits,
               });

  /// <summary>Opens a channel this runtime serves, and keeps it until it is disposed.</summary>
  /// <param name="endpoint">Where the channel connects.</param>
  /// <param name="options">What the channel is opened with, read once and never written to.</param>
  /// <exception cref="ArgumentNullException"><paramref name="options" /> is null.</exception>
  /// <exception cref="ArgumentOutOfRangeException">An option is outside what is admitted.</exception>
  /// <exception cref="ArgumentException">
  ///   The engine dials no such endpoint, or refuses an option given with it.
  /// </exception>
  /// <exception cref="ObjectDisposedException">This runtime is going away.</exception>
  /// <exception cref="InvalidOperationException">The engine refused for a reason of its own.</exception>
  public NativeChannel Channel(string endpoint,
                               ChannelOptions options)
  {
    if (options is null)
    {
      throw new ArgumentNullException(nameof(options));
    }

    // One read of the caller's instance: what a channel sizes its rings from and what it sends
    // the engine are the same number only if nothing can set it in between.
    var settled = new ChannelOptions(options)
                  {
                    DeliveryCredits = options.DeliveryCredits ?? DefaultDeliveryCredits,
                  };

    // The schema's bounds, then this binding's own tighter one. Both are checked here rather
    // than left to the engine, which answers a bad document with a status naming no option.
    settled.Validate();
    RefuseAWindowNoRingCanHold(settled.DeliveryCredits);

    lock (gate_)
    {
      RefuseIfGoingAway();

      // Opened under the lock, so a disposal cannot run beside a creation: one of the two is
      // second, and whichever it is reads what the first did.
      var channel = new NativeChannel(this,
                                      endpoint,
                                      settled);
      channels_.Add(channel);
      return channel;
    }
  }

  /// <summary>What a channel says when it has let go of its own handle.</summary>
  internal void Forget(NativeChannel channel)
  {
    lock (gate_)
    {
      channels_.Remove(channel);
    }
  }

  /// <summary>Disposes what this runtime made, then stops the engine.</summary>
  /// <remarks>The channels first and this last, because a channel outliving its engine is a
  /// handle into a library that may have been unloaded. Disposing a channel twice is a no-op, so
  /// a caller that disposed its own and then this one is the ordinary path and not a race.
  /// </remarks>
  /// <exception cref="InvalidOperationException">
  ///   The engine failed, and the runtime can be neither quiesced nor destroyed.
  /// </exception>
  public async ValueTask DisposeAsync()
  {
    bool mine;
    lock (gate_)
    {
      mine       = !disposing_;
      disposing_ = true;
    }

    if (!mine)
    {
      await disposed_.Task.ConfigureAwait(false);
      return;
    }

    try
    {
      while (true)
      {
        NativeChannel[] open;
        lock (gate_)
        {
          open = channels_.ToArray();
          channels_.Clear();
        }

        if (open.Length == 0)
        {
          break;
        }

        // Out of the set before they are disposed rather than after: a channel whose disposal
        // threw would otherwise be asked again on the next turn, for ever. The loop turns again
        // because a channel disposing itself may have been added to the set by a creation that
        // held the lock while the snapshot above was taken.
        await Task.WhenAll(open.Select(channel => channel.DisposeAsync()
                                                         .AsTask()))
                  .ConfigureAwait(false);
      }

      await RetireAsync()
        .ConfigureAwait(false);

      disposed_.TrySetResult(true);
    }
    catch (Exception raised)
    {
      disposed_.TrySetException(raised);
      throw;
    }
  }

  /// <summary>Read under <c>gate_</c> by every caller, which is what makes it a decision.</summary>
  private void RefuseIfGoingAway()
  {
    if (disposing_)
    {
      throw new ObjectDisposedException(nameof(NativeRuntime),
                                        "the runtime is being disposed and opens no new channel");
    }
  }

  // Checked wherever a window arrives, so which door a caller came through does not decide
  // which bound applies.
  private static void RefuseAWindowNoRingCanHold(int? deliveryCredits)
  {
    if (deliveryCredits > MaxDeliveryCredits)
    {
      throw new ArgumentOutOfRangeException(nameof(ChannelOptions.DeliveryCredits),
                                            deliveryCredits,
                                            $"a delivery window is at most {MaxDeliveryCredits} here: every call of the channel allocates a ring of the next power of two above it");
    }
  }

  /// <summary>The ABI the loaded library speaks.</summary>
  /// <exception cref="RustEngineMissingException">The engine could not be loaded.</exception>
  public static int LibraryAbiVersion
  {
    get
    {
      try
      {
        return NativeMethods.ak_abi_version();
      }
      catch (DllNotFoundException absent)
      {
        throw RustEngineMissingException.For(absent);
      }
    }
  }

  internal async Task WaitForRoomAsync(CancellationToken token)
  {
    while (true)
    {
      await Task.Delay(RoomPollInterval,
                       token)
                .ConfigureAwait(false);

      if (HasRoom(handle_))
      {
        return;
      }
    }
  }

  // Out of WaitForRoomAsync, because the usage is read through its address and an async method
  // may not take one.
  private static unsafe bool HasRoom(ulong runtime)
  {
    ak_memory_usage usage;
    return NativeMethods.ak_runtime_memory_usage(runtime,
                                                 &usage) != ak_status.AK_STATUS_OK || usage.ceiling == 0 ||
           usage.bytes_used < usage.ceiling;
  }

  private async Task RetireAsync()
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
  /// back - ends with SHUTDOWN_COMPLETE, and the engine stores GRPC_STOPPED once its callback has
  /// returned, so a wait the event wakes may still read STOPPING and reads again within 100 ms.
  /// What follows is a thread outside tokio stopping the workers, and QUIESCENT is that thread
  /// having finished: no event can announce it, because whatever emitted the announcement would be
  /// running on the thread whose end it reports. So the long part is waited on by its event and
  /// read again every 100 ms, because a shutdown that fails announces nothing, and the join is
  /// polled.
  ///
  /// <para>
  ///   And there is no deadline, for the reason the engine gives for dropping its own: this state
  ///   is what permits `ak_runtime_destroy` and unloading the library, so patience is the host's
  ///   to spend and no timer can make the promise true early. Giving up on one would call a slow
  ///   shutdown a broken runtime, and since the engine admits one runtime per process that verdict
  ///   is the process's for good. The two failures that are failures answer here: the engine
  ///   saying quiescence is impossible, and a destroy it refuses.
  /// </para>
  private Task QuiescentAsync()
    => QuiescentAsync(() => NativeMethods.ak_runtime_status(handle_),
                      announced_.Next);

  /// <summary>The wait itself, over the state it reads and the announcement it wakes on.</summary>
  internal static async Task QuiescentAsync(Func<ak_runtime_state> status,
                                            Func<Task>                         announced)
  {
    while (true)
    {
      // Taken before the read, so an announcement that lands between the two wakes the wait below.
      var announcement = announced();
      var state        = status();

      switch (state)
      {
        case ak_runtime_state.AK_RUNTIME_QUIESCENT:
          return;

        // The engine says it will never quiesce - a shutdown task that died, a teardown thread it
        // could not start - so this wait ends without the fact it waited for.
        case ak_runtime_state.AK_RUNTIME_FAILED_UNQUIESCED:
          throw NotQuiescent(state);

        case ak_runtime_state.AK_RUNTIME_RUNNING:
        case ak_runtime_state.AK_RUNTIME_GRPC_STOPPING:
          // The state above is what is believed rather than the event. The timer is for the
          // failure: the engine stores it and announces nothing.
          await Task.WhenAny(announcement,
                             Task.Delay(FailurePollInterval))
                    .ConfigureAwait(false);
          break;

        default:
          await Task.Delay(JoinPollInterval)
                    .ConfigureAwait(false);
          break;
      }
    }
  }

  private static InvalidOperationException NotQuiescent(ak_runtime_state state)
    => new($"the runtime cannot quiesce ({state})");

  private void Destroy()
  {
    var status = NativeMethods.ak_runtime_destroy(handle_);
    if (status != ak_status.AK_STATUS_OK)
    {
      throw new InvalidOperationException($"the runtime refused to be destroyed ({status}, {NativeMethods.ak_runtime_status(handle_)})");
    }
  }

  internal static unsafe void OnEvent(void*     runtimeCtx,
                                      void*     callCtx,
                                      ak_event* @event)
  {
    object? target;
    try
    {
      target = GCHandle.FromIntPtr((IntPtr)(callCtx != null
                                              ? callCtx
                                              : runtimeCtx))
                       .Target;
    }
    catch
    {
      NativeMethods.ak_event_consumed(@event->payload);
      return;
    }

    var call  = target as ICallSink;
    var taken = false;
    try
    {
      if (call is not null)
      {
        taken = call.Publish(@event->kind,
                             @event->payload,
                             @event->status_code);
      }

      else if (target is NativeRuntime runtime)
      {
        // A runtime-level event carries no payload, only that the state is moving: the engine
        // stores STOPPED once this callback has returned. So this is a wake-up, and the waiter
        // reads the state for itself, again later if it still reads STOPPING.
        runtime.announced_.Set();
      }
    }
    catch
    {
      // Nothing may unwind into the engine: an exception crossing this callback is undefined on
      // its side of the ABI.
    }
    finally
    {
      // Anything the ring did not take is given back here: a root that no longer names a sink, a
      // publish that threw before storing, an event of the runtime itself. What is owed and never
      // returned is what the shutdown then waits for, forever. A payload the ring did take is the
      // reader's to return, and returning it twice would free it under the reader.
      if (!taken)
      {
        NativeMethods.ak_event_consumed(@event->payload);
      }

      // The terminal is the call's last callback, so its root goes with it whatever the publish
      // did: kept, it would hold the call for the life of the process.
      if (call is not null && @event->kind == ak_event_kind.AK_EVENT_STATUS)
      {
        call.TerminalReturned();
      }
    }
  }
}
