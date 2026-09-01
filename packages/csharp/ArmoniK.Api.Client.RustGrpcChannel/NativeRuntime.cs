using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   The native runtime, and the trampoline every event of it arrives on.
/// </summary>
/// <remarks>
///   One per process is enough: the runtime owns its threads and every channel leases it. Its
///   context and the callback stay rooted until <see cref="Dispose" /> returns, which is longer
///   than the ABI's floor - that one ends at the runtime's last event - and needs no reasoning
///   about which event was last.
/// </remarks>
public sealed class NativeRuntime : IDisposable
{
  /// <summary>
  ///   Rooted for the runtime's lifetime. A delegate marshalled to a function pointer is not kept
  ///   alive by the native side holding that pointer, so letting this be collected would leave the
  ///   library calling into a freed thunk.
  /// </summary>
  private static readonly NativeMethods.AkCallback Trampoline = OnEvent;

  private readonly TaskCompletionSource<bool> stopped_ =
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
    => NativeChannel.Open(handle_,
                          endpoint);

  private static void OnEvent(IntPtr runtimeCtx,
                              IntPtr callCtx,
                              IntPtr eventPtr)
  {
    // Nothing here may throw: unwinding into C is undefined, and this frame is called from one of
    // the library's own threads.
    try
    {
      var @event = Marshal.PtrToStructure<NativeMethods.AkEvent>(eventPtr);
      var payload = Copy(@event.Payload);

      if (@event.Payload.Owner != IntPtr.Zero)
      {
        // Consumed here, on this thread: it frees the bytes and arms the next event, and with one
        // delivery credit a call that waits to consume never sees another one.
        NativeMethods.ak_event_consumed(@event.Payload);
      }

      if (callCtx != IntPtr.Zero)
      {
        var call = GCHandle.FromIntPtr(callCtx)
                           .Target as NativeCall;
        call?.OnEvent(@event.Kind,
                      payload,
                      @event.StatusCode);
        return;
      }

      var runtime = GCHandle.FromIntPtr(runtimeCtx)
                            .Target as NativeRuntime;
      runtime?.OnRuntimeEvent(@event);
    }
    catch
    {
      // Swallowed on purpose: there is nowhere to report it, and letting it out is worse.
    }
  }

  private void OnRuntimeEvent(NativeMethods.AkEvent @event)
  {
    // SHUTDOWN_COMPLETE says whether anything of the runtime is still out; RESOURCES_RELEASED says
    // it no longer is. Only the status says destroying is permitted, so this only wakes the wait.
    if (@event.Kind is NativeMethods.AkEventKind.ShutdownComplete
                    or NativeMethods.AkEventKind.ResourcesReleased)
    {
      stopped_.TrySetResult(true);
    }
  }

  private static byte[] Copy(NativeMethods.AkBytes payload)
  {
    var length = (int)payload.Len;
    if (payload.Ptr == IntPtr.Zero || length == 0)
    {
      return Array.Empty<byte>();
    }

    var bytes = new byte[length];
    Marshal.Copy(payload.Ptr,
                 bytes,
                 0,
                 length);
    return bytes;
  }

  /// <inheritdoc />
  public void Dispose()
  {
    if (Interlocked.Exchange(ref disposed_,
                             1) != 0)
    {
      return;
    }

    NativeMethods.ak_runtime_begin_shutdown(handle_);

    // Quiescence is the permission, and no event is: the host reaches it by having given
    // everything back, which this binding does inside the trampoline as each event arrives.
    var deadline = DateTime.UtcNow.AddSeconds(30);
    while (NativeMethods.ak_runtime_status(handle_) != NativeMethods.AkRuntimeState.Quiescent
           && DateTime.UtcNow < deadline)
    {
      Thread.Sleep(5);
    }

    NativeMethods.ak_runtime_destroy(handle_);

    if (self_.IsAllocated)
    {
      self_.Free();
    }
  }
}
