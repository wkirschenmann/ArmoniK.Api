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
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.RustGrpcChannel.Calls;
using ArmoniK.Api.Client.RustGrpcChannel.Interop;

using Microsoft.Extensions.Logging;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>The native engine, and the channels made from it.</summary>
///
/// One per process, because the engine admits one and says so: `ak_runtime_create` refuses while
/// another lives. That is a fact about the library and not a policy of this type, which is why a
/// second <see cref="Create(ulong,ulong,ILoggerFactory)" /> answers at once instead of waiting for the first to go.
///
/// <para>
///   Its lifetime is the caller's, declared: what it makes, it disposes. A channel cannot outlive
///   the engine that serves it, and rather than leave that to an order of disposal this type
///   keeps what it made and takes it down with itself.
/// </para>
public sealed class NativeRuntime : IAsyncDisposable
{
  private static readonly TimeSpan JoinPollInterval = TimeSpan.FromMilliseconds(1);

  private static readonly TimeSpan FailurePollInterval = TimeSpan.FromMilliseconds(100);

  // The engine holds this pointer for as long as the runtime lives, and a delegate is only as
  // alive as the reference kept to it.
  private static readonly unsafe NativeMethods.ak_runtime_create_callback_delegate Trampoline = OnEvent;

  // The same function, as the type the configuration's entry point declares.
  private static readonly unsafe NativeMethods.ak_runtime_create_from_callback_delegate TrampolineFrom = OnEvent;

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

  // What the engine's logs go to, when the runtime was given a logger factory.
  private readonly EngineLog? log_;

  /// <summary>What asks the engine for a runtime, handed the context its callbacks carry.</summary>
  private unsafe delegate ak_status Creating(void*     context,
                                            void*     logCallback,
                                            void*     logContext,
                                            ulong*    created,
                                            ak_error* error);

  private unsafe NativeRuntime(ILoggerFactory? loggerFactory,
                               Creating        create)
  {
    self_ = GCHandle.Alloc(this);
    log_  = loggerFactory is null
              ? null
              : new EngineLog(loggerFactory);

    // Published before the engine can call back, so that a failure of the first events is reported.
    log_?.Publish();

    ak_status status;
    ak_error  error = default;
    try
    {
      fixed (ulong* created = &handle_)
      {
        status = create((void*)GCHandle.ToIntPtr(self_),
                        log_ is null
                          ? null
                          : EngineLog.Trampoline,
                        log_ is null
                          ? null
                          : log_.Context,
                        created,
                        &error);
      }
    }
    catch
    {
      Abandon();
      throw;
    }

    if (status != ak_status.AK_STATUS_OK)
    {
      var refusal = error.Take();
      Abandon();
      throw new InvalidOperationException($"the native runtime could not be created ({status}): {refusal}");
    }
  }

