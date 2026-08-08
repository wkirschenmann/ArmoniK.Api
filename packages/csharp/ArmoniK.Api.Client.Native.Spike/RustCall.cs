using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   One request in flight, and the demultiplexer for the events it produces.
/// </summary>
/// <remarks>
///   <para>
///     The reactor's contract is what makes this small. Delivery is serialised per request, so
///     nothing here needs a lock; at most one read and one write are armed at a time, so one
///     <see cref="TaskCompletionSource{TResult}" /> each is enough; and COMPLETED is terminal, so
///     that is the single place everything outstanding is resolved.
///   </para>
///   <para>
///     Every source is built with <see cref="TaskCreationOptions.RunContinuationsAsynchronously" />.
///     Without it the continuation - which is, transitively, the caller's own code - would run on a
///     tokio thread inside the callback, and the callback is not allowed to block.
///   </para>
/// </remarks>
internal sealed class RustCall : IDisposable
{
  /// <summary>
  ///   The one callback instance, rooted for the life of the process.
  /// </summary>
  /// <remarks>
  ///   A delegate marshalled to a function pointer is not kept alive by the native side holding
  ///   that pointer. A static field is the simplest thing that is certainly rooted; the per-request
  ///   state travels in <c>ctx</c> instead.
  /// </remarks>
  private static readonly NativeMethods.OnEvent EventCallback = OnEvent;

  private readonly TaskCompletionSource<List<KeyValuePair<string, string>>> headers_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly TaskCompletionSource<Completion> completed_ =
    new(TaskCreationOptions.RunContinuationsAsynchronously);

  private readonly GCHandle self_;
  private readonly IntPtr   request_;

  private TaskCompletionSource<bool>?   write_;
  private TaskCompletionSource<byte[]?>? read_;
  private int                           disposed_;
  private int                           contextReleased_;

  /// <summary>How a request ended.</summary>
  internal readonly struct Completion
  {
    internal Completion(int                                  code,
                        List<KeyValuePair<string, string>>? trailers,
                        string                               message)
    {
      Code     = code;
      Trailers = trailers ?? new List<KeyValuePair<string, string>>();
      Message  = message;
    }

    internal int                                Code     { get; }
    internal List<KeyValuePair<string, string>> Trailers { get; }
    internal string                             Message  { get; }
  }

  /// <summary>Start a request on <paramref name="client" />, throwing if the ABI refuses it.</summary>
  internal RustCall(IntPtr                                       client,
                    IReadOnlyList<KeyValuePair<string, string>> requestHeaders)
  {
    self_ = GCHandle.Alloc(this);
    var blob = Blob.Encode(requestHeaders);

    var status = NativeMethods.ak_request_start(client,
                                                blob,
                                                (UIntPtr)blob.Length,
                                                EventCallback,
                                                GCHandle.ToIntPtr(self_),
                                                out request_,
                                                out var error);
    if (status != NativeMethods.Status.Ok)
    {
      // A start that failed produces no event ever, so the context comes back now rather than on a
      // COMPLETED that is never coming. Through the same method, so there is one place that frees.
      ReleaseContext();
      throw new IOException($"ak_request_start failed with {status}: {NativeMethods.TakeMessage(error)}");
    }

    NativeMethods.TakeMessage(error);
  }

  /// <summary>The response headers, once they arrive.</summary>
  internal Task<List<KeyValuePair<string, string>>> Headers
    => headers_.Task;

  /// <summary>The terminal event.</summary>
  internal Task<Completion> Completed
    => completed_.Task;

  /// <summary>Arm one write and wait for it to be accepted by the connection.</summary>
  internal Task WriteAsync(byte[] buffer,
                           int    offset,
                           int    count)
  {
    var chunk = new byte[count];
    Buffer.BlockCopy(buffer,
                     offset,
                     chunk,
                     0,
                     count);

    var pending = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
    // Published before arming: the event may be delivered before `ak_request_write` has returned.
    write_ = pending;

    var status = NativeMethods.ak_request_write(request_,
                                                chunk,
                                                (UIntPtr)chunk.Length);
    if (status != NativeMethods.Status.Ok)
    {
      write_ = null;
      // A request that has already finished is not a failure to report here: whatever ended it is
      // already on its way through `Completed`.
      pending.TrySetException(new IOException($"ak_request_write failed with {status}"));
    }

    return pending.Task;
  }

  /// <summary>End the request body.</summary>
  internal void CloseSend()
    => NativeMethods.ak_request_close_send(request_);

  /// <summary>Arm one read. Answers with the chunk, or null at the end of the response.</summary>
  internal Task<byte[]?> ReadAsync()
  {
    var pending = new TaskCompletionSource<byte[]?>(TaskCreationOptions.RunContinuationsAsynchronously);
    read_ = pending;

    var status = NativeMethods.ak_request_read(request_);
    if (status != NativeMethods.Status.Ok)
    {
      read_ = null;
      pending.TrySetException(new IOException($"ak_request_read failed with {status}"));
    }

    return pending.Task;
  }

  /// <summary>Cancel the request. Harmless once it has completed.</summary>
  internal void Cancel()
  {
    if (Volatile.Read(ref disposed_) == 0)
    {
      NativeMethods.ak_request_cancel(request_);
    }
  }

  private static void OnEvent(IntPtr             ctx,
                              int                kind,
                              NativeMethods.AkBytesIn payload,
                              int                code)
  {
    // Nothing may unwind into Rust across the ABI. Everything this does is also non-blocking: the
    // sources hand their continuations to the thread pool.
    try
    {
      var call = GCHandle.FromIntPtr(ctx)
                         .Target as RustCall;
      call?.Deliver(kind,
                    Copy(payload),
                    code);
    }
    catch
    {
      // A failure here would strand the caller waiting for an event that already happened, but
      // there is nowhere to report it to and raising is not allowed.
    }
  }

