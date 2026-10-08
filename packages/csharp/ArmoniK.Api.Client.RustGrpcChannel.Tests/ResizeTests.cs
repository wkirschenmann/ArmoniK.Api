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
using System.Buffers;
using System.Threading;
using System.Threading.Tasks;

using Google.Protobuf;

using Grpc.Core;

using NUnit.Framework;

using ArmoniK.Api.Client.RustGrpcChannel.Calls;
using ArmoniK.Api.Client.RustGrpcChannel.Interop;

namespace ArmoniK.Api.Client.RustGrpcChannel.Tests;

/// <summary>A message that turns out longer than the buffer it was lent is written on, not
/// refused: the engine exchanges the buffer for a larger one with what was written kept.</summary>
[TestFixture]
public class ResizeTests : EchoServerFixture
{
  private static readonly Marshaller<EchoReply> Replies = Marshallers.Create(reply => reply.ToByteArray(),
                                                                             EchoReply.Parser.ParseFrom);

  private static Method<EchoRequest, EchoReply> Say(Marshaller<EchoRequest> requests)
    => new(MethodType.Unary,
           "armonik.transport.ffi.test.Echo",
           "Say",
           requests,
           Replies);

  private static Marshaller<EchoRequest> Serializing(Action<EchoRequest, SerializationContext> write)
    => new(write,
           context => EchoRequest.Parser.ParseFrom(context.PayloadAsNewBuffer()));

  /// <summary>Everything in <paramref name="bytes" />, as a serializer that does not know its size
  /// writes it: through whatever span the writer gives, asking again when it is full.</summary>
  private static void Write(IBufferWriter<byte> writer,
                            byte[]              bytes)
  {
    var written = 0;
    while (written < bytes.Length)
    {
      var span  = writer.GetSpan(1);
      var count = Math.Min(span.Length,
                           bytes.Length - written);
      new ReadOnlySpan<byte>(bytes,
                             written,
                             count).CopyTo(span);
      writer.Advance(count);
      written += count;
    }
  }