  /// <summary>Lets go of what a creation that failed held.</summary>
  /// <remarks>After the engine has returned, nothing reaches the log: what the creation logged
  /// before it was refused is written out first.</remarks>
  private void Abandon()
  {
    self_.Free();
    log_?.Close();
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

  /// <summary>Starts the engine, which the caller owns until it disposes it.</summary>
  /// <param name="memoryCeiling">
  ///   The bytes of messages, sent and received, it holds before work waits: a call stops reading
  ///   and a send waits for room. 0 leaves it its own.
  /// </param>
  /// <param name="memoryHardCeiling">
  ///   The bytes past which it stops: a received message that would pass them ends its call with
  ///   <c>RESOURCE_EXHAUSTED</c>. 0 is a quarter above the first threshold in force -
  ///   <paramref name="memoryCeiling" />, or the engine's own when that is 0 - and less than that
  ///   threshold is refused.
  /// </param>
  /// <param name="loggerFactory">
  ///   Where the engine's logs go, the engine's target as the category, or none. Written from a
  ///   thread of its own: a provider is never called on the engine's. The engine's default filter
  ///   selects what reaches it: its own events at information, the libraries' at warning. The factory
  ///   has to outlive the runtime, which writes to it until it is disposed.
  /// </param>
  /// <exception cref="RustEngineMissingException">The engine could not be loaded.</exception>
  /// <exception cref="InvalidOperationException">
  ///   The library speaks another ABI, a runtime already lives in this process, or the second
  ///   threshold is below the first.
  /// </exception>
  public static unsafe NativeRuntime Create(ulong           memoryCeiling     = 0,
                                            ulong           memoryHardCeiling = 0,
                                            ILoggerFactory? loggerFactory     = null)
  {
    RefuseAnotherAbi();

    return new NativeRuntime(loggerFactory,
                             (context,
                              logCallback,
                              logContext,
                              created,
                              error) =>
                             {
                               var config = new ak_runtime_config
                                            {
                                              struct_size         = (uint)Marshal.SizeOf<ak_runtime_config>(),
                                              memory_ceiling      = memoryCeiling,
                                              memory_hard_ceiling = memoryHardCeiling,
                                              log_callback        = logCallback,
                                              log_ctx             = logContext,
                                            };
                               return NativeMethods.ak_runtime_create(&config,
                                                                      Trampoline,
                                                                      context,
                                                                      created,
                                                                      error);
                             });
  }

  /// <summary>Starts the engine with the options its configuration's sources state, read by the engine now.</summary>
  /// <param name="configuration">Where the options are read from, in order.</param>
  /// <param name="loggerFactory">
  ///   Where the engine's logs go, the engine's target as the category, or none. Written from a
  ///   thread of its own: a provider is never called on the engine's. The engine's own filter
  ///   selects what reaches it, <c>Logging.Filter</c> of the runtime's options. The factory has to outlive
  ///   the runtime, which writes to it until it is disposed.
  /// </param>
  /// <exception cref="ArgumentNullException"><paramref name="configuration" /> is null.</exception>
  /// <exception cref="InvalidOperationException">
  ///   A source is refused - a file that does not exist or does not parse, a value that does not
  ///   fit its key, the environment with an empty prefix - the message naming the source and the key's
  ///   path, or the engine refused as <see cref="Create(ulong,ulong,ILoggerFactory)" /> does.
  /// </exception>
  /// <exception cref="RustEngineMissingException">The engine could not be loaded.</exception>
  /// <remarks>
  ///   The sources are read by the engine alone, so a channel reads back from the engine the delivery
  ///   window its options and the sources settle between them, and sizes its rings from that.
  /// </remarks>
  public static NativeRuntime Create(NativeConfiguration configuration,
                                     ILoggerFactory?     loggerFactory = null)
  {
    if (configuration is null)
    {
      throw new ArgumentNullException(nameof(configuration));
    }

    return Create(configuration.Prefix,
                  configuration.Sources,
                  loggerFactory);
  }

  /// <summary>Starts the engine from the sources it reads, in order, under <paramref name="prefix" />.</summary>
  /// <param name="prefix">The sources' prefix; empty takes everything.</param>
  /// <param name="sources">Each source's kind and value.</param>
  /// <param name="loggerFactory">Where the engine's logs go, or none.</param>
  private static unsafe NativeRuntime Create(string                                             prefix,
                                             IReadOnlyList<(ak_source_kind Kind, byte[] Value)> sources,
                                             ILoggerFactory?                                    loggerFactory)
  {
    RefuseAnotherAbi();

    // One array for the prefix and every source's value, so that one pin covers them all.
    var named  = Encoding.UTF8.GetBytes(prefix);
    var values = new byte[named.Length + sources.Sum(source => source.Value.Length)];
    var starts = new int[sources.Count];
    named.CopyTo(values,
                 0);
    var at = named.Length;
    for (var index = 0; index < sources.Count; index++)
    {
      starts[index] = at;
      sources[index]
        .Value.CopyTo(values,
                      at);
      at += sources[index].Value.Length;
    }

    var listed = new ak_config_source[sources.Count];

    return new NativeRuntime(loggerFactory,
                             (context,
                              logCallback,
                              logContext,
                              created,
                              error) =>
                             {
                               fixed (byte* pinned = values)
                               fixed (ak_config_source* first = listed)
                               {
                                 for (var index = 0; index < listed.Length; index++)
                                 {
                                   listed[index] = new ak_config_source
                                                   {
                                                     kind = (uint)sources[index].Kind,
                                                     value = ak_bytes_in.Borrow(pinned + starts[index],
                                                                                sources[index].Value.Length),
                                                   };
                                 }

                                 var config = new ak_config
                                              {
                                                struct_size = (uint)Marshal.SizeOf<ak_config>(),
                                                source_count = (uint)listed.Length,
                                                sources      = first,
                                                prefix = ak_bytes_in.Borrow(pinned,
                                                                            named.Length),
                                                log_callback = logCallback,
                                                log_ctx      = logContext,
                                              };
                                 return NativeMethods.ak_runtime_create_from(&config,
                                                                             TrampolineFrom,
                                                                             context,
                                                                             created,
                                                                             error);
                               }
                             });
  }

  /// <summary>Refuses a library this binding does not speak to, before anything is asked of it.</summary>
  private static void RefuseAnotherAbi()
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
  }

