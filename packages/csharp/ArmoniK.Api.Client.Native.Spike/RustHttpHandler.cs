using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Threading;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   An <see cref="HttpMessageHandler" /> whose transport is the Rust ABI.
/// </summary>
/// <remarks>
///   <para>
///     This is the whole point of the design: what goes under
///     <c>GrpcChannelOptions.HttpHandler</c> is an ordinary message handler, so message framing,
///     <c>grpc-status</c>, deadlines, retry and size limits all stay with Grpc.Net.Client, which
///     maintains them. The slot it occupies is the one <c>WinHttpHandler</c> occupies today.
///   </para>
///   <para>
///     Duplex is not optional here. <see cref="SendAsync" /> returns as soon as the response headers
///     arrive and leaves the request body being pumped in the background; a handler that waited for
///     the request to finish first would make client-streaming and bidirectional calls deadlock.
///   </para>
/// </remarks>
public sealed class RustHttpHandler : HttpMessageHandler
{
  private readonly IntPtr client_;
  private          int    disposed_;

  /// <summary>Build a handler from a transport configuration document.</summary>
  /// <param name="configJson">
  ///   The flat options of <c>armonik_transport::HttpConfig</c>, e.g.
  ///   <c>{"Endpoint": "http://127.0.0.1:5000"}</c>.
  /// </param>
  public RustHttpHandler(string configJson)
  {
    var document = System.Text.Encoding.UTF8.GetBytes(configJson);
    var status = NativeMethods.ak_client_create(document,
                                                (UIntPtr)document.Length,
                                                out client_,
                                                out var error);
    var message = NativeMethods.TakeMessage(error);
    if (status != NativeMethods.Status.Ok)
    {
      throw new InvalidOperationException($"ak_client_create failed with {status}: {message}");
    }
  }

  protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,
                                                               CancellationToken  cancellationToken)
  {
    var call = new RustCall(client_,
                            BuildHeaders(request));

    // Whatever cancels the call - a gRPC deadline, a caller's token - arrives here.
    var registration = cancellationToken.Register(call.Cancel);

    try
    {
      // Deliberately not awaited. `Task.Run` rather than a bare call because the pump must not
      // inherit an ambient SynchronizationContext: under a single-threaded one, which is what an
      // Excel add-in's UI thread is, its continuations would queue behind the very call that is
      // waiting for them.
      var pump = Task.Run(() => PumpRequestAsync(request,
                                                 call));

      var headers = await call.Headers.ConfigureAwait(false);

      var response = BuildResponse(request,
                                   headers,
                                   call,
                                   registration,
                                   pump);
      return response;
    }
    catch
    {
      registration.Dispose();
      call.Dispose();
      throw;
    }
  }

  /// <summary>
  ///   Write the request body into the ABI, one armed write at a time, then end it.
  /// </summary>
  private static async Task PumpRequestAsync(HttpRequestMessage request,
                                             RustCall           call)
  {
    if (request.Content == null)
    {
      call.CloseSend();
      return;
    }

    using var stream = new RequestStream(call);
    try
    {
      await request.Content.CopyToAsync(stream)
                   .ConfigureAwait(false);
      call.CloseSend();
    }
    catch (Exception)
    {
      // The request is over one way or another; its COMPLETED carries the reason, and that is what
      // the reader will see. Nothing useful can be added from here.
      call.Cancel();
    }
  }

  private HttpResponseMessage BuildResponse(HttpRequestMessage                 request,
                                            List<KeyValuePair<string, string>> headers,
                                            RustCall                           call,
                                            CancellationTokenRegistration      registration,
                                            Task                               pump)
  {
    var response = new HttpResponseMessage
                   {
                     // `HttpVersion.Version20` does not exist on .NET Framework.
                     Version        = new Version(2,
                                                  0),
                     RequestMessage = request,
                   };

    var content = new StreamContent(new ResponseStream(call,
                                                       request,
                                                       registration,
                                                       pump));

    foreach (var header in headers)
    {
      if (header.Key == ":status")
      {
        response.StatusCode = (HttpStatusCode)int.Parse(header.Value);
        continue;
      }

      // Content headers belong on the content, and `content-type` in particular: Grpc.Net.Client
      // reads it from there to decide the response is gRPC at all.
      if (!response.Headers.TryAddWithoutValidation(header.Key,
                                                    header.Value))
      {
        content.Headers.TryAddWithoutValidation(header.Key,
                                                header.Value);
      }
    }

    response.Content = content;
    return response;
  }

  /// <summary>
  ///   The request headers, as the ABI wants them: two pseudo-keys, then everything else in order.
  /// </summary>
  private static List<KeyValuePair<string, string>> BuildHeaders(HttpRequestMessage request)
  {
    var headers = new List<KeyValuePair<string, string>>
                  {
                    new(":method",
                        request.Method.Method),
                    // Absolute, because the connection pool keys on the authority.
                    new(":url",
                        request.RequestUri!.AbsoluteUri),
                  };

    foreach (var header in request.Headers)
    {
      foreach (var value in header.Value)
      {
        headers.Add(new KeyValuePair<string, string>(header.Key,
                                                     value));
      }
    }

    if (request.Content != null)
    {
      foreach (var header in request.Content.Headers)
      {
        foreach (var value in header.Value)
        {
          headers.Add(new KeyValuePair<string, string>(header.Key,
                                                       value));
        }
      }
    }

    return headers;
  }

  protected override void Dispose(bool disposing)
  {
    if (Interlocked.Exchange(ref disposed_,
                             1) == 0)
    {
      // Requests still in flight hold their own reference to the pool and finish normally.
      NativeMethods.ak_client_free(client_);
    }

    base.Dispose(disposing);
  }

  /// <summary>The write half: one armed write per <c>WriteAsync</c>.</summary>
  private sealed class RequestStream : Stream
  {
    private readonly RustCall call_;

    internal RequestStream(RustCall call)
      => call_ = call;

    public override bool CanRead
      => false;

    public override bool CanSeek
      => false;

    public override bool CanWrite
      => true;

    public override long Length
      => throw new NotSupportedException();

    public override long Position
    {
      get => throw new NotSupportedException();
      set => throw new NotSupportedException();
    }

    public override Task WriteAsync(byte[]            buffer,
                                    int               offset,
                                    int               count,
                                    CancellationToken cancellationToken)
      => call_.WriteAsync(buffer,
                          offset,
                          count);

    public override void Write(byte[] buffer,
                               int    offset,
                               int    count)
      => WriteAsync(buffer,
                    offset,
                    count,
                    CancellationToken.None)
        .GetAwaiter()
        .GetResult();

    // Every write is already flushed as far as this side is concerned: WRITE_DONE means the chunk
    // was accepted by the connection.
    public override void Flush()
    {
    }

    public override Task FlushAsync(CancellationToken cancellationToken)
      => Task.CompletedTask;

    public override int Read(byte[] buffer,
                             int    offset,
                             int    count)
      => throw new NotSupportedException();

    public override long Seek(long       offset,
                              SeekOrigin origin)
      => throw new NotSupportedException();

    public override void SetLength(long value)
      => throw new NotSupportedException();
  }

  /// <summary>The read half: one armed read per <c>ReadAsync</c>, re-cut to the caller's buffer.</summary>
  private sealed class ResponseStream : Stream
  {
    private readonly RustCall                      call_;
    private readonly HttpRequestMessage            request_;
    private readonly CancellationTokenRegistration registration_;
    private readonly Task                          pump_;

    private byte[] leftover_ = Array.Empty<byte>();
    private int    consumed_;
    private bool   ended_;

    internal ResponseStream(RustCall                      call,
                            HttpRequestMessage            request,
                            CancellationTokenRegistration registration,
                            Task                          pump)
    {
      call_         = call;
      request_      = request;
      registration_ = registration;
      pump_         = pump;
    }

    public override bool CanRead
      => true;

    public override bool CanSeek
      => false;

    public override bool CanWrite
      => false;

    public override long Length
      => throw new NotSupportedException();

    public override long Position
    {
      get => throw new NotSupportedException();
      set => throw new NotSupportedException();
    }

    public override async Task<int> ReadAsync(byte[]            buffer,
                                              int               offset,
                                              int               count,
                                              CancellationToken cancellationToken)
    {
      // A chunk is a DATA frame's worth of body, not a message, and the reader asks for whatever
      // size suits it. Whatever is left over is kept for the next call rather than dropped.
      if (consumed_ < leftover_.Length)
      {
        return Take(buffer,
                    offset,
                    count);
      }

      if (ended_)
      {
        return 0;
      }

      var chunk = await call_.ReadAsync()
                             .ConfigureAwait(false);
      if (chunk == null || chunk.Length == 0)
      {
        await EndAsync()
         .ConfigureAwait(false);
        return 0;
      }

      leftover_ = chunk;
      consumed_ = 0;
      return Take(buffer,
                  offset,
                  count);
    }

    private int Take(byte[] buffer,
                     int    offset,
                     int    count)
    {
      var taken = Math.Min(count,
                           leftover_.Length - consumed_);
      Buffer.BlockCopy(leftover_,
                       consumed_,
                       buffer,
                       offset,
                       taken);
      consumed_ += taken;
      return taken;
    }

    /// <summary>
    ///   Publish the trailers, then report the end of the response.
    /// </summary>
    /// <remarks>
    ///   The convention .NET Framework has for trailers, and the one Grpc.Net.Client reads there:
    ///   an <see cref="System.Net.Http.Headers.HttpHeaders" /> under
    ///   <c>RequestMessage.Properties["__ResponseTrailers"]</c>. It has to be in place before this
    ///   returns zero, because zero is what sends the reader on to look for it.
    /// </remarks>
    private async Task EndAsync()
    {
      ended_ = true;
      var completion = await call_.Completed.ConfigureAwait(false);

      // `HttpHeaders` cannot be constructed from outside its assembly, so the instance comes from a
      // message that exists only to own it.
      var trailers = new HttpResponseMessage().Headers;
      foreach (var trailer in completion.Trailers)
      {
        trailers.TryAddWithoutValidation(trailer.Key,
                                         trailer.Value);
      }

      request_.Properties["__ResponseTrailers"] = trailers;
    }

    public override int Read(byte[] buffer,
                             int    offset,
                             int    count)
      => ReadAsync(buffer,
                   offset,
                   count,
                   CancellationToken.None)
        .GetAwaiter()
        .GetResult();

    public override void Flush()
    {
    }

    public override long Seek(long       offset,
                              SeekOrigin origin)
      => throw new NotSupportedException();

    public override void SetLength(long value)
      => throw new NotSupportedException();

    public override void Write(byte[] buffer,
                               int    offset,
                               int    count)
      => throw new NotSupportedException();

    protected override void Dispose(bool disposing)
    {
      if (disposing)
      {
        registration_.Dispose();
        call_.Dispose();
        // The pump is cancelled by the `Dispose` above rather than awaited: this runs on whatever
        // thread disposed the response, and blocking it is not this handler's business. Reading
        // `Exception` marks a fault observed *if the pump has already finished*; one still running
        // is left to the continuation that `PumpRequestAsync` already has around its own body,
        // which swallows the failure there.
        _ = pump_.Exception;
      }

      base.Dispose(disposing);
    }
  }
}