  private static byte[] Copy(NativeMethods.AkBytesIn payload)
  {
    var length = (int)payload.Len;
    if (length == 0 || payload.Ptr == IntPtr.Zero)
    {
      return Array.Empty<byte>();
    }

    // The payload is borrowed for the duration of this call and nothing else. Copying is the whole
    // of what the caller has to do, and it has to happen before returning.
    var buffer = new byte[length];
    Marshal.Copy(payload.Ptr,
                 buffer,
                 0,
                 length);
    return buffer;
  }

  private void Deliver(int    kind,
                       byte[] payload,
                       int    code)
  {
    switch (kind)
    {
      case NativeMethods.Event.ResponseHeaders:
        headers_.TrySetResult(Blob.Decode(payload));
        break;

      case NativeMethods.Event.WriteDone:
        Interlocked.Exchange(ref write_,
                             null)
                  ?.TrySetResult(true);
        break;

      case NativeMethods.Event.ReadDone:
        Interlocked.Exchange(ref read_,
                             null)
                  ?.TrySetResult(payload);
        break;

      case NativeMethods.Event.Completed:
        Complete(payload,
                 code);
        // Last, and after everything Complete does: the object is only rooted by the handle this
        // gives back, so nothing may touch `this` afterwards.
        ReleaseContext();
        break;
    }
  }

  private void Complete(byte[] payload,
                        int    code)
  {
    var succeeded = code == NativeMethods.Status.Ok;
    var completion = succeeded
                       ? new Completion(code,
                                        Blob.Decode(payload),
                                        string.Empty)
                       : new Completion(code,
                                        null,
                                        System.Text.Encoding.UTF8.GetString(payload));

    // Published before anything waiting is released, so a reader that sees the end of the response
    // can already see the trailers.
    completed_.TrySetResult(completion);

    if (succeeded)
    {
      // The response ended before any header arrived. That is not a transport failure but it is not
      // a response either, and whoever is waiting has to be told something.
      headers_.TrySetException(new IOException("the request completed without a response"));
      Interlocked.Exchange(ref read_,
                           null)
                ?.TrySetResult(null);
      Interlocked.Exchange(ref write_,
                           null)
                ?.TrySetResult(true);
      return;
    }

    // Rule 1 in its general form: an operation still armed is resolved by COMPLETED.
    var failure = Failure(code,
                          completion.Message);
    headers_.TrySetException(failure);
    Interlocked.Exchange(ref read_,
                         null)
              ?.TrySetException(failure);
    Interlocked.Exchange(ref write_,
                         null)
              ?.TrySetException(failure);
  }

  private static Exception Failure(int    code,
                                   string message)
    => code == NativeMethods.Status.Cancelled
         ? new OperationCanceledException(message)
         : new IOException($"the request failed ({code}): {message}");

  /// <summary>
  ///   Give the context back, once, at the only moment the ABI allows.
  /// </summary>
  /// <remarks>
  ///   <para>
  ///     The GCHandle rooting this object belongs to the request's driving task from the moment
  ///     <c>ak_request_start</c> succeeds, and <c>COMPLETED</c> is the task handing it back.
  ///     Releasing the handle does not end that: the terminal event still arrives, and until it does
  ///     the native side still holds a pointer to this object. Freeing at <see cref="Dispose" />
  ///     instead would be the classic use-after-free.
  ///   </para>
  ///   <para>
  ///     A flag rather than a counter, deliberately. There is exactly one native reference to this
  ///     object - the task's - so the count can only ever be zero or one, and a decrement would
  ///     advertise a generality the code does not have, sending the next reader looking for the
  ///     other increment. It becomes a counter the day a second party can hold <c>ctx</c>, which is
  ///     what a shutdown callback or the log bridge would do; the change is local to this method.
  ///     Managed references to this object - the response stream, the request pump - are the
  ///     garbage collector's business and need no handle of their own.
  ///   </para>
  ///   <para>
  ///     Nothing reaches this twice today: a failed start and a <c>COMPLETED</c> are mutually
  ///     exclusive, and the ABI delivers <c>COMPLETED</c> once. The guard is there because the cost
  ///     of being wrong is asymmetric - <see cref="GCHandle.Free" /> on an already-freed handle
  ///     throws, and it would throw inside a callback running on a foreign thread, which is the one
  ///     place an exception must not escape. A contract violation on the other side degrades to
  ///     nothing happening rather than to that.
  ///   </para>
  /// </remarks>
  private void ReleaseContext()
  {
    if (Interlocked.Exchange(ref contextReleased_,
                             1) == 0)
    {
      self_.Free();
    }
  }

  public void Dispose()
  {
    if (Interlocked.Exchange(ref disposed_,
                             1) != 0)
    {
      return;
    }

    // A reference given up, not an object destroyed: the request is cancelled and runs to its
    // COMPLETED event, which is what releases the context. Nothing here frees the GCHandle.
    NativeMethods.ak_request_release(request_);

    // Nothing else will ever complete these now.
    var abandoned = new ObjectDisposedException(nameof(RustCall));
    headers_.TrySetException(abandoned);
    completed_.TrySetException(abandoned);
    Interlocked.Exchange(ref read_,
                         null)
              ?.TrySetException(abandoned);
    Interlocked.Exchange(ref write_,
                         null)
              ?.TrySetException(abandoned);

    // Read so an exception nobody is waiting for does not resurface later as an unobserved one.
    _ = headers_.Task.Exception;
    _ = completed_.Task.Exception;
  }
}