  /// <summary>Starts the engine with the options given, each one left out taking its default.</summary>
  /// <param name="options">What the engine is started with.</param>
  /// <param name="loggerFactory">
  ///   Where the engine's logs go, the engine's target as the category, or none. Written from a
  ///   thread of its own: a provider is never called on the engine's. The engine's own filter
  ///   selects what reaches it, <c>Logging.Filter</c> of the runtime's options. The factory has to outlive
  ///   the runtime, which writes to it until it is disposed.
  /// </param>
  /// <exception cref="ArgumentNullException"><paramref name="options" /> is null.</exception>
  /// <exception cref="ArgumentOutOfRangeException">An option is outside its stated bounds.</exception>
  /// <exception cref="InvalidOperationException">The engine refused as <see cref="Create(ulong,ulong,ILoggerFactory)" /> does.</exception>
  /// <exception cref="RustEngineMissingException">The engine could not be loaded.</exception>
  public static NativeRuntime Create(RuntimeOptions   options,
                                     ILoggerFactory? loggerFactory = null)
  {
    if (options is null)
    {
      throw new ArgumentNullException(nameof(options));
    }

    RefuseAWindowNoRingCanHold(options.ChannelDefaults?.Grpc?.Host?.Receive?.Window);

    var configuration = new NativeConfiguration(string.Empty).LoadConfigFromObject(options);
    return Create(configuration.Prefix,
                  configuration.Sources,
                  loggerFactory);
  }

  /// <summary>Opens a channel with the runtime's channel defaults, and the engine's elsewhere.</summary>
  /// <param name="endpoint">Where the channel connects; empty for the Endpoint of the runtime's options.</param>
  /// <exception cref="ArgumentOutOfRangeException">The runtime's channel defaults state a window no ring can hold.</exception>
  /// <exception cref="ArgumentException">The engine dials no such endpoint.</exception>
  /// <exception cref="ObjectDisposedException">This runtime is going away.</exception>
  /// <exception cref="InvalidOperationException">The engine refused for a reason of its own.</exception>
  public NativeChannel Channel(string endpoint)
    => Channel(endpoint,
               new ChannelOptions());

  /// <summary>Opens a channel with a delivery window, and the runtime's channel defaults elsewhere.</summary>
  /// <param name="endpoint">Where the channel connects; empty for the Endpoint of the runtime's options.</param>
  /// <param name="deliveryCredits">How many of a call's payloads the host may hold at once, the terminal status aside.</param>
  /// <exception cref="ArgumentOutOfRangeException">The window is outside what is admitted.</exception>
  /// <exception cref="ArgumentException">The engine dials no such endpoint.</exception>
  /// <exception cref="ObjectDisposedException">This runtime is going away.</exception>
  /// <exception cref="InvalidOperationException">The engine refused for a reason of its own.</exception>
  public NativeChannel Channel(string endpoint,
                               int deliveryCredits)
    => Channel(endpoint,
               new ChannelOptions
               {
                 Grpc = new GrpcOptions
                        {
                          Host = new HostOptions
                                 {
                                   Receive = new HostReceiveOptions
                                             {
                                               Window = deliveryCredits,
                                             },
                                 },
                        },
               });

