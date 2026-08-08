using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

using ArmoniK.Api.Client.Native.Spike.Generated;

using Google.Protobuf;

using Grpc.Core;
using Grpc.Net.Client;

using NUnit.Framework;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   The go/no-go checklist of the FFI spike, one test per point.
/// </summary>
/// <remarks>
///   The question these answer is not whether the Rust side works - the Rust tests settle that -
///   but whether Grpc.Net.Client on .NET Framework drives a custom duplex handler the way HTTP/2
///   gRPC needs. Tests 3 and 4 are the ones the whole plan turns on.
/// </remarks>
[TestFixture]
public class ChecklistTests
{
  private SpikeServer?    server_;
  private GrpcChannel?    channel_;
  private Raw.RawClient?  client_;

  [OneTimeSetUp]
  public void StartServer()
  {
    server_ = new SpikeServer();
    channel_ = GrpcChannel.ForAddress(server_.Endpoint,
                                      new GrpcChannelOptions
                                      {
                                        HttpHandler = new RustHttpHandler($"{{\"Endpoint\": \"{server_.Endpoint}\"}}"),
                                        // The large-message test deliberately goes past the 4 MiB default.
                                        MaxSendMessageSize    = 64 * 1024 * 1024,
                                        MaxReceiveMessageSize = 64 * 1024 * 1024,
                                      });
    client_ = new Raw.RawClient(channel_);
  }

  [OneTimeTearDown]
  public void StopServer()
  {
    channel_?.Dispose();
    server_?.Dispose();
  }

  private Raw.RawClient Client
    => client_ ?? throw new InvalidOperationException("the fixture did not start");

  private static EchoMsg Msg(string payload)
    => new()
       {
         Payload = ByteString.CopyFromUtf8(payload),
       };

  private static string Text(EchoMsg message)
    => message.Payload.ToStringUtf8();

  private static long LastError;

  private static int Observed(EchoMsg reply)
  {
    var parts = Text(reply)
      .Split(':');
    LastError = long.Parse(parts[1]);
    return int.Parse(parts[0]);
  }

  [Test]
  public async Task Test01_Unary()
  {
    var reply = await Client.UnaryAsync(Msg("ping"));
    Assert.That(Text(reply),
                Is.EqualTo("ping"));
  }

  [Test]
  public async Task Test02_ServerStreaming()
  {
    using var call = Client.ServerStream(Msg("s"));

    var seen = new List<string>();
    while (await call.ResponseStream.MoveNext(CancellationToken.None))
    {
      seen.Add(Text(call.ResponseStream.Current));
    }

    Assert.That(seen,
                Is.EqualTo(new[]
                           {
                             "s#0",
                             "s#1",
                             "s#2",
                             "s#3",
                             "s#4",
                           }),
                "every message, whole and in order");

    var status = call.GetStatus();
    Assert.That(status.StatusCode,
                Is.EqualTo(StatusCode.OK));
  }

  [Test]
  public async Task Test03_ClientStreaming_TheCriticalOne()
  {
    // The question: does Grpc.Net.Client's netfx build pump the request content while the response
    // is still outstanding, or does it wait for the request body to end before it will even ask for
    // the headers? If it waits, this passes but test 4 cannot; if it buffers the whole request, a
    // large one would too. This is the cheaper half of the same question.
    using var call = Client.ClientStream();

    foreach (var part in new[]
                         {
                           "a",
                           "b",
                           "c",
                         })
    {
      await call.RequestStream.WriteAsync(Msg(part));
    }

    await call.RequestStream.CompleteAsync();
    var reply = await call.ResponseAsync;

    Assert.That(Text(reply),
                Is.EqualTo("abc"),
                "every message reached the handler, in order");
  }

