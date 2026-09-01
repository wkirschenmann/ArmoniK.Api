using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   The native runtime, and the trampoline every event of it arrives on.
/// </summary>
/// <remarks>
///   One per process is enough: the runtime owns its threads and every channel leases it.
/// </remarks>
public sealed class NativeRuntime : IDisposable
{
  /// <summary>How long <see cref="Dispose" /> gives the runtime to stop before giving up on it.</summary>
  private static readonly TimeSpan ShutdownTimeout = TimeSpan.FromSeconds(30);

  /// <summary>
  ///   Rooted for the runtime's lifetime. A delegate marshalled to a function pointer is not kept
  ///   alive by the native side holding that pointer, so letting this be collected would leave the
  ///   library calling into a freed thunk.
  /// </summary>
  private static readonly NativeMethods.AkCallback Trampoline = OnEvent;

  private readonly TaskCompletionSource<bool> released_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private GCHandle self_;
  private readonly ulong handle_;
  private int disposed_;

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

  /// <summary>Starts a runtime, after checking the library speaks the ABI this was built against.</summary>
  /// <param name="workerThreads">Zero leaves the choice to the runtime.</param>
  /// <param name="memoryCeiling">Bytes lent buffers may occupy at once. Zero is no ceiling.</param>
  public static NativeRuntime Start(uint workerThreads = 0,
                                    ulong memoryCeiling = 0)
  {
    var found = NativeMethods.ak_abi_version();
    if (found != NativeMethods.AbiVersion)
    {
      throw new InvalidOperationException($"the native library speaks ABI {found}, this binding speaks {NativeMethods.AbiVersion}");
    }

    return new NativeRuntime(workerThreads,
                             memoryCeiling);
  }

  /// <summary>Opens a channel on this runtime.</summary>
  public NativeChannel Channel(string endpoint)
    => new(handle_,
           endpoint);

  /// <summary>What the runtime currently holds against its ceiling.</summary>
  public (ulong Used, ulong Ceiling) MemoryUsage()
    => NativeMethods.ak_runtime_memory_usage(handle_,
                                             out var usage) == NativeMethods.AkStatus.Ok
         ? (usage.BytesUsed, usage.Ceiling)
         : (0UL, 0UL);

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

  /// <inheritdoc />
  /// <exception cref="InvalidOperationException">
  ///   The runtime did not reach quiescence, so destroying it is not permitted and its threads
  ///   stay up. Raised rather than swallowed: nothing else would ever report it.
  /// </exception>
  public void Dispose()
  {
    if (Interlocked.Exchange(ref disposed_,
                             1) != 0)
    {
      return;
    }

    NativeMethods.ak_runtime_begin_shutdown(handle_);

    // Quiescence is reached by giving everything back, not by waiting for it, and the drain of
    // each call is what does that. The wait here is only for the runtime to say it is done.
    if (!released_.Task.Wait(ShutdownTimeout))
    {
      throw new InvalidOperationException($"the runtime did not quiesce within {ShutdownTimeout} ({NativeMethods.ak_runtime_status(handle_)})");
    }

    var status = NativeMethods.ak_runtime_destroy(handle_);
    if (status != NativeMethods.AkStatus.Ok)
    {
      throw new InvalidOperationException($"the runtime refused to be destroyed ({status}, {NativeMethods.ak_runtime_status(handle_)})");
    }

    // Only now: until destroy returns, a callback can still be in flight carrying this root.
    self_.Free();
  }
}
