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
  private readonly int deliveryCredits_;
  private int disposed_;

  internal NativeChannel(ulong runtime,
                         string endpoint,
                         int deliveryCredits)
    : base(endpoint)
  {
    runtime_         = runtime;
    deliveryCredits_ = deliveryCredits;

    var json = Encoding.UTF8.GetBytes($"{{\"endpoint\":{Quote(endpoint)},\"delivery_credits\":{deliveryCredits}}}");
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
                             handle_,
                             deliveryCredits_);

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
