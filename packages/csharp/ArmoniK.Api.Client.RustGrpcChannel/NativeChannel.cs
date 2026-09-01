using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   A channel on the native runtime: calls, over one HTTP/2 session to one endpoint.
/// </summary>
public sealed class NativeChannel : IDisposable
{
  private readonly ulong handle_;
  private int disposed_;

  private NativeChannel(ulong handle)
    => handle_ = handle;

  internal static NativeChannel Open(ulong runtime,
                                     string endpoint)
  {
    var json = Encoding.UTF8.GetBytes($"{{\"endpoint\":\"{endpoint}\"}}");
    var pin = GCHandle.Alloc(json,
                             GCHandleType.Pinned);
    try
    {
      var config = new NativeMethods.AkBytesIn
                   {
                     Ptr = pin.AddrOfPinnedObject(),
                     Len = (UIntPtr)json.Length,
                   };

      var status = NativeMethods.ak_channel_create(runtime,
                                                   config,
                                                   out var handle);
      if (status != NativeMethods.AkStatus.Ok)
      {
        throw new InvalidOperationException($"`{endpoint}` was refused ({status})");
      }

      return new NativeChannel(handle);
    }
    finally
    {
      pin.Free();
    }
  }

  /// <summary>A <see cref="CallInvoker" /> the generated stubs can be built on.</summary>
  public CallInvoker CreateCallInvoker()
    => new NativeCallInvoker(handle_);

  /// <inheritdoc />
  public void Dispose()
  {
    if (Interlocked.Exchange(ref disposed_,
                             1) == 0)
    {
      NativeMethods.ak_channel_release(handle_);
    }
  }
}
