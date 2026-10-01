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
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

using Google.Protobuf;

using Grpc.Core;

using NUnit.Framework;

using ArmoniK.Api.Client.RustGrpcChannel.Calls;
using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>The receiving half driven by hand: the test publishes the events the engine would.</summary>
/// <remarks>Every payload is owned by nobody, which the ABI gives back as a no-op, so no engine is
/// involved.</remarks>
[TestFixture]
public class ReceiverTests
{
  // Built here, because the tests that publish it are async and an async method may not hold a
  // pointer.
  private static unsafe ak_bytes Payload(IntPtr at,
                                         int    length)
    => new()
       {
         ptr = (byte*)at,
         len = (nuint)length,
       };

  /// <remarks>
  ///   The prologue leaves the head in the ring when the call is ending before the terminal is in,
  ///   and a send that failed ends it so with no drain owed: the reader takes the head. That head
  ///   said no response came, and whoever takes it, the terminal answers the headers the same way.
  /// </remarks>
  [Test]
  public async Task AHeadTheReaderTakesAnswersTheHeadersAsTheProloguesWould()
  {
    var call = new EndingCall();
    var receiver = new Receiver<EchoReply>(call,
                                           1,
                                           Marshallers.Create<EchoReply>(reply => reply.ToByteArray(),
                                                                         EchoReply.Parser.ParseFrom));
    receiver.StartPrologue();

    // Ended before the head lands, so the prologue finds no terminal behind it and leaves.
    call.EndCall();
    receiver.Publish(ak_event_kind.AK_EVENT_INITIAL_METADATA,
                     default,
                     (int)ak_head_origin.AK_HEAD_NO_RESPONSE);
    await receiver.PrologueFinished.ConfigureAwait(false);

    var reading = receiver.MoveNext(CancellationToken.None);
    receiver.Publish(ak_event_kind.AK_EVENT_STATUS,
                     default,
                     (int)StatusCode.Cancelled);

    Assert.ThrowsAsync<RpcException>(async () => await reading.ConfigureAwait(false));
    var headers = Assert.ThrowsAsync<RpcException>(async () => await receiver.ResponseHeadersAsync.ConfigureAwait(false));
    Assert.That(headers!.StatusCode,
                Is.EqualTo(StatusCode.Cancelled));
  }

