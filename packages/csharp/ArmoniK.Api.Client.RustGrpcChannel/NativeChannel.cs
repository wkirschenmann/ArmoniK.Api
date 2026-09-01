using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using Grpc.Core;

namespace ArmoniK.Api.Client.RustGrpcChannel;

/// <summary>
///   A channel on the native runtime: calls, over one HTTP/2 session to one endpoint.
/// </summary>
public sealed class NativeChannel : ChannelBase, IDisposable
{
  private readonly ulong runtime_;
  private readonly ulong handle_;
  private int disposed_;

  internal NativeChannel(ulong runtime,
                         string endpoint)
    : base(endpoint)
  {
    runtime_ = runtime;

    var json = Encoding.UTF8.GetBytes($"{{\"endpoint\":{Quote(endpoint)}}}");
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
                                                   out handle_);
      if (status != NativeMethods.AkStatus.Ok)
      {
        throw new InvalidOperationException($"`{endpoint}` was refused ({status})");
      }
    }
    finally
    {
      pin.Free();
    }
  }

  /// <inheritdoc />
  public override CallInvoker CreateCallInvoker()
    => new NativeCallInvoker(runtime_,
                             handle_);

  /// <inheritdoc />
  protected override Task ShutdownAsyncCore()
  {
    Dispose();
    return Task.CompletedTask;
  }

  /// <inheritdoc />
  public void Dispose()
  {
    if (Interlocked.Exchange(ref disposed_,
                             1) != 0)
    {
      return;
    }

    NativeMethods.ak_channel_release(handle_);
  }

  private static string Quote(string value)
  {
    var quoted = new StringBuilder(value.Length + 2).Append('"');
    foreach (var character in value)
    {
      switch (character)
      {
        case '"':
          quoted.Append("\\\"");
          break;

        case '\\':
          quoted.Append("\\\\");
          break;

        default:
          if (character < ' ')
          {
            quoted.Append("\\u")
                  .Append(((int)character).ToString("x4"));
          }
          else
          {
            quoted.Append(character);
          }

          break;
      }
    }

    return quoted.Append('"')
                 .ToString();
  }
}
