using System;
using System.Collections.Concurrent;
using System.Threading;

namespace ArmoniK.Api.Client.Native.Spike;

/// <summary>
///   One thread, and a <see cref="SynchronizationContext" /> that posts everything back to it.
/// </summary>
/// <remarks>
///   A stand-in for the shape that makes .NET Framework gRPC hard: an Excel add-in's UI thread,
///   where a blocking call is made and every captured continuation queues behind that same blocked
///   call. Anything in the handler that awaited without <c>ConfigureAwait(false)</c>, or that
///   started work on the calling thread, deadlocks here rather than merely being slow.
/// </remarks>
internal sealed class SingleThreadedContext : SynchronizationContext
{
  private readonly BlockingCollection<(SendOrPostCallback Callback, object? State)> queue_ = new();

  public override void Post(SendOrPostCallback callback,
                            object?            state)
    => queue_.Add((callback, state));

  public override void Send(SendOrPostCallback callback,
                            object?            state)
    => throw new NotSupportedException("a single-threaded context cannot answer a synchronous send");

  /// <summary>
  ///   Run <paramref name="body" /> on the loop thread, pumping posted callbacks until it returns.
  /// </summary>
  internal T Run<T>(Func<T> body)
  {
    var         previous = Current;
    T?          result   = default;
    Exception?  failure  = null;
    var         done     = new ManualResetEventSlim();

    var thread = new Thread(() =>
                            {
                              SetSynchronizationContext(this);
                              try
                              {
                                result = body();
                              }
                              catch (Exception exception)
                              {
                                failure = exception;
                              }
                              finally
                              {
                                done.Set();
                                queue_.CompleteAdding();
                              }
                            })
                 {
                   IsBackground = true,
                   Name         = "single-threaded-context",
                 };
    thread.Start();

    // The point of the fixture: this thread is busy running `body`, so anything posted to the
    // context is only pumped once `body` has already returned. A handler that needed the loop to
    // make progress would never get it.
    if (!done.Wait(TimeSpan.FromSeconds(30)))
    {
      throw new TimeoutException("the blocking call deadlocked under a single-threaded SynchronizationContext");
    }

    foreach (var (callback, state) in queue_.GetConsumingEnumerable())
    {
      callback(state);
    }

    SetSynchronizationContext(previous);
    thread.Join(TimeSpan.FromSeconds(5));

    if (failure != null)
    {
      throw failure;
    }

    return result!;
  }
}