  /// <summary>A serializer that announces eight bytes and writes a message of any length, whose
  /// buffer is exchanged for a larger one as often as it fills.</summary>
  [Test]
  public async Task AMessageLongerThanItsAnnouncementIsWrittenThroughTheExchangeOfItsBuffer()
  {
    await using var channel = Runtime.Channel(Endpoint);

    // Long enough for the buffer to be exchanged some sixteen times, from eight bytes.
    var text = new string('x',
                          300_000);

    var announcements = 0;
    var requests = Serializing((request,
                                context) =>
                               {
                                 Interlocked.Increment(ref announcements);
                                 context.SetPayloadLength(8);
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    for (var turn = 0; turn < 2; ++turn)
    {
      var reply = channel.CreateCallInvoker()
                         .BlockingUnaryCall(Say(requests),
                                            null,
                                            new CallOptions(),
                                            new EchoRequest
                                            {
                                              Text = text,
                                            });
      Assert.That(reply.Text,
                  Is.EqualTo(text));
    }

    Assert.That(announcements,
                Is.EqualTo(2),
                "each message was serialized once: nothing was given up and begun again");
  }

  /// <summary>What a serializer holds when the buffer is exchanged is a disposed view, rather than
  /// the arena the engine has taken back.</summary>
  [Test]
  public async Task AViewHeldAcrossTheExchangeIsDisposed()
  {
    await using var channel = Runtime.Channel(Endpoint);

    Exception? raised = null;
    var requests = Serializing((request,
                                context) =>
                               {
                                 var bytes = request.ToByteArray();
                                 context.SetPayloadLength(4);
                                 var writer = context.GetBufferWriter();
                                 var held   = writer.GetMemory(1);
                                 bytes.AsSpan(0,
                                              4)
                                      .CopyTo(held.Span);
                                 writer.Advance(4);

                                 // Past the four announced.
                                 Write(writer,
                                       bytes.AsSpan(4)
                                            .ToArray());
                                 try
                                 {
                                   _ = held.Span;
                                 }
                                 catch (Exception exception)
                                 {
                                   raised = exception;
                                 }

                                 context.Complete();
                               });

    var reply = channel.CreateCallInvoker()
                       .BlockingUnaryCall(Say(requests),
                                          null,
                                          new CallOptions(),
                                          new EchoRequest
                                          {
                                            Text = "the whole of this is longer than four bytes",
                                          });

    Assert.Multiple(() =>
                    {
                      Assert.That(reply.Text,
                                  Is.EqualTo("the whole of this is longer than four bytes"),
                                  "what was written before the exchange is kept");
                      Assert.That(raised,
                                  Is.InstanceOf<ObjectDisposedException>());
                    });
  }

  /// <summary>A message that outgrows its buffer where the ceiling has no room for the larger one
  /// waits for the room, as a lend does, and is sent whole once it comes.</summary>
  /// <remarks>The buffer it held is given back for the wait, and the next attempt lends at the
  /// length the first asked for, so the wait is for the room the message needs.</remarks>
  [Test]
  public async Task AMessageThatOutgrowsItsBufferWaitsForRoomUnderAMemoryCeiling()
  {
    const int ceiling = 64 * 1024;

    var runtime = await RestartAsync(memoryCeiling: ceiling)
                    .ConfigureAwait(false);

    await using var holder  = runtime.Channel(Endpoint);
    await using var growing = runtime.Channel(Endpoint);

    using var holding = new ManualResetEventSlim(false);
    using var release = new ManualResetEventSlim(false);
    using var refused = new ManualResetEventSlim(false);

    // Lends most of the ceiling and keeps it until told to go: the message below has room for its
    // announcement and not for what it turns out to be.
    var holds = Serializing((request,
                             context) =>
                            {
                              context.SetPayloadLength(40 * 1024);
                              holding.Set();
                              release.Wait();
                              context.Complete(request.ToByteArray());
                            });

    var attempts = 0;
    var text     = new string('y',
                              40_000);
    var outgrows = Serializing((request,
                                context) =>
                               {
                                 Interlocked.Increment(ref attempts);
                                 context.SetPayloadLength(8);
                                 try
                                 {
                                   Write(context.GetBufferWriter(),
                                         request.ToByteArray());
                                 }
                                 finally
                                 {
                                   // Whether the exchange was refused or not, this attempt is done
                                   // with the room it asked for.
                                   refused.Set();
                                 }

                                 context.Complete();
                               });

    var holdsTheCeiling = Task.Run(() => holder.CreateCallInvoker()
                                               .BlockingUnaryCall(Say(holds),
                                                                  null,
                                                                  new CallOptions(),
                                                                  new EchoRequest
                                                                  {
                                                                    Text = "held",
                                                                  }));
    try
    {
      Assert.That(holding.Wait(TimeSpan.FromSeconds(30)),
                  Is.True,
                  "the ceiling is lent and held");

      var sending = Task.Run(() => growing.CreateCallInvoker()
                                          .BlockingUnaryCall(Say(outgrows),
                                                             null,
                                                             new CallOptions(),
                                                             new EchoRequest
                                                             {
                                                               Text = text,
                                                             }));

      Assert.That(refused.Wait(TimeSpan.FromSeconds(30)),
                  Is.True,
                  "the message reached the end of the buffer it was lent");
      Assert.That(sending.IsCompleted,
                  Is.False,
                  "and waits: the room is held by another call");

      release.Set();

      Assert.That(await Task.WhenAny(sending,
                                     Task.Delay(TimeSpan.FromSeconds(30)))
                            .ConfigureAwait(false),
                  Is.SameAs(sending),
                  "the room came back and the message was sent");
      Assert.Multiple(() =>
                      {
                        Assert.That(sending.Result.Text,
                                    Is.EqualTo(text));
                        Assert.That(attempts,
                                    Is.GreaterThanOrEqualTo(2),
                                    "the first attempt gave its buffer back to wait");
                      });
    }
    finally
    {
      release.Set();
      await holdsTheCeiling.ConfigureAwait(false);
    }
  }

  /// <summary>A message that outgrows its buffer past half the ceiling is sized by what it needs
  /// and not by the doubling, which the ceiling could never hold.</summary>
  [Test]
  public async Task AMessageThatOutgrowsItsBufferPastHalfTheCeilingIsNotRefusedForItsDoubling()
  {
    const int ceiling = 64 * 1024;

    var runtime = await RestartAsync(memoryCeiling: ceiling)
                    .ConfigureAwait(false);

    await using var channel = runtime.Channel(Endpoint);

    var text = new string('w',
                          41_000);
    var requests = Serializing((request,
                                context) =>
                               {
                                 context.SetPayloadLength(40_000);
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    var reply = channel.CreateCallInvoker()
                       .BlockingUnaryCall(Say(requests),
                                          null,
                                          new CallOptions(),
                                          new EchoRequest
                                          {
                                            Text = text,
                                          });

    Assert.That(reply.Text,
                Is.EqualTo(text));
  }

  /// <summary>A message that outgrows its buffer at the top of the ceiling is sized by what it
  /// needs: the double and the quarter are both past the ceiling, and the message is not.</summary>
  [Test]
  public async Task AMessageThatOutgrowsItsBufferAtTheTopOfTheCeilingIsSizedByWhatItNeeds()
  {
    const int ceiling = 64 * 1024;

    var runtime = await RestartAsync(memoryCeiling: ceiling)
                    .ConfigureAwait(false);

    await using var channel = runtime.Channel(Endpoint);

    var text = new string('v',
                          63_000);
    var requests = Serializing((request,
                                context) =>
                               {
                                 context.SetPayloadLength(60_000);
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    var reply = channel.CreateCallInvoker()
                       .BlockingUnaryCall(Say(requests),
                                          null,
                                          new CallOptions(),
                                          new EchoRequest
                                          {
                                            Text = text,
                                          });

    Assert.That(reply.Text,
                Is.EqualTo(text));
  }

  /// <summary>A message longer than the ceiling is refused as too large, whatever it announced.</summary>
  [Test]
  public async Task AMessageThatOutgrowsTheCeilingItselfIsRefusedAsTooLarge()
  {
    const int ceiling = 64 * 1024;

    var runtime = await RestartAsync(memoryCeiling: ceiling)
                    .ConfigureAwait(false);

    await using var channel = runtime.Channel(Endpoint);

    var requests = Serializing((request,
                                context) =>
                               {
                                 context.SetPayloadLength(40_000);
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    var refused = Assert.Throws<RpcException>(() => channel.CreateCallInvoker()
                                                           .BlockingUnaryCall(Say(requests),
                                                                              null,
                                                                              new CallOptions(),
                                                                              new EchoRequest
                                                                              {
                                                                                Text = new string('u',
                                                                                                  70_000),
                                                                              }));

    Assert.That(refused!.StatusCode,
                Is.EqualTo(StatusCode.ResourceExhausted));
  }

  /// <summary>An array longer than what was announced takes the buffer's place and is sent
  /// whole.</summary>
  /// <remarks>A guard against a regression and not a witness of the exchange: with the room free,
  /// giving the buffer back and lending again would pass too. What the exchange adds is that no
  /// other call can take the room between the two, which only a race could show.</remarks>
  [Test]
  public async Task AnArrayLargerThanTheAnnouncementReplacesItUnderAMemoryCeiling()
  {
    const int ceiling = 64 * 1024;

    var runtime = await RestartAsync(memoryCeiling: ceiling)
                    .ConfigureAwait(false);

    await using var channel = runtime.Channel(Endpoint);

    // Announced at three quarters of the ceiling and replaced by a larger array: the old and the
    // new buffer together are past the ceiling, and the exchange is not a refusal.
    var text = new string('z',
                          ceiling - 2_000);
    var requests = Serializing((request,
                                context) =>
                               {
                                 context.SetPayloadLength(ceiling / 4 * 3);
                                 context.Complete(request.ToByteArray());
                               });

    var reply = channel.CreateCallInvoker()
                       .BlockingUnaryCall(Say(requests),
                                          null,
                                          new CallOptions(),
                                          new EchoRequest
                                          {
                                            Text = text,
                                          });

    Assert.That(reply.Text,
                Is.EqualTo(text));
  }

  /// <summary>A serializer that announces nothing, which Grpc.Core allows, still writes: its first
  /// bytes are the first lend, and the buffer grows from there.</summary>
  [Test]
  public async Task AMessageThatAnnouncedNothingIsWrittenThroughTheLendItAsksFor()
  {
    await using var channel = Runtime.Channel(Endpoint);

    var text = new string('n',
                          100_000);
    var requests = Serializing((request,
                                context) =>
                               {
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    var reply = channel.CreateCallInvoker()
                       .BlockingUnaryCall(Say(requests),
                                          null,
                                          new CallOptions(),
                                          new EchoRequest
                                          {
                                            Text = text,
                                          });

    Assert.That(reply.Text,
                Is.EqualTo(text));
  }

  /// <summary>A serializer that announces zero bytes and then writes is in the same state as one
  /// that announced nothing: no buffer is lent yet.</summary>
  [Test]
  public async Task AMessageThatAnnouncedZeroBytesIsWrittenThroughTheLendItAsksFor()
  {
    await using var channel = Runtime.Channel(Endpoint);

    var requests = Serializing((request,
                                context) =>
                               {
                                 context.SetPayloadLength(0);
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    var reply = channel.CreateCallInvoker()
                       .BlockingUnaryCall(Say(requests),
                                          null,
                                          new CallOptions(),
                                          new EchoRequest
                                          {
                                            Text = "written after announcing nothing",
                                          });

    Assert.That(reply.Text,
                Is.EqualTo("written after announcing nothing"));
  }

  /// <summary>Lengths above this are the ones the failing allocator below refuses.</summary>
  private const int Affordable = 4096;

  /// <summary>An allocator that fails on a doubled size serves the size the message needs: the
  /// write is not aborted for a size it did not ask for.</summary>
  /// <remarks>The engine's allocator does not fail on request within a ceiling, so the exchange is
  /// replaced for the test by one that answers with the allocator's failure once, for the first
  /// length above <see cref="Affordable" />, and is the engine's otherwise.</remarks>
  [Test]
  public async Task AnAllocatorThatFailsOnTheDoubledSizeIsAskedForWhatTheMessageNeeds()
  {
    await using var channel = Runtime.Channel(Endpoint);

    var text     = new string('a',
                              20_000);
    var failures = 0;
    var requests = Serializing((request,
                                context) =>
                               {
                                 context.SetPayloadLength(8);
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    var engine = LentBuffer.Exchanger;
    LentBuffer.Exchanger = (ak_buffer     buffer,
                            nuint         length,
                            nuint         keep,
                            out ak_buffer resized) =>
                           {
                             if (length > Affordable && failures == 0)
                             {
                               ++failures;
                               resized = default;
                               return ak_status.AK_STATUS_INTERNAL;
                             }

                             return LentBuffer.Engine(buffer,
                                                      length,
                                                      keep,
                                                      out resized);
                           };
    try
    {
      var reply = channel.CreateCallInvoker()
                         .BlockingUnaryCall(Say(requests),
                                            null,
                                            new CallOptions(),
                                            new EchoRequest
                                            {
                                              Text = text,
                                            });

      Assert.Multiple(() =>
                      {
                        Assert.That(failures,
                                    Is.EqualTo(1),
                                    "the allocator failed once");
                        Assert.That(reply.Text,
                                    Is.EqualTo(text));
                      });
    }
    finally
    {
      LentBuffer.Exchanger = engine;
    }
  }

  /// <summary>An allocator that fails on the size the message needs too ends the write as an
  /// internal fault, after the one retry and no more.</summary>
  [Test]
  public async Task AnAllocatorThatFailsOnWhatTheMessageNeedsAbortsTheWriteAfterOneRetry()
  {
    await using var channel = Runtime.Channel(Endpoint);

    var attempts = 0;
    var requests = Serializing((request,
                                context) =>
                               {
                                 context.SetPayloadLength(8);
                                 Write(context.GetBufferWriter(),
                                       request.ToByteArray());
                                 context.Complete();
                               });

    var engine = LentBuffer.Exchanger;
    LentBuffer.Exchanger = (ak_buffer     buffer,
                            nuint         length,
                            nuint         keep,
                            out ak_buffer resized) =>
                           {
                             if (length > Affordable)
                             {
                               ++attempts;
                               resized = default;
                               return ak_status.AK_STATUS_INTERNAL;
                             }

                             return LentBuffer.Engine(buffer,
                                                      length,
                                                      keep,
                                                      out resized);
                           };
    try
    {
      var refused = Assert.Throws<RpcException>(() => channel.CreateCallInvoker()
                                                             .BlockingUnaryCall(Say(requests),
                                                                                null,
                                                                                new CallOptions(),
                                                                                new EchoRequest
                                                                                {
                                                                                  Text = new string('b',
                                                                                                    20_000),
                                                                                }));

      Assert.Multiple(() =>
                      {
                        Assert.That(refused!.StatusCode,
                                    Is.EqualTo(StatusCode.Internal));
                        Assert.That(attempts,
                                    Is.EqualTo(2),
                                    "the doubled size, then the size asked for");
                      });
    }
    finally
    {
      LentBuffer.Exchanger = engine;
    }
  }
}