  [Test]
  public async Task Test04_Bidirectional_StrictAlternation_TheCriticalOne()
  {
    // Nothing on either side may buffer. The server answers each message as it arrives, and this
    // refuses to send the next one until it has read the previous reply, so anything holding the
    // request body back until it closes shows up here as a hang rather than as a wrong answer.
    using var call = Client.Bidi();

    foreach (var part in new[]
                         {
                           "one",
                           "two",
                           "three",
                         })
    {
      await call.RequestStream.WriteAsync(Msg(part));

      var moved = await call.ResponseStream.MoveNext(CancellationToken.None);
      Assert.That(moved,
                  Is.True,
                  $"the reply to {part} never arrived: something is buffering");
      Assert.That(Text(call.ResponseStream.Current),
                  Is.EqualTo(part));
    }

    await call.RequestStream.CompleteAsync();
    Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None),
                Is.False);
    Assert.That(call.GetStatus()
                    .StatusCode,
                Is.EqualTo(StatusCode.OK));
  }

  [Test]
  public async Task Test05_TrailersCarryTheGrpcStatus()
  {
    // A successful call ends with `grpc-status: 0` in the trailers, which only reaches
    // Grpc.Net.Client through `RequestMessage.Properties["__ResponseTrailers"]` on this platform.
    // If the handler did not publish them, the call would fail rather than succeed.
    using var call = Client.ServerStream(Msg("t"));
    while (await call.ResponseStream.MoveNext(CancellationToken.None))
    {
    }

    Assert.That(call.GetStatus()
                    .StatusCode,
                Is.EqualTo(StatusCode.OK));
    Assert.That(call.GetTrailers(),
                Is.Not.Null);
  }

  [Test]
  public void Test06_TrailersOnlyResponse()
  {
    // The handler returns a status before writing any header, so `grpc-status` arrives in the
    // response headers and the body is empty. A reader that insisted on trailers would hang.
    var failure = Assert.ThrowsAsync<RpcException>(async () => await Client.FailAsync(Msg("x")));
    Assert.That(failure!.StatusCode,
                Is.EqualTo(StatusCode.FailedPrecondition));
    Assert.That(failure.Status.Detail,
                Does.Contain("refused on purpose"));
  }

  [Test]
  public void Test07_DeadlineTerminatesTheCall()
  {
    // What this gates is the CancellationToken -> ak_request_cancel path: Grpc.Net.Client emits a
    // `grpc-timeout` header, cancels its token when the deadline elapses, and the call has to end
    // there rather than wait on a server that never answers.
    //
    // What it deliberately does NOT gate is which status comes out. Against this handler the call
    // ends as DeadlineExceeded on a cold connection and as Cancelled on a warm one, and the reason
    // is inside Grpc.Net.Client: its deadline callback sets `_deadline = DateTime.MaxValue` before
    // completing the call task with the DeadlineExceeded status, and `ResolveException` decides
    // between the two codes by reading exactly those. A handler that surfaces the cancellation
    // between those two writes - which an in-process transport does, and a socket usually does not -
    // lands on Cancelled. Nothing on this side of the ABI can close that window. Recorded as an
    // open item rather than asserted, so that the gate stays about the transport.
    var started = DateTime.UtcNow;
    using var call = Client.Hang(deadline: DateTime.UtcNow.AddSeconds(2));

    var failure = Assert.ThrowsAsync<RpcException>(async () =>
                                                   {
                                                     await call.RequestStream.WriteAsync(Msg("h"));
                                                     await call.RequestStream.CompleteAsync();
                                                     await call.ResponseStream.MoveNext(CancellationToken.None);
                                                   });

    var elapsed = DateTime.UtcNow - started;
    TestContext.WriteLine($"deadline surfaced after {elapsed.TotalSeconds:F1}s as {failure!.StatusCode}: {failure.Status.Detail}");

    Assert.That(elapsed,
                Is.LessThan(TimeSpan.FromSeconds(10)),
                "the deadline never reached the transport");
    Assert.That(failure.StatusCode,
                Is.AnyOf(StatusCode.DeadlineExceeded,
                         StatusCode.Cancelled),
                "the call has to end at the deadline; which of the two codes is a Grpc.Net.Client race");
  }

  [Test]
  public async Task Test08_CancellationReachesTheServer()
  {
    var before = Observed(await Client.CancelsObservedAsync(Msg("")));

    using (var cancellation = new CancellationTokenSource())
    {
      using var call = Client.CancelWatch(cancellationToken: cancellation.Token);
      await call.RequestStream.WriteAsync(Msg("live"));
      Assert.That(await call.ResponseStream.MoveNext(CancellationToken.None),
                  Is.True);

      cancellation.Cancel();

      var failure = Assert.ThrowsAsync<RpcException>(async () => await call.ResponseStream.MoveNext(CancellationToken.None));
      Assert.That(failure!.StatusCode,
                  Is.EqualTo(StatusCode.Cancelled));
    }

    // The observation is indirect, and has to be: `tonic` turns a client RST_STREAM(CANCEL) on a
    // request stream into a clean end of stream on purpose, so no handler sees a cancellation as
    // such. `CancelWatch` counts request streams that end, and its client never half-closes, so an
    // end can only mean the reset arrived. Without the `h2::Reason::CANCEL` the abort error carries,
    // the reset would be INTERNAL_ERROR and the server would call it a failure instead.
    var deadline = DateTime.UtcNow.AddSeconds(5);
    int after;
    // The server reports "<count>:<last gRPC code seen on a request stream>".
    do
    {
      await Task.Delay(100);
      after = Observed(await Client.CancelsObservedAsync(Msg("")));
    } while (after == before && DateTime.UtcNow < deadline);

    TestContext.WriteLine($"cancellations observed by the server: {before} then {after} (last stream error {LastError})");
    Assert.That(after,
                Is.GreaterThan(before),
                "the server never observed the cancellation");
  }

  [Test]
  public async Task Test09_LargeMessagesBothWays()
  {
    // 16 MiB, far past the 64 KiB initial HTTP/2 window, so the request has to be admitted a window
    // at a time through a queue that holds one chunk and the response has to be reassembled from
    // however many chunks the connection chose.
    var payload = new string('x',
                             16 * 1024 * 1024);
    var reply = await Client.UnaryAsync(Msg(payload));

    Assert.That(reply.Payload.Length,
                Is.EqualTo(payload.Length));
    Assert.That(Text(reply),
                Is.EqualTo(payload));
  }

  [Test]
  public void Test10_BlockingCallUnderASingleThreadedSynchronizationContext()
  {
    // The Excel add-in shape: one UI thread, a SynchronizationContext that posts everything back to
    // it, and a generated client's blocking method doing sync-over-async on top. It only works if
    // no continuation of ours ever needs that thread - which is why every await in the handler is
    // `ConfigureAwait(false)` and the request pump starts on the thread pool.
    var loop = new SingleThreadedContext();
    var result = loop.Run(() => Text(Client.Unary(Msg("blocking"))));

    Assert.That(result,
                Is.EqualTo("blocking"));
  }

  [Test]
  public async Task Test11_PerCallOverhead()
  {
    // A measurement, not a gate.
    const int warmup = 20;
    const int rounds = 200;

    for (var index = 0; index < warmup; index++)
    {
      await Client.UnaryAsync(Msg("m"));
    }

    var stopwatch = Stopwatch.StartNew();
    for (var index = 0; index < rounds; index++)
    {
      await Client.UnaryAsync(Msg("m"));
    }

    stopwatch.Stop();
    var perCall = stopwatch.Elapsed.TotalMilliseconds / rounds;
    TestContext.WriteLine($"unary round-trip over the Rust handler: {perCall:F3} ms/call over {rounds} calls");
    Assert.That(perCall,
                Is.GreaterThan(0));
  }
}