  /// <remarks>
  ///   A terminal nobody can decode leaves no trailers to answer with, so a head that left the
  ///   headers to it fails them rather than answering an empty set as a success, whether a read or
  ///   the drain consumes it.
  /// </remarks>
  [Test]
  public async Task AnUnreadableTerminalFailsTheHeadersItWasToAnswer([Values] bool drained)
  {
    // An empty reason, then one trailer whose key Metadata refuses.
    var blob = new List<byte>();
    void Chunk(byte[] bytes)
    {
      blob.AddRange(BitConverter.GetBytes((uint)bytes.Length));
      blob.AddRange(bytes);
    }

    Chunk(Array.Empty<byte>());
    blob.AddRange(BitConverter.GetBytes(1u));
    Chunk(Encoding.ASCII.GetBytes("Bad Key"));
    Chunk(Encoding.ASCII.GetBytes("v"));

    var memory = Marshal.AllocHGlobal(blob.Count);
    try
    {
      Marshal.Copy(blob.ToArray(),
                   0,
                   memory,
                   blob.Count);

      var call = new EndingCall();
      var receiver = new Receiver<EchoReply>(call,
                                             1,
                                             Marshallers.Create<EchoReply>(reply => reply.ToByteArray(),
                                                                           EchoReply.Parser.ParseFrom));
      receiver.StartPrologue();

      call.EndCall();
      receiver.Publish(ak_event_kind.AK_EVENT_INITIAL_METADATA,
                       default,
                       (int)ak_head_origin.AK_HEAD_TRAILERS_ONLY);
      await receiver.PrologueFinished.ConfigureAwait(false);

      var reading = drained
                      ? null
                      : receiver.MoveNext(CancellationToken.None);
      receiver.Publish(ak_event_kind.AK_EVENT_STATUS,
                       Payload(memory,
                               blob.Count),
                       (int)StatusCode.OK);

      if (reading is null)
      {
        receiver.CancelAndDrain();
        await receiver.Settled.ConfigureAwait(false);
      }
      else
      {
        Assert.ThrowsAsync<RpcException>(async () => await reading.ConfigureAwait(false));
      }

      var headers = Assert.ThrowsAsync<RpcException>(async () => await receiver.ResponseHeadersAsync.ConfigureAwait(false));
      Assert.That(headers!.StatusCode,
                  Is.EqualTo(StatusCode.Internal));
    }
    finally
    {
      Marshal.FreeHGlobal(memory);
    }
  }

#if DEBUG
  /// <remarks>
  ///   The interleaving held open: a read finds the ring empty, and before it waits a cancel ends
  ///   it and hands the ring to the drain, which finds the ring empty too and takes the terminal
  ///   once it lands. The cancel's wake-up has to reach the read, which then finds itself ended,
  ///   and not go to the drain alone. The hook is Debug-only and process-wide.
  /// </remarks>
  [Test]
  [NonParallelizable]
  public async Task AReadHeldBeforeItsWaitSeesTheCancelThatEndedIt()
  {
    var receiver = new Receiver<EchoReply>(new EndingCall(),
                                           1,
                                           Marshallers.Create<EchoReply>(reply => reply.ToByteArray(),
                                                                         EchoReply.Parser.ParseFrom));
    receiver.StartPrologue();
    receiver.Publish(ak_event_kind.AK_EVENT_INITIAL_METADATA,
                     default,
                     (int)ak_head_origin.AK_HEAD_RECEIVED);
    await receiver.PrologueFinished.ConfigureAwait(false);

    using var readerHeld    = new SemaphoreSlim(0);
    using var releaseReader = new SemaphoreSlim(0);
    using var drainHeld     = new SemaphoreSlim(0);
    using var releaseDrain  = new SemaphoreSlim(0);
    TestHooks.FoundTheRingEmpty = consumer => consumer switch
                                              {
                                                RingConsumer.Reader => Hold(readerHeld,
                                                                            releaseReader),
                                                RingConsumer.Drain => Hold(drainHeld,
                                                                           releaseDrain),
                                                _ => Task.CompletedTask,
                                              };
    try
    {
      var reading = receiver.MoveNext(CancellationToken.None);
      Assert.That(await readerHeld.WaitAsync(TimeSpan.FromSeconds(10))
                                  .ConfigureAwait(false),
                  Is.True,
                  "the read found the ring empty");

      receiver.CancelAndDrain();
      Assert.That(await drainHeld.WaitAsync(TimeSpan.FromSeconds(10))
                                 .ConfigureAwait(false),
                  Is.True,
                  "the drain found the ring empty");

      receiver.Publish(ak_event_kind.AK_EVENT_STATUS,
                       default,
                       (int)StatusCode.Cancelled);
      releaseDrain.Release();
      Assert.That(await Task.WhenAny(receiver.Settled,
                                     Task.Delay(TimeSpan.FromSeconds(10)))
                            .ConfigureAwait(false),
                  Is.SameAs(receiver.Settled),
                  "the drain took the terminal");

      releaseReader.Release();
      Assert.That(await Task.WhenAny(reading,
                                     Task.Delay(TimeSpan.FromSeconds(10)))
                            .ConfigureAwait(false),
                  Is.SameAs(reading),
                  "the read waits for a wake-up that went to the drain");
      Assert.ThrowsAsync<RpcException>(async () => await reading.ConfigureAwait(false));
    }
    finally
    {
      TestHooks.FoundTheRingEmpty = null;
      releaseReader.Release();
      releaseDrain.Release();
    }
  }

  private static async Task Hold(SemaphoreSlim held,
                                 SemaphoreSlim release)
  {
    held.Release();
    await release.WaitAsync()
                 .ConfigureAwait(false);
  }
#endif

  private sealed class EndingCall : ICallState
  {
    private readonly CancellationTokenSource ending_ = new();

    public ulong Handle
      => 0;

    public CancellationToken Ending
      => ending_.Token;

    public Task<Status> TerminalAsync
      => Task.FromResult(Status.DefaultCancelled);

    public Metadata Trailers
      => Metadata.Empty;

    public void EndCall()
      => ending_.Cancel();
  }
}