  /// <summary>Opens a channel this runtime serves, and keeps it until it is disposed.</summary>
  /// <param name="endpoint">Where the channel connects; empty for the Endpoint of the runtime's options.</param>
  /// <param name="options">What the channel is opened with, read once and never written to.</param>
  /// <exception cref="ArgumentNullException"><paramref name="options" /> is null.</exception>
  /// <exception cref="ArgumentOutOfRangeException">
  ///   An option is outside what is admitted, or the runtime's channel defaults state a window no ring can hold.
  /// </exception>
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

    // One read of the caller's instance, so what is validated is what is sent.
    var settled = new ChannelOptions(options);

    // The schema's bounds, then this binding's own tighter one. Both are checked here rather
    // than left to the engine, which answers a bad document with a status naming no option. A
    // window the runtime's sources state is checked once the engine has settled it.
    settled.Validate();
    RefuseAWindowNoRingCanHold(settled.Grpc?.Host?.Receive?.Window);

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
  internal static void RefuseAWindowNoRingCanHold(int? deliveryCredits)
  {
    if (deliveryCredits > MaxDeliveryCredits)
    {
      throw new ArgumentOutOfRangeException("Grpc.Host.Receive.Window",
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

  private async Task RetireAsync()
  {
    BeginShutdown();
    await QuiescentAsync()
      .ConfigureAwait(false);
    Destroy();

    // Only once both have answered. A runtime that refused to be destroyed still holds this
    // pointer, and freeing the root would hand its next callback whatever the slot is reused for.
    self_.Free();

    // The engine logs nothing after the destroy, so what is queued is written and the log's
    // context released; off this thread, since the writer's providers may take their time.
    if (log_ is not null)
    {
      await Task.Run(log_.Close)
                .ConfigureAwait(false);
    }
  }

  // Out of RetireAsync, which as an async method may not pass a pointer.
  private unsafe void BeginShutdown()
    => NativeMethods.ak_runtime_begin_shutdown(handle_,
                                               null);

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

  private unsafe void Destroy()
  {
    ak_error error = default;
    var status = NativeMethods.ak_runtime_destroy(handle_,
                                                  &error);
    if (status != ak_status.AK_STATUS_OK)
    {
      throw new InvalidOperationException($"the runtime refused to be destroyed ({status}, {NativeMethods.ak_runtime_status(handle_)}): {error.Take()}");
    }
  }

  internal static unsafe void OnEvent(void*     runtimeCtx,
                                      void*     callCtx,
                                      ak_event* events,
                                      nuint     count)
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
      for (nuint at = 0; at < count; at++)
      {
        NativeMethods.ak_event_consumed(events[at].payload);
      }

      return;
    }

    var call     = target as ICallSink;
    var terminal = false;
    var stored   = false;
    for (nuint at = 0; at < count; at++)
    {
      var @event = &events[at];
      var taken  = false;
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
      catch (Exception raised)
      {
        // Nothing may unwind into the engine: an exception crossing this callback is undefined
        // on its side of the ABI. It is reported to the log, which queues it and returns.
        EngineLog.Current?.Caught("an event of a call could not be handed to its reader",
                                  raised);
      }
      finally
      {
        // Anything the ring did not take is given back here: a root that no longer names a sink,
        // a publish that threw before storing, an event of the runtime itself. What is owed and
        // never returned is what the shutdown then waits for, forever. A payload the ring did take
        // is the reader's to return, and returning it twice would free it under the reader.
        if (!taken)
        {
          NativeMethods.ak_event_consumed(@event->payload);
        }

        stored   |= taken;
        terminal |= @event->kind == ak_event_kind.AK_EVENT_STATUS;
      }
    }

    if (call is null)
    {
      return;
    }

    try
    {
      // Once for the whole callback, so the reader is woken once for what came together, and not
      // at all for an acquittal, which the ring never sees.
      if (stored)
      {
        call.Arrived();
      }
    }
    catch (Exception raised)
    {
      // As above: nothing may unwind into the engine.
      EngineLog.Current?.Caught("the reader of a call could not be woken for the events handed to it",
                                raised);
    }
    finally
    {
      // The terminal is the call's last callback, so its root goes with it whatever the publish
      // did: kept, it would hold the call for the life of the process.
      if (terminal)
      {
        call.TerminalReturned();
      }
    }
  }
}
